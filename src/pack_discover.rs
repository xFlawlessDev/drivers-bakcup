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

        drivers.push(PackDriver {
            category: Some(category),
            source_dir,
            inf_name,
            device_class,
            driver_version: parsed.raw_version_info.driver_version.clone(),
            provider: parsed.raw_version_info.provider.clone(),
            device_count: parsed.drivers.len(),
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

/// Determine the category folder for a driver.
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

/// Compute the normalized folder name for a driver within a category.
pub fn normalized_folder_name(category: &str, driver: &PackDriver) -> String {
    let class = driver.device_class.as_deref().unwrap_or("Unknown");
    let stem = crate::util::strip_inf_extension(&driver.inf_name);
    let version = clean_version(driver.driver_version.as_deref());

    // Prefer a class-derived label but fall back to the category when the class
    // is missing or clearly a placeholder ("System ; TODO: edit Class").
    let class_label = if is_placeholder_class(class) {
        category.trim().to_string()
    } else {
        canonical_class_name(class.trim())
    };

    sanitize_folder_name(&format!("{}_{}_{}", class_label, stem, version))
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
            category: Some(category.to_string()),
            inf_hash: hash.to_string(),
            files: vec![PathBuf::from(inf)],
            total_bytes: 10,
        }
    }

    #[test]
    fn folder_name_uses_class_and_version() {
        let d = driver(
            "03_VGA",
            Path::new("src/03_VGA/Nvidia"),
            "nvcvegpu.inf",
            "Display",
            "32.0.15.7705",
            "h1",
        );
        assert_eq!(
            normalized_folder_name("03_VGA", &d),
            "Display_nvcvegpu_32.0.15.7705"
        );
    }

    #[test]
    fn placeholder_class_falls_back_to_category() {
        let d = driver(
            "09_ANX",
            Path::new("src/09_ANX"),
            "ANXUcmCxCD.inf",
            "System ; TODO: edit Class",
            "1.9.2.4",
            "h2",
        );
        assert_eq!(
            normalized_folder_name("09_ANX", &d),
            "09_ANX_ANXUcmCxCD_1.9.2.4"
        );
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
            normalized_folder_name("03_VGA", &d),
            "System_CtaChildDriver_1.0.0.18"
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
        assert!(normalized_folder_name("01_Chipset", &d).ends_with("_NoVersion"));
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
}
