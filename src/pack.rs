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
    bundle_label, clean_driver_date, clean_version, count_tree, discover_drivers,
    discover_installer_only_categories, format_device_names, format_hardware_ids,
    manifest_class_name, normalized_folder_name,
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
    /// Zip each package folder after copying.
    pub zip: bool,
    /// Keep the source folder after zipping (only meaningful with `zip`).
    pub keep_folders: bool,
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

    // Mark packages that a newer version of the same driver supersedes.
    let superseded = mark_superseded(&mut rows);
    if superseded > 0 {
        println!("Superseded packages: {}", superseded);
    }

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

    // Optionally zip each package folder, then drop the folder unless asked to
    // keep it. The manifest is written before zipping so it can record archives.
    if opts.zip && opts.copy_files {
        zip_packages(&mut rows, &pack_root, opts)?;
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

/// One installer bundle: INFs that share a source folder.
struct Bundle<'a> {
    /// The representative INF, used for the folder name and metadata.
    primary: &'a PackDriver,
    /// All INF names in this bundle (including the primary).
    inf_names: Vec<String>,
    /// Source folders contributing to this bundle.
    source_dirs: Vec<PathBuf>,
    /// Union of device names across all members.
    device_names: Vec<String>,
    /// Union of hardware IDs across all members.
    hardware_ids: Vec<String>,
    /// Total device entries across all members.
    device_count: usize,
}

impl<'a> Bundle<'a> {
    fn new(driver: &'a PackDriver) -> Self {
        Self {
            primary: driver,
            inf_names: vec![driver.inf_name.clone()],
            source_dirs: vec![driver.source_dir.clone()],
            device_names: driver.device_names.clone(),
            hardware_ids: driver.hardware_ids.clone(),
            device_count: driver.device_count,
        }
    }

    fn absorb(&mut self, driver: &'a PackDriver) {
        if !self.inf_names.contains(&driver.inf_name) {
            self.inf_names.push(driver.inf_name.clone());
        }
        if !self.source_dirs.contains(&driver.source_dir) {
            self.source_dirs.push(driver.source_dir.clone());
        }
        self.device_names.extend(driver.device_names.clone());
        self.hardware_ids.extend(driver.hardware_ids.clone());
        self.device_count += driver.device_count;
    }

    /// Merge another bundle (same driver shipped in other folders).
    fn merge(&mut self, other: &Bundle<'a>) {
        for name in &other.inf_names {
            if !self.inf_names.contains(name) {
                self.inf_names.push(name.clone());
            }
        }
        for dir in &other.source_dirs {
            if !self.source_dirs.contains(dir) {
                self.source_dirs.push(dir.clone());
            }
        }
        self.device_names.extend(other.device_names.clone());
        self.hardware_ids.extend(other.hardware_ids.clone());
        self.device_count += other.device_count;
    }
}

/// Group key identifying an installer bundle (INF + payload in one folder).
///
/// A bundle is everything shipped in one source folder. Vendors stage a complete
/// installer there and the payload (DLLs, `.cat`, `.sys`) is shared between every
/// INF in it, so the folder is the correct unit to copy. Keying by folder rather
/// than by catalog avoids splitting a multi-catalog folder — such as Intel
/// graphics, which contains both `igdlh.cat` and `extinf.cat` — into two packages
/// that each need the whole payload.
fn bundle_key(driver: &PackDriver) -> String {
    format!(
        "dir:{}",
        relative_path_str(&driver.source_dir).to_lowercase()
    )
}

/// Merge key identifying the same driver shipped as several folder bundles.
///
/// Some vendors ship one driver as many single-INF folders (e.g. Intel Bluetooth
/// `ibtusb` under `CCP`, `GAP`, `GFP`, ...). Their INFs differ slightly but the
/// driver identity — INF stem + device name + version — is the same, so those
/// bundles are merged into one output package.
fn merge_bundle_key(bundle: &Bundle) -> String {
    let primary = bundle.primary;
    let inf_stem = crate::util::strip_inf_extension(&primary.inf_name).to_lowercase();
    let device = bundle
        .device_names
        .first()
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_default();
    let version = primary
        .driver_version
        .as_deref()
        .unwrap_or("Unknown")
        .trim()
        .to_lowercase();
    format!("{}|{}|{}", inf_stem, device, version)
}

/// Build the planned manifest rows, one per output package.
///
/// Two phases:
/// 1. **Bundle** — INFs sharing a source folder become one installer bundle, so
///    shared payload is copied once (e.g. Intel graphics: 11 INFs, one folder).
/// 2. **Merge** — bundles describing the same driver (INF stem + device + version)
///    are collapsed, so a driver shipped as many single-INF folders becomes one
///    package (e.g. Intel Bluetooth `ibtusb` under `CCP`/`GAP`/...).
fn plan_layout(drivers: &[PackDriver], opts: &PackOptions) -> Vec<ManifestRow> {
    let slug = opts.metadata.slug();

    // Phase 1: bundle by source folder, preserving first-seen order.
    let mut folder_order: Vec<String> = Vec::new();
    let mut by_folder: HashMap<String, Bundle> = HashMap::new();
    for driver in drivers {
        let key = bundle_key(driver);
        match by_folder.get_mut(&key) {
            Some(bundle) => bundle.absorb(driver),
            None => {
                folder_order.push(key.clone());
                by_folder.insert(key, Bundle::new(driver));
            }
        }
    }

    // Phase 2: merge bundles that describe the same driver.
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Bundle> = HashMap::new();
    if opts.dedup {
        for folder_key in &folder_order {
            let bundle = &by_folder[folder_key];
            let key = merge_bundle_key(bundle);
            match groups.get_mut(&key) {
                Some(existing) => existing.merge(bundle),
                None => {
                    order.push(key.clone());
                    groups.insert(
                        key,
                        Bundle {
                            primary: bundle.primary,
                            inf_names: bundle.inf_names.clone(),
                            source_dirs: bundle.source_dirs.clone(),
                            device_names: bundle.device_names.clone(),
                            hardware_ids: bundle.hardware_ids.clone(),
                            device_count: bundle.device_count,
                        },
                    );
                }
            }
        }
    } else {
        for (idx, folder_key) in folder_order.iter().enumerate() {
            let bundle = &by_folder[folder_key];
            let key = format!("__{}__", idx);
            order.push(key.clone());
            groups.insert(
                key,
                Bundle {
                    primary: bundle.primary,
                    inf_names: bundle.inf_names.clone(),
                    source_dirs: bundle.source_dirs.clone(),
                    device_names: bundle.device_names.clone(),
                    hardware_ids: bundle.hardware_ids.clone(),
                    device_count: bundle.device_count,
                },
            );
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
        let label = bundle_label(driver, &group.inf_names, group.source_dirs.len());
        let base_folder = normalized_folder_name(driver, &label);
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

        // The folder holds every INF in the bundle; list them in the manifest.
        let mut bundle_infs = group.inf_names.clone();
        bundle_infs.sort();
        bundle_infs.dedup();
        let bundle_label = bundle_infs
            .iter()
            .map(|n| crate::util::strip_inf_extension(n))
            .collect::<Vec<_>>()
            .join(", ");

        // Extra source folders in the bundle (same catalog shipped elsewhere).
        let merged_from: Vec<String> = group
            .source_dirs
            .iter()
            .filter(|d| d.as_path() != driver.source_dir.as_path())
            .map(|d| relative_path_str(d))
            .collect();

        let status = if !merged_from.is_empty() || bundle_infs.len() > 1 {
            "package"
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
            device_class: manifest_class_name(driver.device_class.as_deref()),
            driver_name: bundle_label,
            driver_version: clean_version(driver.driver_version.as_deref()),
            driver_date: clean_driver_date(driver.driver_date.as_deref()),
            provider: driver
                .provider
                .clone()
                .unwrap_or_else(|| "Unknown".to_string()),
            catalog: driver
                .catalog_file
                .clone()
                .unwrap_or_else(|| "None".to_string()),
            device_count: group.device_count,
            device_name_count: device_names.len(),
            device_names: format_device_names(&device_names),
            hardware_id_count: hardware_ids.len(),
            hardware_ids: format_hardware_ids(&hardware_ids),
            normalized_folder: folder,
            source_folder: source_rel,
            merged_from: merged_from.join("; "),
            archive: String::new(),
            superseded_by: String::new(),
            file_count: driver.files.len(),
            total_bytes: driver.total_bytes,
            inf_hash: driver.inf_hash.clone(),
            duplicate_of: String::new(),
            status: status.to_string(),
        });
    }

    rows
}

/// Mark packages that a newer version supersedes.
///
/// A pack can ship two versions of the same driver (for example `mshdadac`
/// 30.0.101.1371 and 31.0.101.2125). Both are kept — Windows ranks them by
/// version — but older ones are flagged so QA can see which is current.
///
/// Two rows are compared when they share the same INF name and at least one
/// hardware ID, and the older version is marked. Overlap (not exact equality) is
/// used because a newer INF often lists a subset of the older one's IDs.
///
/// Returns the number of rows marked `superseded`.
fn mark_superseded(rows: &mut [ManifestRow]) -> usize {
    // Group row indices by INF name so only related packages are compared.
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, row) in rows.iter().enumerate() {
        if row.status == "installer-only" || row.hardware_ids.is_empty() {
            continue;
        }
        by_name
            .entry(row.driver_name.to_lowercase())
            .or_default()
            .push(idx);
    }

    // Collect (old_idx, superseding_folder) pairs first to avoid borrowing rows
    // mutably while reading them.
    let mut marks: Vec<(usize, String)> = Vec::new();

    for indices in by_name.values() {
        if indices.len() < 2 {
            continue;
        }

        for &candidate in indices {
            // Find the highest version among rows that overlap this candidate's
            // hardware IDs and share its source folder family.
            let mut best: Option<(usize, Vec<u64>)> = None;

            for &other in indices {
                if other == candidate {
                    continue;
                }
                if !hardware_overlaps(&rows[candidate], &rows[other]) {
                    continue;
                }
                let order =
                    compare_versions(&rows[other].driver_version, &rows[candidate].driver_version);
                if order == std::cmp::Ordering::Greater {
                    let parsed = parse_version(&rows[other].driver_version);
                    let better = best.as_ref().map(|(_, v)| parsed > *v).unwrap_or(true);
                    if better {
                        best = Some((other, parsed));
                    }
                }
            }

            if let Some((winner, _)) = best {
                marks.push((candidate, rows[winner].normalized_folder.clone()));
            }
        }
    }

    let count = marks.len();
    for (idx, folder) in marks {
        rows[idx].superseded_by = folder;
        rows[idx].status = "superseded".to_string();
    }
    count
}

/// True when two rows share at least one hardware ID.
fn hardware_overlaps(a: &ManifestRow, b: &ManifestRow) -> bool {
    let split = |s: &str| -> Vec<String> {
        s.split("; ")
            .map(|x| x.trim().to_lowercase())
            .filter(|x| !x.is_empty())
            .collect()
    };
    let (set_a, set_b) = (split(&a.hardware_ids), split(&b.hardware_ids));
    if set_a.is_empty() || set_b.is_empty() {
        return false;
    }
    set_a.iter().any(|id| set_b.contains(id))
}

/// Parse a dotted version string into numeric components.
fn parse_version(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<u64>().ok())
        .collect()
}

/// Compare two dotted version strings numerically (`31.0.101.2125` > `30.0.101.1371`).
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let (va, vb) = (parse_version(a), parse_version(b));
    for i in 0..va.len().max(vb.len()) {
        let x = va.get(i).copied().unwrap_or(0);
        let y = vb.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
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
            driver_date: "None".to_string(),
            provider: String::new(),
            catalog: String::new(),
            device_count: 0,
            device_names: String::new(),
            device_name_count: 0,
            hardware_id_count: 0,
            hardware_ids: String::new(),
            normalized_folder: name.clone(),
            source_folder: relative_path_str(dir),
            merged_from: String::new(),
            archive: String::new(),
            superseded_by: String::new(),
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
    // Map source folder -> every INF discovered there.
    let mut by_source: HashMap<String, Vec<&PackDriver>> = HashMap::new();
    for d in drivers {
        by_source
            .entry(relative_path_str(&d.source_dir))
            .or_default()
            .push(d);
    }

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

        let dest = pack_root.join(&row.normalized_folder);
        fs::create_dir_all(&dest)
            .with_context(|| format!("Failed to create {}", dest.display()))?;

        // Source folders that contribute to this bundle: the representative's
        // folder plus any other members recorded in `Merged From`.
        let mut source_folders: Vec<PathBuf> = vec![driver_source_folder(row, &by_source)];
        for extra in row.merged_from.split("; ") {
            let extra = extra.trim();
            if extra.is_empty() {
                continue;
            }
            let folder = PathBuf::from(extra.replace('/', "\\"));
            if folder.is_dir() {
                source_folders.push(folder);
            }
        }

        for source_dir in &source_folders {
            // Every INF that lives in this source folder belongs to the bundle.
            let infs: Vec<&str> = by_source
                .get(&relative_path_str(source_dir))
                .map(|v| v.iter().map(|d| d.inf_name.as_str()).collect())
                .unwrap_or_default();

            // Shared payload (non-INF files and non-package subfolders).
            let mut files = crate::pack_discover::collect_bundle_extra_files(source_dir);
            // Ensure every INF in the folder is included.
            files.extend(infs.iter().map(PathBuf::from));
            files.sort();
            files.dedup();

            for file in &files {
                let src = source_dir.join(file);
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

/// Resolve the representative source folder for a manifest row.
fn driver_source_folder(
    row: &ManifestRow,
    by_source: &HashMap<String, Vec<&PackDriver>>,
) -> PathBuf {
    let direct = PathBuf::from(row.source_folder.replace('/', "\\"));
    if direct.is_dir() {
        return direct;
    }
    // Fall back to any known source folder whose bundle matches this row.
    by_source
        .keys()
        .find(|k| k.ends_with(&row.source_folder))
        .map(|k| PathBuf::from(k.replace('/', "\\")))
        .unwrap_or(direct)
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

/// Zip each package folder into `<folder>.zip` and record the archive name.
///
/// Unless `keep_folders` is set, the folder is removed afterwards so the pack
/// contains only archives plus the manifests -- ready to upload as-is.
fn zip_packages(rows: &mut [ManifestRow], pack_root: &Path, opts: &PackOptions) -> Result<()> {
    let mut zipped = 0usize;
    let mut failed = 0usize;

    // Installer-only categories are zipped as a whole category.
    for row in rows.iter_mut() {
        let folder_name = if row.status == "installer-only" {
            row.category.clone()
        } else {
            row.normalized_folder.clone()
        };

        let folder_path = pack_root.join(&folder_name);
        if !folder_path.is_dir() {
            continue;
        }

        let archive_name = format!("{}.zip", folder_name);
        let archive_path = pack_root.join(&archive_name);

        match crate::zip::zip_directory(&folder_path, &archive_path, opts.verbose) {
            Ok(()) => {
                row.archive = archive_name;
                zipped += 1;
                if !opts.keep_folders {
                    if let Err(e) = fs::remove_dir_all(&folder_path) {
                        eprintln!("Warning: could not remove {}: {}", folder_path.display(), e);
                    }
                }
            }
            Err(e) => {
                eprintln!("✗ Failed to zip {}: {}", folder_name, e);
                failed += 1;
            }
        }
    }

    println!("\nArchives created: {} ({} failed)", zipped, failed);
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
            zip: false,
            keep_folders: false,
            verbose: false,
            dry_run: true,
        }
    }

    #[test]
    fn bundles_infs_in_the_same_folder() {
        // Intel graphics style: one folder, several INFs, shared payload.
        let mut a = driver(
            "03_Graphics",
            Path::new("src/Drivers"),
            "iigd_dch.inf",
            "Display",
            "31.0.101.5081",
            "hA",
        );
        a.catalog_file = Some("igdlh.cat".to_string());
        let mut b = driver(
            "03_Graphics",
            Path::new("src/Drivers"),
            "iigd_dch_d.inf",
            "Display",
            "31.0.101.5081",
            "hB",
        );
        b.catalog_file = Some("igdlh.cat".to_string());
        let mut c = driver(
            "03_Graphics",
            Path::new("src/Drivers"),
            "iigd_ext.inf",
            "Extension",
            "31.0.101.5081",
            "hC",
        );
        c.catalog_file = Some("extinf.cat".to_string());

        let rows = plan_layout(&[a, b, c], &opts());
        assert_eq!(rows.len(), 1, "INFs in one folder form one package");
        assert_eq!(rows[0].status, "package");
        assert!(rows[0].driver_name.contains("iigd_dch"));
        assert!(rows[0].driver_name.contains("iigd_ext"));
    }

    #[test]
    fn different_folders_stay_separate() {
        let a = driver(
            "Display",
            Path::new("src/a"),
            "iigd_dch.inf",
            "Display",
            "31.0",
            "h1",
        );
        let b = driver(
            "Net",
            Path::new("src/b"),
            "rt640x64.inf",
            "Net",
            "10.0",
            "h2",
        );

        let rows = plan_layout(&[a, b], &opts());
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn keep_duplicates_disables_merging() {
        // Same driver in two folders; merging would collapse them.
        let a = driver(
            "Bluetooth",
            Path::new("src/CCP"),
            "ibtusb.inf",
            "Bluetooth",
            "1.0",
            "h1",
        );
        let b = driver(
            "Bluetooth",
            Path::new("src/GAP"),
            "ibtusb.inf",
            "Bluetooth",
            "1.0",
            "h2",
        );

        // With merging on they collapse to one package.
        assert_eq!(plan_layout(&[a.clone(), b.clone()], &opts()).len(), 1);

        // With merging off each folder stays its own package.
        let mut o = opts();
        o.dedup = false;
        let rows = plan_layout(&[a, b], &o);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn merges_same_driver_across_folders() {
        // Intel Bluetooth style: one INF per folder, same driver.
        let ds = vec![
            driver(
                "Bluetooth",
                Path::new("src/CCP"),
                "ibtusb.inf",
                "Bluetooth",
                "23.60.0.1",
                "hA",
            ),
            driver(
                "Bluetooth",
                Path::new("src/GAP"),
                "ibtusb.inf",
                "Bluetooth",
                "23.60.0.1",
                "hB",
            ),
            driver(
                "Bluetooth",
                Path::new("src/GFP"),
                "ibtusb.inf",
                "Bluetooth",
                "23.60.0.1",
                "hC",
            ),
        ];
        let rows = plan_layout(&ds, &opts());
        assert_eq!(rows.len(), 1, "same driver across folders merges");
        assert_eq!(rows[0].status, "package");
        assert!(rows[0].merged_from.contains("GAP"));
        assert!(rows[0].merged_from.contains("GFP"));
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
    fn bundled_package_unions_hardware_ids() {
        let mut a = driver("Net", Path::new("src/a"), "rt.inf", "Net", "1.0", "h1");
        a.hardware_ids = vec!["PCI\\VEN_10EC&DEV_8168".to_string()];
        let mut b = driver("Net", Path::new("src/a"), "rt2.inf", "Net", "1.0", "h2");
        b.hardware_ids = vec!["PCI\\VEN_10EC&DEV_8125".to_string()];

        let rows = plan_layout(&[a, b], &opts());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].hardware_id_count, 2);
        assert!(rows[0].hardware_ids.contains("DEV_8168"));
        assert!(rows[0].hardware_ids.contains("DEV_8125"));
    }

    #[test]
    fn compare_versions_orders_numerically() {
        use std::cmp::Ordering;
        assert_eq!(
            compare_versions("31.0.101.2125", "30.0.101.1371"),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("30.0.101.1371", "31.0.101.2125"),
            Ordering::Less
        );
        assert_eq!(compare_versions("1.2.3", "1.2.3"), Ordering::Equal);
        assert_eq!(compare_versions("1.10", "1.9"), Ordering::Greater);
    }

    #[test]
    fn marks_older_version_as_superseded() {
        let mut old = driver(
            "MEDIA",
            Path::new("src/a"),
            "mshdadac.inf",
            "MEDIA",
            "30.0.101.1371",
            "hA",
        );
        old.hardware_ids = vec!["HDAUDIO\\FUNC_01&VEN_8086&DEV_281B".to_string()];
        let mut new = driver(
            "MEDIA",
            Path::new("src/b"),
            "mshdadac.inf",
            "MEDIA",
            "31.0.101.2125",
            "hB",
        );
        new.hardware_ids = vec!["HDAUDIO\\FUNC_01&VEN_8086&DEV_281B".to_string()];

        let mut rows = plan_layout(&[old, new], &opts());
        assert_eq!(rows.len(), 2);
        let count = mark_superseded(&mut rows);
        assert_eq!(count, 1);

        let superseded: Vec<_> = rows.iter().filter(|r| r.status == "superseded").collect();
        let current: Vec<_> = rows.iter().filter(|r| r.status != "superseded").collect();
        assert_eq!(superseded.len(), 1);
        assert_eq!(superseded[0].driver_version, "30.0.101.1371");
        assert!(!superseded[0].superseded_by.is_empty());
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].driver_version, "31.0.101.2125");
    }

    #[test]
    fn does_not_mark_different_hardware() {
        let mut a = driver(
            "MEDIA",
            Path::new("src/a"),
            "mshdadac.inf",
            "MEDIA",
            "30.0.0.1",
            "hA",
        );
        a.hardware_ids = vec!["HDAUDIO\\DEV_AAAA".to_string()];
        let mut b = driver(
            "MEDIA",
            Path::new("src/b"),
            "mshdadac.inf",
            "MEDIA",
            "31.0.0.1",
            "hB",
        );
        b.hardware_ids = vec!["HDAUDIO\\DEV_BBBB".to_string()];

        let mut rows = plan_layout(&[a, b], &opts());
        assert_eq!(mark_superseded(&mut rows), 0);
        assert!(rows.iter().all(|r| r.status != "superseded"));
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
