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
            provider,
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

/// Collect the files that belong to a driver package.
///
/// A package is the INF plus everything under its own folder, excluding nested
/// folders that themselves contain an INF (those are separate packages). This
/// keeps self-contained bundles (with `Config/`, `DLLs/`, etc.) intact without
/// swallowing sibling packages.
fn collect_package_files(source_dir: &Path, inf_name: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();

    // The INF itself.
    files.push(PathBuf::from(inf_name));

    if let Ok(entries) = fs::read_dir(source_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Skip sub-folders that are their own driver package.
                if contains_inf(&path) {
                    continue;
                }
                collect_tree_files(source_dir, &path, &mut files);
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

/// Compute the normalized folder name for a driver.
///
/// Format: `<Class>_<Provider>_<Version>_<InfName>`. The INF name keeps the
/// folder unique when a provider ships several packages at the same version
/// (e.g. `iigd_dch` and `iigd_dch_d`), which would otherwise collide.
pub fn normalized_folder_name(driver: &PackDriver) -> String {
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
    let inf_stem = crate::util::strip_inf_extension(&driver.inf_name);

    sanitize_folder_name(&format!("{}_{}_{}_{}", class, provider, version, inf_stem))
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

/// True for INF classes that vendors leave as unresolved placeholders.
pub fn is_placeholder_class(class: &str) -> bool {
    let lower = class.to_lowercase();
    lower.contains("todo") || lower.contains("unknown") || lower.trim().is_empty()
}

/// Maximum number of entries listed in a manifest row before summarising.
pub const MAX_MANIFEST_LIST: usize = 10;

/// Format a list for the manifest, capped with a `+N more` suffix.
///
/// Items are expected to be unique and sorted. At most [`MAX_MANIFEST_LIST`] are
/// listed so packages with hundreds of devices (e.g. GPU INFs) stay readable.
pub fn format_list(items: &[String]) -> String {
    format_list_capped(items, MAX_MANIFEST_LIST)
}

/// Format a list, capping at `max` items.
pub fn format_list_capped(items: &[String], max: usize) -> String {
    if items.is_empty() {
        return String::new();
    }
    if items.len() <= max {
        return items.join("; ");
    }
    let shown = items[..max].join("; ");
    format!("{}; +{} more", shown, items.len() - max)
}

/// Format a package's device names for the manifest.
pub fn format_device_names(names: &[String]) -> String {
    format_list(names)
}

/// Format a package's hardware IDs for the manifest.
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
            provider: Some("Intel".to_string()),
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
        assert_eq!(
            normalized_folder_name(&d),
            "Display_Intel_32.0.15.7705_nvcvegpu"
        );
    }

    #[test]
    fn folder_name_is_unique_per_inf() {
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
        assert_ne!(normalized_folder_name(&a), normalized_folder_name(&b));
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
        assert_eq!(
            normalized_folder_name(&d),
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
        assert_eq!(normalized_folder_name(&d), "System_Unknown_1.0_foo");
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
        assert_eq!(
            normalized_folder_name(&d),
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
        assert!(
            normalized_folder_name(&d).contains("_NoVersion_"),
            "expected a NoVersion segment in {}",
            normalized_folder_name(&d)
        );
    }

    #[test]
    fn merge_key_ignores_source_folder() {
        // Same class + device name + version, different source folders/hashes.
        let a = driver(
            "Bluetooth",
            Path::new("src/oem37"),
            "ibtusb.inf",
            "Bluetooth",
            "23.150.0.3",
            "hA",
        );
        let b = driver(
            "Bluetooth",
            Path::new("src/oem45"),
            "ibtusb.inf",
            "Bluetooth",
            "23.150.0.3",
            "hB",
        );
        assert_eq!(a.merge_key(), b.merge_key());
        assert_ne!(a.inf_hash, b.inf_hash);
    }

    #[test]
    fn merge_key_distinguishes_versions() {
        let a = driver(
            "Display",
            Path::new("src/a"),
            "x.inf",
            "Display",
            "1.0",
            "h1",
        );
        let b = driver(
            "Display",
            Path::new("src/b"),
            "x.inf",
            "Display",
            "2.0",
            "h2",
        );
        assert_ne!(a.merge_key(), b.merge_key());
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
    fn device_names_long_list_is_capped() {
        let names: Vec<String> = (0..25).map(|i| format!("Dev{}", i)).collect();
        let out = format_device_names(&names);
        assert!(out.ends_with("+15 more"));
        assert!(out.starts_with("Dev0; Dev1"));
    }

    #[test]
    fn device_names_empty() {
        assert_eq!(format_device_names(&[]), "");
    }

    #[test]
    fn hardware_ids_are_capped_like_names() {
        let ids: Vec<String> = (0..12).map(|i| format!("PCI\\DEV_{}", i)).collect();
        let out = format_hardware_ids(&ids);
        assert!(out.ends_with("+2 more"));
        assert!(out.starts_with("PCI\\DEV_0"));
    }

    #[test]
    fn provider_key_is_humanised() {
        assert_eq!(clean_provider_key("PROVIDER_NAME"), "Provider Name");
        assert_eq!(clean_provider_key("Mfg"), "Mfg");
    }
}
