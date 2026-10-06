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
    count_tree, discover_drivers, discover_installer_only_categories, normalized_folder_name,
};
use anyhow::{bail, Context, Result};
use std::collections::{HashMap, HashSet};
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

    let (mut rows, _) = plan_layout(&drivers, opts);

    // Categories with no INF (e.g. vendor EXE installers) are still part of the
    // pack; emit a manifest row so they are not silently lost.
    append_installer_only_rows(&mut rows, &installer_only, opts);

    let unique = rows.iter().filter(|r| r.duplicate_of.is_empty()).count();
    let duplicates = rows.len() - unique;
    println!("Unique: {} | Duplicates: {}", unique, duplicates);

    if opts.dry_run {
        println!("\nDry run: no files were written.");
        print_preview(&rows);
        return Ok(PackResult { unique, duplicates });
    }

    let pack_root = opts.output.join(opts.metadata.slug());
    let drivers_root = pack_root.join("Drivers");

    if opts.copy_files {
        materialize(&rows, &drivers, &drivers_root, opts.verbose)?;
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

/// Build the planned manifest rows, detecting duplicates by INF hash.
fn plan_layout(
    drivers: &[PackDriver],
    opts: &PackOptions,
) -> (Vec<ManifestRow>, HashMap<String, String>) {
    let mut rows = Vec::new();
    // INF hash -> normalized folder of the first occurrence.
    let mut seen: HashMap<String, String> = HashMap::new();
    let slug = opts.metadata.slug();

    for driver in drivers {
        let category = driver
            .category
            .clone()
            .unwrap_or_else(|| "Uncategorized".to_string());
        let folder = normalized_folder_name(&category, driver);
        let normalized_rel = format!("Drivers/{}/{}", category, folder);
        let source_rel = relative_path_str(&driver.source_dir);

        let duplicate_of = if opts.dedup {
            seen.get(&driver.inf_hash).cloned().unwrap_or_default()
        } else {
            String::new()
        };
        let is_duplicate = !duplicate_of.is_empty();

        if !is_duplicate {
            seen.entry(driver.inf_hash.clone())
                .or_insert_with(|| normalized_rel.clone());
        }

        rows.push(ManifestRow {
            pack: slug.clone(),
            odm: opts.metadata.odm.clone().unwrap_or_default(),
            model: opts.metadata.model.clone().unwrap_or_default(),
            revision: opts.metadata.revision.clone().unwrap_or_default(),
            category: category.clone(),
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
            device_count: driver.device_count,
            normalized_folder: normalized_rel,
            source_folder: source_rel,
            file_count: driver.files.len(),
            total_bytes: driver.total_bytes,
            inf_hash: driver.inf_hash.clone(),
            duplicate_of,
            status: if is_duplicate { "duplicate" } else { "unique" }.to_string(),
        });
    }

    (rows, seen)
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
            normalized_folder: format!("Drivers/{}", name),
            source_folder: relative_path_str(dir),
            file_count,
            total_bytes,
            inf_hash: String::new(),
            duplicate_of: String::new(),
            status: "installer-only".to_string(),
        });
    }
}

/// Copy unique driver packages into the normalized layout.
fn materialize(
    rows: &[ManifestRow],
    drivers: &[PackDriver],
    drivers_root: &Path,
    verbose: bool,
) -> Result<()> {
    // Map source folder -> driver so we can copy the right files.
    let by_source: HashMap<String, &PackDriver> = drivers
        .iter()
        .map(|d| (relative_path_str(&d.source_dir), d))
        .collect();

    let mut copied_dirs: HashSet<String> = HashSet::new();

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
                let dest = drivers_root.join(&row.category);
                copy_tree(&src, &dest, verbose)?;
                if verbose {
                    println!(
                        "  Packed installer-only {} -> Drivers/{}",
                        row.category, row.category
                    );
                }
            }
            continue;
        }

        if !row.duplicate_of.is_empty() {
            if verbose {
                println!("  Skipping duplicate: {}", row.source_folder);
            }
            continue;
        }

        let driver = match by_source.get(&row.source_folder) {
            Some(d) => *d,
            None => continue,
        };

        if !copied_dirs.insert(row.normalized_folder.clone()) {
            // Two different INFs resolved to the same folder name; disambiguate.
            continue;
        }

        let dest = drivers_root.join(row.normalized_folder.replace("Drivers/", ""));
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
    fn duplicate_detection_by_hash() {
        let ds = vec![
            driver(
                "01_Chipset",
                Path::new("src/a"),
                "AlderLakeSystem.inf",
                "System",
                "1.0",
                "same",
            ),
            driver(
                "01_Chipset",
                Path::new("src/b"),
                "AlderLakeSystem.inf",
                "System",
                "1.0",
                "same",
            ),
        ];
        let (rows, _) = plan_layout(&ds, &opts());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].status, "unique");
        assert_eq!(rows[1].status, "duplicate");
        assert!(!rows[1].duplicate_of.is_empty());
    }

    #[test]
    fn keep_duplicates_disables_dedup() {
        let ds = vec![
            driver(
                "01_Chipset",
                Path::new("src/a"),
                "X.inf",
                "System",
                "1.0",
                "same",
            ),
            driver(
                "01_Chipset",
                Path::new("src/b"),
                "X.inf",
                "System",
                "1.0",
                "same",
            ),
        ];
        let mut o = opts();
        o.dedup = false;
        let (rows, _) = plan_layout(&ds, &o);
        assert!(rows.iter().all(|r| r.status == "unique"));
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
