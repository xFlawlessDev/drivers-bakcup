//! The `backup` (and `export --files`) driver export logic.

use crate::csv_export::{export_backup_summary_csv, write_device_class_legend};
use crate::driver_store::{get_driver_store_packages, select_unbound_non_microsoft};
use crate::models::{DeviceClassLegendEntry, DriverStorePackage, PnPSignedDriver};
use crate::pnputil::{export_driver, ExportOutcome};
use crate::util::{
    format_driver_date, is_microsoft_provider, is_unsafe_path, sanitize_class_folder_name,
    sanitize_folder_name, strip_inf_extension,
};
use crate::wmi_query::WmiQuery;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// High-level entry point for the `backup` command.
pub fn run_backup(output: &Path, verbose: bool, dry_run: bool) -> Result<()> {
    validate_admin_privileges()?;
    validate_output_directory(output)?;

    let wmi = WmiQuery::connect()?;
    let all_drivers = wmi.signed_drivers()?;
    let non_ms: Vec<PnPSignedDriver> = all_drivers
        .into_iter()
        .filter(|d| !is_microsoft_provider(d.driver_provider_name.as_deref()))
        .collect();

    if non_ms.is_empty() {
        println!(
            "No present non-Microsoft devices found; checking the driver store for unbound packages..."
        );
    }

    let base_dir = create_base_backup_directory(output)?;
    let result = export_drivers(&base_dir, non_ms, dry_run, verbose)?;

    write_legend_if_needed(&base_dir, &result, dry_run)?;
    print_summary(&result);
    write_summary_csv_if_needed(&base_dir, dry_run, verbose)?;

    Ok(())
}

/// Result of an export pass.
struct ExportResult {
    backed_up: usize,
    failed: usize,
    unbound_backed_up: usize,
    device_info: Vec<PnPSignedDriver>,
    store_packages: Vec<DriverStorePackage>,
}

/// Check that the process can write to a location that requires elevation.
fn validate_admin_privileges() -> Result<()> {
    let test_path = Path::new("C:\\Windows\\Temp\\driver_backup_admin_test");
    match fs::write(test_path, "test") {
        Ok(_) => {
            let _ = fs::remove_file(test_path);
            Ok(())
        }
        Err(_) => bail!(
            "This program requires administrative privileges to access driver information. \
             Please run as Administrator."
        ),
    }
}

/// Validate that the output directory exists or can be created, and is writable.
fn validate_output_directory(output: &Path) -> Result<()> {
    if output.exists() && !output.is_dir() {
        bail!(
            "Output path exists but is not a directory: {}",
            output.display()
        );
    }

    if !output.exists() {
        fs::create_dir_all(output)
            .with_context(|| format!("Failed to create output directory: {}", output.display()))?;
    }

    let test_file = output.join("write_test.tmp");
    fs::write(&test_file, "test")
        .with_context(|| format!("Cannot write to output directory: {}", output.display()))?;
    fs::remove_file(&test_file).ok();

    Ok(())
}

fn create_base_backup_directory(output: &Path) -> Result<PathBuf> {
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let backup_dir = output.join(format!("drivers_{}", timestamp));
    fs::create_dir_all(&backup_dir).with_context(|| {
        format!(
            "Failed to create backup directory: {}",
            backup_dir.display()
        )
    })?;
    Ok(backup_dir)
}

/// Extract the OEM INF name (`oemN.inf`) from a driver's reported INF name.
fn extract_oem_inf_name(inf_name: &str) -> Option<String> {
    let inf_lower = inf_name.to_lowercase();
    if inf_lower.starts_with("oem")
        && inf_lower.ends_with(".inf")
        && inf_lower
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
    {
        Some(inf_lower)
    } else {
        None
    }
}

/// Run both export passes and return the results.
fn export_drivers(
    base_dir: &Path,
    drivers: Vec<PnPSignedDriver>,
    dry_run: bool,
    verbose: bool,
) -> Result<ExportResult> {
    // Group present drivers by device class, then by OEM INF.
    let mut bound_oem_infs: HashSet<String> = HashSet::new();
    let mut by_class: HashMap<String, HashMap<String, Vec<PnPSignedDriver>>> = HashMap::new();

    for driver in drivers {
        let inf_name = match &driver.inf_name {
            Some(name) => name.clone(),
            None => continue,
        };
        let oem_inf = match extract_oem_inf_name(&inf_name) {
            Some(oem) => oem,
            None => {
                if verbose {
                    println!("Skipping non-OEM INF: {}", inf_name);
                }
                continue;
            }
        };

        bound_oem_infs.insert(oem_inf.clone());
        let device_class = driver
            .device_class
            .as_deref()
            .unwrap_or("Unknown_Class")
            .to_string();
        by_class
            .entry(device_class)
            .or_default()
            .entry(oem_inf)
            .or_default()
            .push(driver);
    }

    let mut backed_up = 0;
    let mut failed = 0;
    let mut device_info = Vec::new();

    // Pass 1: packages bound to a present device.
    let mut class_keys: Vec<_> = by_class.keys().cloned().collect();
    class_keys.sort();
    for device_class in class_keys {
        let infs_in_class = &by_class[&device_class];
        let class_dir = base_dir.join(sanitize_class_folder_name(&device_class));

        if verbose {
            println!("Processing Device Class: {}", device_class);
            println!(
                "  Class Folder: {}",
                class_dir.file_name().unwrap_or_default().to_string_lossy()
            );
            println!(
                "  Number of driver packages in this class: {}",
                infs_in_class.len()
            );
            println!();
        }

        if !dry_run {
            fs::create_dir_all(&class_dir).with_context(|| {
                format!("Failed to create class directory: {}", class_dir.display())
            })?;
        }

        let mut inf_keys: Vec<_> = infs_in_class.keys().cloned().collect();
        inf_keys.sort();

        for oem_inf in inf_keys {
            let drivers_for_package = &infs_in_class[&oem_inf];
            let (folder_name, driver_dir) = package_folder(&class_dir, drivers_for_package);
            print_package_details(verbose, &folder_name, &oem_inf, drivers_for_package);

            if dry_run {
                backed_up += 1;
                device_info.extend(drivers_for_package.clone());
                continue;
            }

            if is_unsafe_path(&driver_dir.to_string_lossy()) {
                eprintln!(
                    "Skipping export due to unsafe path: {}",
                    driver_dir.display()
                );
                failed += 1;
                continue;
            }

            fs::create_dir_all(&driver_dir).with_context(|| {
                format!(
                    "Failed to create driver directory: {}",
                    driver_dir.display()
                )
            })?;

            if verbose {
                println!("      Created folder: {}", driver_dir.display());
                println!(
                    "        Exporting {} to {}...",
                    oem_inf,
                    driver_dir.display()
                );
            }

            match export_driver(&oem_inf, &driver_dir) {
                ExportOutcome::Success => {
                    backed_up += 1;
                    device_info.extend(drivers_for_package.clone());
                    if verbose {
                        println!("        ✓ Successfully exported: {}", oem_inf);
                    }
                }
                ExportOutcome::Failed(reason) => {
                    eprintln!("✗ Failed to export {}: {}", oem_inf, reason);
                    failed += 1;
                }
            }
        }
    }

    // Pass 2: installed packages not bound to a present device.
    let store_packages = get_driver_store_packages();
    let unbound_backed_up = export_unbound(
        base_dir,
        &bound_oem_infs,
        &store_packages,
        dry_run,
        verbose,
        &mut failed,
    )?;
    backed_up += unbound_backed_up;

    Ok(ExportResult {
        backed_up,
        failed,
        unbound_backed_up,
        device_info,
        store_packages,
    })
}

/// Compute the folder name and path for a driver package bound to devices.
fn package_folder(class_dir: &Path, drivers: &[PnPSignedDriver]) -> (String, PathBuf) {
    let primary_device_name = drivers
        .first()
        .and_then(|d| d.device_name.as_deref())
        .unwrap_or("Unknown_Device");
    let version = drivers
        .first()
        .and_then(|d| d.driver_version.as_deref())
        .unwrap_or("Unknown_Version");
    let folder_name = sanitize_folder_name(&format!("{}_{} Package", primary_device_name, version));
    let dir = class_dir.join(&folder_name);
    (folder_name, dir)
}

fn print_package_details(
    verbose: bool,
    folder_name: &str,
    oem_inf: &str,
    drivers: &[PnPSignedDriver],
) {
    if !verbose {
        return;
    }
    let primary = drivers.first();
    println!(
        "  Processing driver package: {} ({})",
        primary
            .and_then(|d| d.device_name.as_deref())
            .unwrap_or("Unknown"),
        oem_inf
    );
    println!("    Folder: {}", folder_name);
    println!("    Number of devices in this package: {}", drivers.len());
    println!();
    for (index, driver) in drivers.iter().enumerate() {
        println!(
            "      {}. Device: {}",
            index + 1,
            driver.device_name.as_deref().unwrap_or("Unknown")
        );
        println!(
            "         INF: {}",
            driver.inf_name.as_deref().unwrap_or("Unknown")
        );
        println!(
            "         Hardware ID: {}",
            driver.hardware_id.as_deref().unwrap_or("Unknown")
        );
        println!(
            "         Device ID: {}",
            driver.device_id.as_deref().unwrap_or("Unknown")
        );
        println!(
            "         Description: {}",
            driver.description.as_deref().unwrap_or("Unknown")
        );
        println!(
            "         Provider: {}",
            driver.driver_provider_name.as_deref().unwrap_or("Unknown")
        );
        println!(
            "         Version: {}",
            driver.driver_version.as_deref().unwrap_or("Unknown")
        );
        println!(
            "         Date: {}",
            format_driver_date(driver.driver_date.as_deref())
        );
        println!();
    }
}

/// Export packages installed but not bound to a present device.
fn export_unbound(
    base_dir: &Path,
    bound_oem_infs: &HashSet<String>,
    packages: &[DriverStorePackage],
    dry_run: bool,
    verbose: bool,
    failed: &mut usize,
) -> Result<usize> {
    if packages.is_empty() {
        if verbose {
            println!("No driver store packages reported by pnputil.");
        }
        return Ok(0);
    }

    let unbound = select_unbound_non_microsoft(packages, bound_oem_infs);
    if verbose {
        println!(
            "Driver store check: {} package(s) installed, {} not bound to a present device.",
            packages.len(),
            unbound.len()
        );
    }

    let mut backed_up = 0;
    for pkg in unbound {
        let class_dir = base_dir.join(sanitize_class_folder_name(
            pkg.class_name.as_deref().unwrap_or("Unknown_Class"),
        ));
        let original = pkg.original_name.as_deref().unwrap_or(&pkg.published_name);
        let stem = strip_inf_extension(original);
        let version = pkg.driver_version.as_deref().unwrap_or("Unknown_Version");
        let folder_name = sanitize_folder_name(&format!("{}_{} Package", stem, version));
        let driver_dir = class_dir.join(&folder_name);

        if verbose {
            println!(
                "  Found unbound package: {} ({}) [{}] provider: {}",
                pkg.published_name,
                original,
                pkg.class_name.as_deref().unwrap_or("Unknown"),
                pkg.provider_name.as_deref().unwrap_or("Unknown")
            );
            println!("    Folder: {}", driver_dir.display());
        }

        if dry_run {
            backed_up += 1;
            continue;
        }

        if is_unsafe_path(&driver_dir.to_string_lossy()) {
            eprintln!(
                "Skipping export due to unsafe path: {}",
                driver_dir.display()
            );
            *failed += 1;
            continue;
        }

        fs::create_dir_all(&driver_dir).with_context(|| {
            format!(
                "Failed to create driver directory: {}",
                driver_dir.display()
            )
        })?;

        match export_driver(&pkg.published_name, &driver_dir) {
            ExportOutcome::Success => {
                backed_up += 1;
                if verbose {
                    println!(
                        "        ✓ Successfully exported unbound: {}",
                        pkg.published_name
                    );
                }
            }
            ExportOutcome::Failed(reason) => {
                eprintln!(
                    "✗ Failed to export unbound {}: {}",
                    pkg.published_name, reason
                );
                *failed += 1;
            }
        }
    }

    Ok(backed_up)
}

/// Build the Device Class -> Class GUID legend from present devices and the
/// driver store.
pub fn build_device_class_legend(
    drivers: &[PnPSignedDriver],
    store_packages: &[DriverStorePackage],
) -> Vec<DeviceClassLegendEntry> {
    // Key: lowercased class name -> (display name, guid, devices, packages)
    let mut by_class: HashMap<String, DeviceClassLegendEntry> = HashMap::new();

    for driver in drivers {
        let class = match driver.device_class.as_deref() {
            Some(c) if !c.is_empty() => c,
            _ => continue,
        };
        let entry = by_class
            .entry(class.to_lowercase())
            .or_insert_with(|| (class.to_string(), String::new(), 0, 0));
        if entry.1.is_empty() {
            if let Some(ref guid) = driver.class_guid {
                entry.1 = guid.clone();
            }
        }
        entry.2 += 1;
    }

    for pkg in store_packages {
        let class = match pkg.class_name.as_deref() {
            Some(c) if !c.is_empty() => c,
            _ => continue,
        };
        let entry = by_class
            .entry(class.to_lowercase())
            .or_insert_with(|| (class.to_string(), String::new(), 0, 0));
        if entry.1.is_empty() {
            if let Some(ref guid) = pkg.class_guid {
                entry.1 = guid.clone();
            }
        }
        entry.3 += 1;
    }

    let mut legend: Vec<DeviceClassLegendEntry> = by_class.into_values().collect();
    legend.sort_by_key(|entry| entry.0.to_lowercase());
    legend
}

fn write_legend_if_needed(base_dir: &Path, result: &ExportResult, dry_run: bool) -> Result<()> {
    if dry_run {
        return Ok(());
    }
    let legend = build_device_class_legend(&result.device_info, &result.store_packages);
    let legend_path = base_dir.join("device_class_legend.csv");
    write_device_class_legend(&legend, &legend_path)?;
    println!("Device class legend written: {}", legend_path.display());
    Ok(())
}

fn print_summary(result: &ExportResult) {
    println!("\nDriver export completed!");
    println!(
        "Successfully exported: {} driver packages",
        result.backed_up
    );
    if result.unbound_backed_up > 0 {
        println!(
            "  ({} of these were installed but not bound to a present device)",
            result.unbound_backed_up
        );
    }
    if result.failed > 0 {
        println!("Failed to export: {} drivers", result.failed);
    }
}

fn write_summary_csv_if_needed(base_dir: &Path, dry_run: bool, verbose: bool) -> Result<()> {
    if dry_run {
        return Ok(());
    }

    println!("\nScanning exported drivers to create summary...");
    let parsed = crate::inf_parser::find_inf_files(base_dir)?
        .iter()
        .filter_map(|p| crate::inf_parser::parse_inf_file(p).ok())
        .collect::<Vec<_>>();

    if parsed.is_empty() {
        println!("No INF files found in backup folder.");
        return Ok(());
    }

    let csv_path = base_dir.join("all_drivers.csv");
    export_backup_summary_csv(&parsed, base_dir, &csv_path)?;

    println!("Summary CSV created: {}", csv_path.display());
    println!("Total INF files: {}", parsed.len());
    let total_devices: usize = parsed.iter().map(|f| f.drivers.len()).sum();
    println!("Total device entries: {}", total_devices);

    let _ = verbose;
    println!("\nBackup location: {}", base_dir.display());
    Ok(())
}

/// Export driver files for the `export --files` command.
///
/// Returns success/failure counts. Shared by `export` so both commands back up
/// the same set of packages.
pub fn export_files_for_inventory(
    drivers: &[PnPSignedDriver],
    backup_dir: &Path,
    include_all: bool,
    verbose: bool,
) -> Result<(usize, usize)> {
    let mut exported_infs: HashSet<String> = HashSet::new();
    let mut success = 0;
    let mut failed = 0;

    for driver in drivers {
        let inf_name = match &driver.inf_name {
            Some(name) => name.clone(),
            None => continue,
        };
        let inf_lower = inf_name.to_lowercase();
        if !inf_lower.starts_with("oem") || !exported_infs.insert(inf_lower.clone()) {
            continue;
        }

        let class = driver.device_class.as_deref().unwrap_or("Unknown");
        let version = driver.driver_version.as_deref().unwrap_or("Unknown");
        let provider = driver.driver_provider_name.as_deref().unwrap_or("Unknown");
        let folder_name = sanitize_folder_name(&format!("{}_{}_{}", class, provider, version));
        let driver_dir = backup_dir.join(&folder_name);
        fs::create_dir_all(&driver_dir).ok();

        if verbose {
            println!("  Exporting {} -> {}", inf_name, folder_name);
        }

        match export_driver(&inf_name, &driver_dir) {
            ExportOutcome::Success => success += 1,
            ExportOutcome::Failed(reason) => {
                failed += 1;
                if verbose {
                    eprintln!("    Failed to export {}: {}", inf_name, reason);
                }
            }
        }
    }

    // Installed packages not bound to a present device.
    let store_packages = get_driver_store_packages();
    let unbound: Vec<_> = store_packages
        .iter()
        .filter(|pkg| !exported_infs.contains(&pkg.published_name))
        .filter(|pkg| include_all || !is_microsoft_provider(pkg.provider_name.as_deref()))
        .collect();

    if !unbound.is_empty() {
        println!(
            "Found {} installed package(s) not bound to a present device.",
            unbound.len()
        );
    }

    for pkg in unbound {
        let class = pkg.class_name.as_deref().unwrap_or("Unknown");
        let version = pkg.driver_version.as_deref().unwrap_or("Unknown");
        let provider = pkg.provider_name.as_deref().unwrap_or("Unknown");
        let original = pkg.original_name.as_deref().unwrap_or(&pkg.published_name);
        let folder_name = sanitize_folder_name(&format!("{}_{}_{}", class, provider, version));
        let driver_dir = backup_dir.join(&folder_name);
        fs::create_dir_all(&driver_dir).ok();

        if verbose {
            println!(
                "  Exporting unbound {} ({}) -> {}",
                pkg.published_name, original, folder_name
            );
        }

        match export_driver(&pkg.published_name, &driver_dir) {
            ExportOutcome::Success => success += 1,
            ExportOutcome::Failed(reason) => {
                failed += 1;
                if verbose {
                    eprintln!("    Failed to export {}: {}", pkg.published_name, reason);
                }
            }
        }
    }

    Ok((success, failed))
}
