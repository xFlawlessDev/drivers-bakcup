//! Discovery and normalization helpers for the `pack` command.
//!
//! This module walks a vendor driver pack, works out each package's category and
//! name, and collects the files that belong to each package. It contains no
//! filesystem mutation; [`crate::pack`] uses these results to write output.

use crate::inf_parser::parse_inf_file;
use crate::models::PackDriver;
use crate::util::{sanitize_folder_name, sha256_file};
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Walk the source tree and collect one [`PackDriver`] per INF file.
pub fn discover_drivers(source: &Path, verbose: bool) -> Result<Vec<PackDriver>> {
    let inf_files = find_inf_files(source)?;
    let mut drivers = Vec::new();

    for inf_path in inf_files {
        let parsed = match parse_inf_file(&inf_path) {
            Ok(p) => p,
            Err(e) => {
                if verbose {
                    eprintln!("Warning: failed to parse {}: {}", inf_path.display(), e);
                }
                continue;
            }
        };

        let source_dir = match inf_path.parent() {
            Some(dir) => dir.to_path_buf(),
            None => continue,
        };

        let inf_name = inf_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.inf")
            .to_string();

        let inf_hash = match sha256_file(&inf_path) {
            Ok(hash) => hash,
            Err(e) => {
                if verbose {
                    eprintln!("Warning: cannot hash {}: {}", inf_path.display(), e);
                }
                continue;
            }
        };

        let files = collect_package_files(&source_dir, &inf_name);
        let total_bytes = files
            .iter()
            .filter_map(|f| fs::metadata(source_dir.join(f)).ok())
            .map(|m| m.len())
            .sum();

        let device_class = parsed.raw_version_info.class.clone();
        let category = resolve_category(source, &source_dir, device_class.as_deref());

        // The `[Version]` provider is often a `%StringKey%` reference; resolve it
        // so folder names and the manifest show a real provider name.
        let provider = resolve_provider(&parsed);

        // Normalize the catalog file name so INFs that share a catalog (and are
        // therefore one installer bundle) can be grouped reliably.
        let catalog_file = parsed
            .raw_version_info
            .catalog_file
            .as_deref()
            .map(|c| c.trim().to_lowercase())
            .filter(|c| !c.is_empty());

        // Unique, sorted friendly device names for the manifest.
        let mut device_names: Vec<String> = parsed
            .drivers
            .iter()
            .filter_map(|d| d.device_name.clone())
            .filter(|n| !n.trim().is_empty())
            .collect();
        device_names.sort();
        device_names.dedup();

        // Unique, sorted hardware IDs for the manifest.
        let mut hardware_ids: Vec<String> = parsed
            .drivers
            .iter()
            .filter_map(|d| d.hardware_id.clone())
            .filter(|h| !h.trim().is_empty())
            .collect();
        hardware_ids.sort();
        hardware_ids.dedup();

        drivers.push(PackDriver {
            category: Some(category),
            source_dir,
            inf_name,
            device_class,
            driver_version: parsed.raw_version_info.driver_version.clone(),
            driver_date: parsed.raw_version_info.driver_date.clone(),
            provider,
            catalog_file,
            device_count: parsed.drivers.len(),
            device_names,
            hardware_ids,
            inf_hash,
            files,
            total_bytes,
        });
    }

    Ok(drivers)
}

/// Find every INF file under `dir`, recursively, sorted.
fn find_inf_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        let entries = match fs::read_dir(&current) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .map(|e| e.eq_ignore_ascii_case("inf"))
                .unwrap_or(false)
            {
                out.push(path);
            }
        }
    }

    out.sort();
    Ok(out)
}

/// Resolve the provider name from an INF.
///
/// The parser resolves `[Version] Provider` references against `[Strings]`, so
/// this normally just returns that value. A `%Key%` fallback is kept for INFs
/// whose string table is missing.
fn resolve_provider(parsed: &crate::models::ParsedInfFile) -> Option<String> {
    let raw = parsed.raw_version_info.provider.as_deref()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    if raw.starts_with('%') && raw.ends_with('%') && raw.len() > 2 {
        // Unresolved reference: fall back to a device's provider, then the
        // manufacturer, then a humanised key name.
        if let Some(resolved) = parsed
            .drivers
            .iter()
            .filter_map(|d| d.driver_provider_name.as_deref())
            .find(|p| !p.trim().is_empty() && !(p.starts_with('%') && p.ends_with('%')))
        {
            return Some(resolved.to_string());
        }
        if let Some(mfg) = parsed
            .drivers
            .iter()
            .filter_map(|d| d.manufacturer.as_deref())
            .find(|m| !m.trim().is_empty())
        {
            return Some(mfg.to_string());
        }
        let key = &raw[1..raw.len() - 1];
        return Some(clean_provider_key(key));
    }

    Some(raw.to_string())
}

/// Turn a provider string key such as `PROVIDER_NAME` into `Provider_Name`.
fn clean_provider_key(key: &str) -> String {
    let words: Vec<String> = key
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect();
    if words.is_empty() {
        key.to_string()
    } else {
        words.join(" ")
    }
}

/// Resolve the category folder for a driver.
///
/// When the source is already organized into category folders (e.g. `01_Chipset`)
/// the top-level folder is used. When the source is flat (one folder per OEM INF,
/// as some ODMs ship), fall back to the device class so the normalized pack stays
/// groupable.
pub fn resolve_category(source: &Path, source_dir: &Path, device_class: Option<&str>) -> String {
    // Is the driver nested at least two levels below the source? If so, the
    // first component is a real category folder.
    if let Ok(rel) = source_dir.strip_prefix(source) {
        let mut components = rel.components();
        if let (Some(_first), Some(_second)) = (components.next(), components.next()) {
            if let Some(first) = rel.components().next() {
                return first.as_os_str().to_string_lossy().to_string();
            }
        }
    }

    // Flat layout: group by device class instead of the per-driver folder name.
    device_class
        .map(|c| canonical_class_name(c.trim()))
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "Uncategorized".to_string())
}

/// Normalize a device class to its conventional Windows spelling.
///
/// INF files are inconsistent (`net`, `Net`, `MEDIA`); the same class should map
/// to one folder. Unknown classes are returned with their original casing.
pub fn canonical_class_name(class: &str) -> String {
    const KNOWN: [&str; 20] = [
        "Display",
        "Net",
        "MEDIA",
        "System",
        "Extension",
        "SoftwareComponent",
        "SoftwareDevice",
        "HIDClass",
        "USB",
        "Bluetooth",
        "Printer",
        "Monitor",
        "Image",
        "AudioProcessingObject",
        "SDHost",
        "SCSIAdapter",
        "NetService",
        "USBFunctionController",
        "Firmware",
        "SecurityDevices",
    ];

    let lower = class.to_lowercase();
    for known in KNOWN {
        if known.to_lowercase() == lower {
            return known.to_string();
        }
    }
    class.to_string()
}

/// Find top-level category folders that contain files but no INF anywhere.
///
/// Vendor packs often ship EXE/INI-only installers (e.g. audio effects) that are
/// still part of the pack. These must not be dropped from a normalized pack.
pub fn discover_installer_only_categories(source: &Path, drivers: &[PackDriver]) -> Vec<PathBuf> {
    // Top-level folders that actually contain a driver (by source path).
    let mut top_dirs_with_inf: std::collections::HashSet<PathBuf> =
        std::collections::HashSet::new();
    for driver in drivers {
        if let Ok(rel) = driver.source_dir.strip_prefix(source) {
            if let Some(first) = rel.components().next() {
                top_dirs_with_inf.insert(source.join(first.as_os_str()));
            }
        }
    }

    let mut installer_only = Vec::new();
    let entries = match fs::read_dir(source) {
        Ok(e) => e,
        Err(_) => return installer_only,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || top_dirs_with_inf.contains(&path) {
            continue;
        }
        if has_any_file(&path) {
            installer_only.push(path);
        }
    }

    installer_only.sort();
    installer_only
}

/// True when a directory tree contains at least one file.
fn has_any_file(dir: &Path) -> bool {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = match fs::read_dir(&current) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                return true;
            }
        }
    }
    false
}

/// Count files and total bytes in a directory tree.
pub fn count_tree(dir: &Path) -> (usize, u64) {
    let mut files = 0;
    let mut bytes = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = match fs::read_dir(&current) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files += 1;
                if let Ok(meta) = path.metadata() {
                    bytes += meta.len();
                }
            }
        }
    }
    (files, bytes)
}

/// Collect the files that belong to a driver INF.
///
/// Only the INF and the files it actually references (its catalog plus files
/// named in the INF) are attributed to a single INF. Shared payload in the same
/// folder (common for multi-INF bundles such as Intel graphics) is attributed
/// to the bundle instead, so it is not duplicated per INF.
fn collect_package_files(source_dir: &Path, inf_name: &str) -> Vec<PathBuf> {
    let mut files = vec![PathBuf::from(inf_name)];

    let inf_path = source_dir.join(inf_name);
    let content = match read_text_lossy(&inf_path) {
        Some(c) => c.to_lowercase(),
        None => {
            files.sort();
            files.dedup();
            return files;
        }
    };

    if let Ok(entries) = fs::read_dir(source_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.eq_ignore_ascii_case(inf_name) {
                continue;
            }
            // Attribute a file to this INF only when the INF names it.
            if content.contains(&name.to_lowercase()) {
                files.push(PathBuf::from(name));
            }
        }
    }

    files.sort();
    files.dedup();
    files
}

/// Read a file as text, handling UTF-16 BOMs; `None` on failure.
fn read_text_lossy(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let units: Vec<u16> = bytes[2..]
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return Some(String::from_utf16_lossy(&units));
    }
    Some(String::from_utf8_lossy(&bytes).to_string())
}

/// Collect every file directly inside `source_dir` (the shared bundle payload).
///
/// Used when several INFs share one folder: the non-INF files are attributed to
/// the bundle once rather than to each INF.
pub fn collect_bundle_extra_files(source_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(source_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Include sub-trees that are not themselves driver packages.
                if !contains_inf(&path) {
                    collect_tree_files(source_dir, &path, &mut files);
                }
            } else {
                files.push(PathBuf::from(entry.file_name()));
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

/// Recursively collect file paths relative to `base`.
fn collect_tree_files(base: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_tree_files(base, &path, out);
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.push(rel.to_path_buf());
        }
    }
}

/// True when a directory tree contains any `.inf` file.
fn contains_inf(dir: &Path) -> bool {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = match fs::read_dir(&current) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .map(|e| e.eq_ignore_ascii_case("inf"))
                .unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}

/// Compute the normalized folder name for an installer bundle.
///
/// Format: `<Class>_<Provider>_<Version>_<Bundle>`, where `<Bundle>` is the
/// catalog stem when the bundle shares one (e.g. `igdlh`), otherwise the primary
/// INF stem. This keeps single-INF packages specific while giving multi-INF
/// bundles (Intel graphics) one shared, recognizable name.
pub fn normalized_folder_name(driver: &PackDriver, bundle_label: &str) -> String {
    let class = driver
        .device_class
        .as_deref()
        .map(|c| {
            if is_placeholder_class(c) {
                "Unknown".to_string()
            } else {
                canonical_class_name(c.trim())
            }
        })
        .unwrap_or_else(|| "Unknown".to_string());

    let provider = driver
        .provider
        .as_deref()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "Unknown".to_string());

    let version = clean_version(driver.driver_version.as_deref());

    sanitize_folder_name(&format!(
        "{}_{}_{}_{}",
        class, provider, version, bundle_label
    ))
}

/// The bundle label used in folder names.
///
/// For a merged bundle (same driver in several folders) the INF stem is the
/// clearest label. For a folder bundle (several INFs sharing payload) the source
/// folder name is used, since the INFs differ. Falls back to the catalog stem and
/// then the primary INF stem.
pub fn bundle_label(driver: &PackDriver, inf_names: &[String], source_count: usize) -> String {
    // Merged across folders: label by the driver's INF.
    if source_count > 1 {
        return file_stem(&driver.inf_name);
    }

    // Single folder with several INFs: label by the folder.
    if inf_names.len() > 1 {
        if let Some(name) = driver.source_dir.file_name() {
            let name = name.to_string_lossy().to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }

    // Single INF: use the INF stem. It is the most specific label and avoids
    // collisions when several packages share one catalog (Intel graphics ships
    // `cui_dch`, `igcc_dch` and `iigd_dch` under the same `igdlh.cat`).
    file_stem(&driver.inf_name)
}

/// Strip the final extension from a file name (`igdlh.cat` -> `igdlh`).
fn file_stem(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => name.to_string(),
    }
}

/// Reduce an INF driver version to a compact token suitable for a folder name.
///
/// Some INFs contain `stampinf` placeholders such as
/// `1.0.0.18; TODO: set DriverVer in stampinf property pages`; only the leading
/// version-like token is kept.
pub fn clean_version(version: Option<&str>) -> String {
    let raw = version.unwrap_or("").trim();
    if raw.is_empty() {
        return "NoVersion".to_string();
    }

    let token = raw
        .split(|c: char| c == ';' || c.is_whitespace())
        .next()
        .unwrap_or("")
        .trim();

    if token.is_empty() {
        "NoVersion".to_string()
    } else {
        token.to_string()
    }
}

/// Normalize an INF `DriverVer` date to ISO `YYYY-MM-DD` when possible.
///
/// INF dates are usually `MM/DD/YYYY`. Returns `Unknown` when absent and the
/// original value when it cannot be parsed.
pub fn clean_driver_date(date: Option<&str>) -> String {
    let raw = date.unwrap_or("").trim();
    if raw.is_empty() {
        return "Unknown".to_string();
    }

    // Take the date portion before any `;` comment or trailing time.
    let token = raw
        .split(';')
        .next()
        .unwrap_or("")
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim();

    if let Some((y, m, d)) = parse_inf_date(token) {
        return format!("{:04}-{:02}-{:02}", y, m, d);
    }

    token.to_string()
}

/// Parse `MM/DD/YYYY` or `YYYY/MM/DD` into `(year, month, day)`.
fn parse_inf_date(token: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = token.split('/').collect();
    if parts.len() != 3 {
        return None;
    }
    let a: u32 = parts[0].parse().ok()?;
    let b: u32 = parts[1].parse().ok()?;
    let c: u32 = parts[2].parse().ok()?;

    // A 4-digit first component means YYYY/MM/DD.
    let (year, month, day) = if parts[0].len() == 4 {
        (a, b, c)
    } else {
        (c, a, b)
    };

    if (1..=12).contains(&month) && (1..=31).contains(&day) && year >= 1900 {
        Some((year, month, day))
    } else {
        None
    }
}

/// True for INF classes that vendors leave as unresolved placeholders.
pub fn is_placeholder_class(class: &str) -> bool {
    let lower = class.to_lowercase();
    lower.contains("todo") || lower.contains("unknown") || lower.trim().is_empty()
}

/// Normalize a device class for display in the manifest: canonical casing and
/// placeholder classes (e.g. `System ; TODO: edit Class`) reduced to `Unknown`.
pub fn manifest_class_name(class: Option<&str>) -> String {
    match class {
        Some(c) if !is_placeholder_class(c) => canonical_class_name(c.trim()),
        _ => "Unknown".to_string(),
    }
}

/// Join manifest list values in full.
///
/// Nothing is truncated: these columns feed downstream metadata (driver upload),
/// where the complete set of device names and hardware IDs is required. Empty
/// and duplicate entries are removed and the result is stable-ordered.
pub fn format_list(items: &[String]) -> String {
    let mut unique: Vec<&str> = items
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    unique.sort_unstable();
    unique.dedup();
    unique.join("; ")
}

/// Format a package's device names for the manifest (complete).
pub fn format_device_names(names: &[String]) -> String {
    format_list(names)
}

/// Format a package's hardware IDs for the manifest (complete).
pub fn format_hardware_ids(ids: &[String]) -> String {
    format_list(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PackDriver;
    use std::path::PathBuf;

    fn driver(
        category: &str,
        dir: &Path,
        inf: &str,
        class: &str,
        version: &str,
        hash: &str,
    ) -> PackDriver {
        PackDriver {
            source_dir: dir.to_path_buf(),
            inf_name: inf.to_string(),
            device_class: Some(class.to_string()),
            driver_version: Some(version.to_string()),
            driver_date: Some("01/15/2025".to_string()),
            provider: Some("Intel".to_string()),
            catalog_file: None,
            device_count: 1,
            device_names: vec!["Intel(R) UHD Graphics".to_string()],
            hardware_ids: vec!["PCI\\VEN_8086&DEV_4680".to_string()],
            category: Some(category.to_string()),
            inf_hash: hash.to_string(),
            files: vec![PathBuf::from(inf)],
            total_bytes: 10,
        }
    }

    #[test]
    fn folder_name_uses_class_provider_version_inf() {
        let d = driver(
            "03_VGA",
            Path::new("src/03_VGA/Nvidia"),
            "nvcvegpu.inf",
            "Display",
            "32.0.15.7705",
            "h1",
        );
        let label = bundle_label(&d, &["nvcvegpu.inf".to_string()], 1);
        assert_eq!(
            normalized_folder_name(&d, &label),
            "Display_Intel_32.0.15.7705_nvcvegpu"
        );
    }

    #[test]
    fn bundle_label_for_folder_bundle_uses_folder_name() {
        // Several INFs share a folder: label by the folder.
        let mut d = driver(
            "Display",
            Path::new("src/03_Graphics/Drivers"),
            "iigd_dch.inf",
            "Display",
            "31.0",
            "h1",
        );
        d.catalog_file = Some("igdlh.cat".to_string());
        let infs = vec!["iigd_dch.inf".to_string(), "iigd_ext.inf".to_string()];
        assert_eq!(bundle_label(&d, &infs, 1), "Drivers");
    }

    #[test]
    fn bundle_label_for_merged_bundle_uses_inf_stem() {
        // Same INF name across several folders: label by the INF.
        let d = driver(
            "Bluetooth",
            Path::new("src/CCP"),
            "ibtusb.inf",
            "Bluetooth",
            "23.60.0.1",
            "h1",
        );
        assert_eq!(bundle_label(&d, &["ibtusb.inf".to_string()], 8), "ibtusb");
    }

    #[test]
    fn folder_name_is_unique_per_source_folder() {
        let a = driver(
            "Display",
            Path::new("src/a"),
            "iigd_dch.inf",
            "Display",
            "32.0",
            "h1",
        );
        let b = driver(
            "Display",
            Path::new("src/b"),
            "iigd_dch_d.inf",
            "Display",
            "32.0",
            "h2",
        );
        let la = bundle_label(&a, &["iigd_dch.inf".to_string()], 1);
        let lb = bundle_label(&b, &["iigd_dch_d.inf".to_string()], 1);
        assert_ne!(
            normalized_folder_name(&a, &la),
            normalized_folder_name(&b, &lb)
        );
    }

    #[test]
    fn placeholder_class_becomes_unknown() {
        let d = driver(
            "09_ANX",
            Path::new("src/09_ANX"),
            "ANXUcmCxCD.inf",
            "System ; TODO: edit Class",
            "1.9.2.4",
            "h2",
        );
        let label = bundle_label(&d, &["ANXUcmCxCD.inf".to_string()], 1);
        assert_eq!(
            normalized_folder_name(&d, &label),
            "Unknown_Intel_1.9.2.4_ANXUcmCxCD"
        );
    }

    #[test]
    fn missing_provider_becomes_unknown() {
        let mut d = driver(
            "01_Chipset",
            Path::new("src/a"),
            "foo.inf",
            "System",
            "1.0",
            "h",
        );
        d.provider = None;
        let label = bundle_label(&d, &["foo.inf".to_string()], 1);
        assert_eq!(normalized_folder_name(&d, &label), "System_Unknown_1.0_foo");
    }

    #[test]
    fn stampinf_placeholder_version_is_cleaned() {
        let d = driver(
            "03_VGA",
            Path::new("src/03_VGA/Cta"),
            "CtaChildDriver.inf",
            "System",
            "1.0.0.18; TODO: set DriverVer in stampinf property pages",
            "h3",
        );
        let label = bundle_label(&d, &["CtaChildDriver.inf".to_string()], 1);
        assert_eq!(
            normalized_folder_name(&d, &label),
            "System_Intel_1.0.0.18_CtaChildDriver"
        );
    }

    #[test]
    fn missing_version_uses_placeholder() {
        let d = driver(
            "01_Chipset",
            Path::new("src/a"),
            "foo.inf",
            "System",
            "",
            "h4",
        );
        let label = bundle_label(&d, &["foo.inf".to_string()], 1);
        let name = normalized_folder_name(&d, &label);
        assert!(
            name.contains("_NoVersion_"),
            "expected a NoVersion segment in {}",
            name
        );
    }

    #[test]
    fn class_casing_is_canonical() {
        assert_eq!(canonical_class_name("net"), "Net");
        assert_eq!(canonical_class_name("NET"), "Net");
        assert_eq!(canonical_class_name("media"), "MEDIA");
        assert_eq!(canonical_class_name("CustomClass"), "CustomClass");
    }

    #[test]
    fn flat_layout_groups_by_class() {
        let class = resolve_category(Path::new("src"), Path::new("src/oem0_foo"), Some("net"));
        assert_eq!(class, "Net");
    }

    #[test]
    fn grouped_layout_keeps_folder_category() {
        let class = resolve_category(
            Path::new("src"),
            Path::new("src/01_Chipset/DriverFiles/x64"),
            Some("System"),
        );
        assert_eq!(class, "01_Chipset");
    }

    #[test]
    fn device_names_short_list_is_joined() {
        let names = vec!["A".to_string(), "B".to_string()];
        assert_eq!(format_device_names(&names), "A; B");
    }

    #[test]
    fn device_names_long_list_is_complete() {
        let names: Vec<String> = (0..25).map(|i| format!("Dev{}", i)).collect();
        let out = format_device_names(&names);
        // Nothing is truncated, and no summary suffix is added.
        assert!(!out.contains("more"));
        assert_eq!(out.split("; ").count(), 25);
        assert!(out.contains("Dev24"));
    }

    #[test]
    fn device_names_empty() {
        assert_eq!(format_device_names(&[]), "");
    }

    #[test]
    fn hardware_ids_are_complete_and_deduped() {
        let mut ids: Vec<String> = (0..12).map(|i| format!("PCI\\DEV_{}", i)).collect();
        ids.push("PCI\\DEV_0".to_string()); // duplicate
        let out = format_hardware_ids(&ids);
        assert!(!out.contains("more"));
        assert_eq!(out.split("; ").count(), 12);
        assert!(out.contains("PCI\\DEV_11"));
    }

    #[test]
    fn provider_key_is_humanised() {
        assert_eq!(clean_provider_key("PROVIDER_NAME"), "Provider Name");
        assert_eq!(clean_provider_key("Mfg"), "Mfg");
    }

    #[test]
    fn manifest_class_normalises_placeholders_and_casing() {
        assert_eq!(manifest_class_name(Some("net")), "Net");
        assert_eq!(
            manifest_class_name(Some("System ; TODO: edit Class")),
            "Unknown"
        );
        assert_eq!(
            manifest_class_name(Some("System ; TODO: specify appropriate Class")),
            "Unknown"
        );
        assert_eq!(manifest_class_name(None), "Unknown");
        assert_eq!(manifest_class_name(Some("MEDIA")), "MEDIA");
    }

    #[test]
    fn clean_version_strips_todo_suffix() {
        assert_eq!(
            clean_version(Some(
                "1.0.0.16; TODO: set DriverVer in stampinf property pages"
            )),
            "1.0.0.16"
        );
        assert_eq!(clean_version(None), "NoVersion");
    }

    #[test]
    fn driver_date_is_normalised_to_iso() {
        assert_eq!(clean_driver_date(Some("06/17/2026")), "2026-06-17");
        assert_eq!(clean_driver_date(Some("2026/06/17")), "2026-06-17");
        assert_eq!(clean_driver_date(Some("11/2/2025")), "2025-11-02");
        assert_eq!(clean_driver_date(Some("06/17/2026; comment")), "2026-06-17");
        assert_eq!(clean_driver_date(None), "Unknown");
    }

    #[test]
    fn driver_date_invalid_is_returned_as_is() {
        assert_eq!(clean_driver_date(Some("not-a-date")), "not-a-date");
        assert_eq!(clean_driver_date(Some("13/45/2026")), "13/45/2026");
    }
}
