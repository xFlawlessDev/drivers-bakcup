//! The `pack` command: normalize a vendor driver pack into a consistent,
//! upload-ready layout with a manifest.
//!
//! Vendors ship driver packs with inconsistent structures (different nesting,
//! OS sub-folders, mixed casing, duplicate INFs). This module orchestrates the
//! normalization: it discovers packages (see [`crate::pack_discover`]), detects
//! duplicates by content hash, and copies each unique package into a flat,
//! predictably named layout:
//!
//! ```text
//! <out>/<pack_slug>/
//!   manifest.csv              one row per discovered driver package
//!   summary.csv              totals and per-class counts
//!   Drivers/
//!     <Category>/<Driver>_<Version>/
//!       <driver files>
//! ```

use crate::csv_export::{
    relative_path_str, write_pack_manifest, write_pack_summary_csv, ManifestRow,
};
use crate::models::{PackDriver, PackMetadata};
use crate::pack_discover::{
    count_tree, discover_drivers, discover_installer_only_categories, format_device_names,
    format_hardware_ids, normalized_folder_name,
};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Options controlling a pack run.
pub struct PackOptions {
    pub source: PathBuf,
    pub output: PathBuf,
    pub metadata: PackMetadata,
    pub dedup: bool,
    pub copy_files: bool,
    pub verbose: bool,
    pub dry_run: bool,
}

/// Result of a pack run.
pub struct PackResult {
    pub unique: usize,
    pub duplicates: usize,
}

/// Entry point for the `pack` command.
pub fn run_pack(opts: &PackOptions) -> Result<PackResult> {
    if !opts.source.is_dir() {
        bail!("Source must be a directory: {}", opts.source.display());
    }

    println!("Driver Pack Normalizer");
    println!("======================");
    println!("Source: {}", opts.source.display());
    println!("Output: {}", opts.output.display());
    println!("Pack:   {}", opts.metadata.slug());
    println!();

    let drivers = discover_drivers(&opts.source, opts.verbose)?;
    let installer_only = discover_installer_only_categories(&opts.source, &drivers);

    if drivers.is_empty() && installer_only.is_empty() {
        println!("No driver packages (INF files) found under the source.");
        return Ok(PackResult {
            unique: 0,
            duplicates: 0,
        });
    }

    println!(
        "Discovered {} driver package(s) ({} installer-only category folder(s)).",
        drivers.len(),
        installer_only.len()
    );

    let mut rows = plan_layout(&drivers, opts);

    // Categories with no INF (e.g. vendor EXE installers) are still part of the
    // pack; emit a manifest row so they are not silently lost.
    append_installer_only_rows(&mut rows, &installer_only, opts);

    let unique = rows
        .iter()
        .filter(|r| r.duplicate_of.is_empty() && r.status != "installer-only")
        .count();
    let duplicates = rows.iter().filter(|r| !r.duplicate_of.is_empty()).count();
    println!("Unique: {} | Duplicates: {}", unique, duplicates);

    if opts.dry_run {
        println!("\nDry run: no files were written.");
        print_preview(&rows);
        return Ok(PackResult { unique, duplicates });
    }

    let pack_root = opts.output.join(opts.metadata.slug());

    if opts.copy_files {
        materialize(&rows, &drivers, &pack_root, opts.verbose)?;
    }

    fs::create_dir_all(&pack_root)
        .with_context(|| format!("Failed to create {}", pack_root.display()))?;

    let manifest_path = pack_root.join("manifest.csv");
    write_pack_manifest(&rows, &manifest_path)?;
    println!("\nManifest written: {}", manifest_path.display());

    let summary_path = pack_root.join("summary.csv");
    write_pack_summary_csv(&rows, &summary_path)?;
    println!("Summary written:  {}", summary_path.display());

    println!("\nNormalized pack ready at: {}", pack_root.display());

    Ok(PackResult { unique, duplicates })
}

/// A merged group of packages that describe the same driver.
struct DriverGroup<'a> {
    /// The representative package whose files will be copied.
    primary: &'a PackDriver,
    /// Source folders of the other packages merged into this group.
    others: Vec<&'a PackDriver>,
    /// Union of device names across all members.
    device_names: Vec<String>,
    /// Union of hardware IDs across all members.
    hardware_ids: Vec<String>,
    /// Total device entries across all members.
    device_count: usize,
}

/// Build the planned manifest rows.
///
/// Packages are grouped by [`PackDriver::merge_key`] (device class + device name +
/// driver version), matching how the `export` command collapses a single driver
/// that ships as several OEM packages. Pure duplicates (identical INF content)
/// within a group are noted via `Duplicate Of`.
fn plan_layout(drivers: &[PackDriver], opts: &PackOptions) -> Vec<ManifestRow> {
    let slug = opts.metadata.slug();

    // Preserve first-seen order for stable output.
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, DriverGroup> = HashMap::new();

    for (idx, driver) in drivers.iter().enumerate() {
        let key = if opts.dedup {
            driver.merge_key()
        } else {
            // Disable merging: make each package its own group.
            format!("__{}__{}", idx, driver.inf_hash)
        };

        match groups.get_mut(&key) {
            Some(group) => {
                group.others.push(driver);
                for name in &driver.device_names {
                    group.device_names.push(name.clone());
                }
                for id in &driver.hardware_ids {
                    group.hardware_ids.push(id.clone());
                }
                group.device_count += driver.device_count;
            }
            None => {
                order.push(key.clone());
                groups.insert(
                    key,
                    DriverGroup {
                        primary: driver,
                        others: Vec::new(),
                        device_names: driver.device_names.clone(),
                        hardware_ids: driver.hardware_ids.clone(),
                        device_count: driver.device_count,
                    },
                );
            }
        }
    }

    let mut rows = Vec::new();
    let mut used_folders: HashMap<String, usize> = HashMap::new();
    for key in order {
        let group = &groups[&key];
        let driver = group.primary;

        let mut device_names = group.device_names.clone();
        device_names.sort();
        device_names.dedup();
        let mut hardware_ids = group.hardware_ids.clone();
        hardware_ids.sort();
        hardware_ids.dedup();

        // Ensure the folder name is unique. Two distinct packages can otherwise
        // resolve to the same name (e.g. `heci.inf` shipped for win8 and win10).
        let base_folder = normalized_folder_name(driver);
        let folder = match used_folders.get_mut(&base_folder) {
            Some(count) => {
                *count += 1;
                format!("{}_{}", base_folder, *count)
            }
            None => {
                used_folders.insert(base_folder.clone(), 1);
                base_folder
            }
        };

        let source_rel = relative_path_str(&driver.source_dir);
        let merged_from: Vec<String> = group
            .others
            .iter()
            .map(|d| relative_path_str(&d.source_dir))
            .collect();

        // Duplicates within the group: identical INF content to another member.
        let duplicate_of = group
            .others
            .iter()
            .find(|d| d.inf_hash == driver.inf_hash)
            .map(|_| folder.clone())
            .unwrap_or_default();

        let status = if !merged_from.is_empty() {
            "merged"
        } else {
            "unique"
        };

        rows.push(ManifestRow {
            pack: slug.clone(),
            odm: opts.metadata.odm.clone().unwrap_or_default(),
            model: opts.metadata.model.clone().unwrap_or_default(),
            revision: opts.metadata.revision.clone().unwrap_or_default(),
            category: driver
                .category
                .clone()
                .unwrap_or_else(|| "Uncategorized".to_string()),
            device_class: driver
                .device_class
                .clone()
                .unwrap_or_else(|| "Unknown".to_string()),
            driver_name: crate::util::strip_inf_extension(&driver.inf_name).to_string(),
            driver_version: driver
                .driver_version
                .clone()
                .unwrap_or_else(|| "Unknown".to_string()),
            provider: driver
                .provider
                .clone()
                .unwrap_or_else(|| "Unknown".to_string()),
            device_count: group.device_count,
            device_name_count: device_names.len(),
            device_names: format_device_names(&device_names),
            hardware_id_count: hardware_ids.len(),
            hardware_ids: format_hardware_ids(&hardware_ids),
            normalized_folder: folder,
            source_folder: source_rel,
            merged_from: merged_from.join("; "),
            file_count: driver.files.len(),
            total_bytes: driver.total_bytes,
            inf_hash: driver.inf_hash.clone(),
            duplicate_of,
            status: status.to_string(),
        });
    }

    rows
}

/// Append manifest rows for installer-only categories.
fn append_installer_only_rows(
    rows: &mut Vec<ManifestRow>,
    categories: &[PathBuf],
    opts: &PackOptions,
) {
    for dir in categories {
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Uncategorized".to_string());

        let (file_count, total_bytes) = count_tree(dir);

        rows.push(ManifestRow {
            pack: opts.metadata.slug(),
            odm: opts.metadata.odm.clone().unwrap_or_default(),
            model: opts.metadata.model.clone().unwrap_or_default(),
            revision: opts.metadata.revision.clone().unwrap_or_default(),
            category: name.clone(),
            device_class: "Installer".to_string(),
            driver_name: name.clone(),
            driver_version: "None".to_string(),
            provider: String::new(),
            device_count: 0,
            device_names: String::new(),
            device_name_count: 0,
            hardware_id_count: 0,
            hardware_ids: String::new(),
            normalized_folder: name.clone(),
            source_folder: relative_path_str(dir),
            merged_from: String::new(),
            file_count,
            total_bytes,
            inf_hash: String::new(),
            duplicate_of: String::new(),
            status: "installer-only".to_string(),
        });
    }
}

/// Copy unique driver packages into the normalized flat layout.
fn materialize(
    rows: &[ManifestRow],
    drivers: &[PackDriver],
    pack_root: &Path,
    verbose: bool,
) -> Result<()> {
    // Map source folder -> driver so we can copy the right files.
    let by_source: HashMap<String, &PackDriver> = drivers
        .iter()
        .map(|d| (relative_path_str(&d.source_dir), d))
        .collect();

    for row in rows {
        // Installer-only categories are copied as whole trees.
        if row.status == "installer-only" {
            let src = PathBuf::from(row.source_folder.replace('/', "\\"));
            let src = if src.is_dir() {
                src
            } else {
                PathBuf::from(&row.source_folder)
            };
            if src.is_dir() {
                let dest = pack_root.join(&row.category);
                copy_tree(&src, &dest, verbose)?;
                if verbose {
                    println!("  Packed installer-only {}", row.category);
                }
            }
            continue;
        }

        // Merged groups are represented by their primary package; the other
        // members are recorded in `Merged From` but not copied again.
        let driver = match by_source.get(&row.source_folder) {
            Some(d) => *d,
            None => continue,
        };

        let dest = pack_root.join(&row.normalized_folder);
        fs::create_dir_all(&dest)
            .with_context(|| format!("Failed to create {}", dest.display()))?;

        for file in &driver.files {
            let src = driver.source_dir.join(file);
            if src.is_dir() {
                continue;
            }
            let target = dest.join(file);
            if let Some(parent) = target.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Err(e) = fs::copy(&src, &target) {
                if verbose {
                    eprintln!(
                        "Warning: failed to copy {} -> {}: {}",
                        src.display(),
                        target.display(),
                        e
                    );
                }
            }
        }

        if verbose {
            println!(
                "  Packed {} -> {}",
                row.source_folder, row.normalized_folder
            );
        }
    }

    Ok(())
}

/// Recursively copy a directory tree.
fn copy_tree(src: &Path, dest: &Path, verbose: bool) -> Result<()> {
    fs::create_dir_all(dest).with_context(|| format!("Failed to create {}", dest.display()))?;
    let entries = match fs::read_dir(src) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &target, verbose)?;
        } else if path.is_file() {
            if let Err(e) = fs::copy(&path, &target) {
                if verbose {
                    eprintln!("Warning: failed to copy {}: {}", path.display(), e);
                }
            }
        }
    }
    Ok(())
}

fn print_preview(rows: &[ManifestRow]) {
    println!("\nPlanned layout (first 25):");
    for row in rows.iter().take(25) {
        let marker = if row.duplicate_of.is_empty() {
            " "
        } else {
            "="
        };
        println!(
            "  [{}] {}  <-  {}",
            marker, row.normalized_folder, row.source_folder
        );
    }
    if rows.len() > 25 {
        println!("  ... and {} more", rows.len() - 25);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PackMetadata;

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

    fn opts() -> PackOptions {
        PackOptions {
            source: PathBuf::from("src"),
            output: PathBuf::from("out"),
            metadata: PackMetadata {
                odm: Some("Clevo".into()),
                model: Some("765".into()),
                revision: Some("V2xxRNP_1.08.06_0911".into()),
            },
            dedup: true,
            copy_files: false,
            verbose: false,
            dry_run: true,
        }
    }

    #[test]
    fn merges_same_driver_from_multiple_oem_packages() {
        // oem37..oem45 style: same class+device+version, different INF hashes.
        let ds = vec![
            driver(
                "Bluetooth",
                Path::new("src/oem37"),
                "ibtusb.inf",
                "Bluetooth",
                "23.150.0.3",
                "hA",
            ),
            driver(
                "Bluetooth",
                Path::new("src/oem38"),
                "ibtusb.inf",
                "Bluetooth",
                "23.150.0.3",
                "hB",
            ),
            driver(
                "Bluetooth",
                Path::new("src/oem45"),
                "ibtusb.inf",
                "Bluetooth",
                "23.150.0.3",
                "hC",
            ),
        ];
        let rows = plan_layout(&ds, &opts());
        assert_eq!(rows.len(), 1, "should merge into a single package");
        assert_eq!(rows[0].status, "merged");
        assert_eq!(
            rows[0].normalized_folder,
            "Bluetooth_Intel_23.150.0.3_ibtusb"
        );
        assert!(rows[0].merged_from.contains("oem38"));
        assert!(rows[0].merged_from.contains("oem45"));
    }

    #[test]
    fn does_not_merge_different_versions() {
        let ds = vec![
            driver(
                "Display",
                Path::new("src/a"),
                "x.inf",
                "Display",
                "1.0",
                "h1",
            ),
            driver(
                "Display",
                Path::new("src/b"),
                "x.inf",
                "Display",
                "2.0",
                "h2",
            ),
        ];
        let rows = plan_layout(&ds, &opts());
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn keep_duplicates_disables_merging() {
        let ds = vec![
            driver(
                "Bluetooth",
                Path::new("src/a"),
                "ibtusb.inf",
                "Bluetooth",
                "1.0",
                "h1",
            ),
            driver(
                "Bluetooth",
                Path::new("src/b"),
                "ibtusb.inf",
                "Bluetooth",
                "1.0",
                "h2",
            ),
        ];
        let mut o = opts();
        o.dedup = false;
        let rows = plan_layout(&ds, &o);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn manifest_row_includes_device_names_and_hardware_ids() {
        let ds = vec![driver(
            "03_VGA",
            Path::new("src/a"),
            "iigd_dch.inf",
            "Display",
            "32.0.101.6556",
            "h",
        )];
        let rows = plan_layout(&ds, &opts());
        assert_eq!(rows[0].device_names, "Intel(R) UHD Graphics");
        assert_eq!(rows[0].device_name_count, 1);
        assert_eq!(rows[0].hardware_ids, "PCI\\VEN_8086&DEV_4680");
        assert_eq!(rows[0].hardware_id_count, 1);
    }

    #[test]
    fn merged_group_unions_hardware_ids() {
        let mut a = driver("Net", Path::new("src/a"), "rt.inf", "Net", "1.0", "h1");
        a.hardware_ids = vec!["PCI\\VEN_10EC&DEV_8168".to_string()];
        let mut b = driver("Net", Path::new("src/b"), "rt.inf", "Net", "1.0", "h2");
        b.hardware_ids = vec!["PCI\\VEN_10EC&DEV_8125".to_string()];

        let rows = plan_layout(&[a, b], &opts());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].hardware_id_count, 2);
        assert!(rows[0].hardware_ids.contains("DEV_8168"));
        assert!(rows[0].hardware_ids.contains("DEV_8125"));
    }

    #[test]
    fn pack_slug_is_filesystem_safe() {
        let m = PackMetadata {
            odm: Some("Clevo Inc.".into()),
            model: Some("765".into()),
            revision: Some("V2xxRNP_1.08.06_0911".into()),
        };
        assert_eq!(m.slug(), "Clevo_Inc._765_V2xxRNP_1.08.06_0911");
    }
}
