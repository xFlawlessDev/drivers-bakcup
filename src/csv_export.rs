//! CSV writers for the various commands.

use crate::models::{DeviceClassLegendEntry, ParsedInfFile, PnPSignedDriver};
use crate::util::{escape_csv, format_driver_date};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

/// Write a CSV file, creating parent directories as needed.
fn write_csv(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    std::fs::write(path, content)
        .with_context(|| format!("Failed to write CSV file: {}", path.display()))
}

/// Resolve an INF `[Version]` provider reference to a display name.
fn resolve_provider(parsed: &ParsedInfFile) -> &str {
    let provider = parsed
        .raw_version_info
        .provider
        .as_deref()
        .unwrap_or("Unknown");
    if provider.starts_with('%') && provider.ends_with('%') {
        parsed
            .drivers
            .first()
            .and_then(|d| d.driver_provider_name.as_deref())
            .unwrap_or(provider)
    } else {
        provider
    }
}

/// Write the driver-class legend CSV.
pub fn write_device_class_legend(
    legend: &[DeviceClassLegendEntry],
    output_path: &Path,
) -> Result<()> {
    let mut csv_content =
        String::from("Device Class,Class GUID,Present Devices,Installed Packages\n");
    for (class, guid, device_count, package_count) in legend {
        let guid_field = if guid.is_empty() {
            "Unknown"
        } else {
            guid.as_str()
        };
        csv_content.push_str(&format!(
            "{},{},{},{}\n",
            escape_csv(class),
            escape_csv(guid_field),
            device_count,
            package_count
        ));
    }
    write_csv(output_path, &csv_content)
}

/// Export WMI driver info grouped by driver version (collection).
pub fn export_wmi_drivers_csv(
    drivers: &[PnPSignedDriver],
    output_path: &Path,
    verbose: bool,
) -> Result<()> {
    let inf_lookup = crate::driver_store::build_inf_lookup();

    let mut grouped: HashMap<String, Vec<&PnPSignedDriver>> = HashMap::new();
    for driver in drivers {
        let version = driver
            .driver_version
            .as_deref()
            .unwrap_or("Unknown")
            .to_string();
        grouped.entry(version).or_default().push(driver);
    }

    let mut csv_content = String::from(
        "Collection,Device Class,Provider,Driver Version,Driver Date,Device Count,Actual INFs,Device Names,Hardware IDs\n",
    );

    let mut sorted_keys: Vec<_> = grouped.keys().cloned().collect();
    sorted_keys.sort();

    for version in &sorted_keys {
        let drivers_for_version = match grouped.get(version) {
            Some(d) if !d.is_empty() => d,
            _ => continue,
        };
        let first = drivers_for_version[0];

        let driver_date = format_driver_date(first.driver_date.as_deref());

        let mut actual_infs: Vec<String> = drivers_for_version
            .iter()
            .filter_map(|d| {
                let oem = d.inf_name.as_deref()?.to_lowercase();
                Some(inf_lookup.get(&oem).cloned().unwrap_or(oem))
            })
            .collect();
        actual_infs.sort();
        actual_infs.dedup();

        let device_names: Vec<String> = drivers_for_version
            .iter()
            .filter_map(|d| d.device_name.clone())
            .collect();
        let hardware_ids: Vec<String> = drivers_for_version
            .iter()
            .filter_map(|d| d.hardware_id.clone())
            .collect();

        let provider = first.driver_provider_name.as_deref().unwrap_or("Unknown");
        let collection_name = format!("{} {} Package", provider, version);

        csv_content.push_str(&format!(
            "{},{},{},{},{},{},{},{},{}\n",
            escape_csv(&collection_name),
            escape_csv(first.device_class.as_deref().unwrap_or("Unknown")),
            escape_csv(provider),
            escape_csv(version),
            escape_csv(&driver_date),
            drivers_for_version.len(),
            escape_csv(&actual_infs.join("; ")),
            escape_csv(&device_names.join("; ")),
            escape_csv(&hardware_ids.join("; ")),
        ));
    }

    write_csv(output_path, &csv_content)?;

    println!("CSV created: {}", output_path.display());
    println!("Total collections: {}", grouped.len());
    println!("Total devices: {}", drivers.len());

    if verbose {
        println!("\nDriver collections exported:");
        for version in &sorted_keys {
            let drivers_for_version = match grouped.get(version) {
                Some(d) if !d.is_empty() => d,
                _ => continue,
            };
            let first = drivers_for_version[0];
            let provider = first.driver_provider_name.as_deref().unwrap_or("Unknown");
            println!(
                "\n  {} {} - {} devices",
                provider,
                version,
                drivers_for_version.len()
            );
            for driver in drivers_for_version {
                let oem = driver
                    .inf_name
                    .as_deref()
                    .unwrap_or("unknown")
                    .to_lowercase();
                let actual = inf_lookup.get(&oem).map(|s| s.as_str()).unwrap_or(&oem);
                println!(
                    "    - {} | {} | {}",
                    driver.device_name.as_deref().unwrap_or("Unknown"),
                    driver.hardware_id.as_deref().unwrap_or("Unknown"),
                    actual
                );
            }
        }
    }

    Ok(())
}

/// Export parsed INF files (from `inspect`) to a detailed CSV.
pub fn export_inspect_csv(parsed_files: &[ParsedInfFile], output_path: &Path) -> Result<()> {
    let mut csv_content = String::from(
        "Device Name,Driver Version,Driver Date,Hardware ID,INF Name,Description,Provider,Device Class,Class GUID,Catalog File,Manufacturer\n",
    );

    for parsed in parsed_files {
        for driver in &parsed.drivers {
            csv_content.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{}\n",
                escape_csv(driver.device_name.as_deref().unwrap_or("Unknown")),
                escape_csv(driver.driver_version.as_deref().unwrap_or("Unknown")),
                escape_csv(driver.driver_date.as_deref().unwrap_or("Unknown")),
                escape_csv(driver.hardware_id.as_deref().unwrap_or("Unknown")),
                escape_csv(driver.inf_name.as_deref().unwrap_or("Unknown")),
                escape_csv(driver.description.as_deref().unwrap_or("Unknown")),
                escape_csv(driver.driver_provider_name.as_deref().unwrap_or("Unknown")),
                escape_csv(driver.device_class.as_deref().unwrap_or("Unknown")),
                escape_csv(driver.class_guid.as_deref().unwrap_or("Unknown")),
                escape_csv(driver.catalog_file.as_deref().unwrap_or("Unknown")),
                escape_csv(driver.manufacturer.as_deref().unwrap_or("Unknown")),
            ));
        }
    }

    write_csv(output_path, &csv_content)?;
    println!("Exported to: {}", output_path.display());
    Ok(())
}

/// Export `scan` results (one row per INF file) to CSV.
pub fn export_scan_csv(parsed_files: &[ParsedInfFile], output_path: &Path) -> Result<()> {
    let mut csv_content = String::from(
        "INF File,Device Class,Provider,Driver Version,Driver Date,Device Count,Device Names,Hardware IDs\n",
    );
    for parsed in parsed_files {
        let device_names = join_opt(parsed.drivers.iter().map(|d| d.device_name.clone()));
        let hwids = join_opt(parsed.drivers.iter().map(|d| d.hardware_id.clone()));
        csv_content.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            escape_csv(&parsed.file_name),
            escape_csv(
                parsed
                    .raw_version_info
                    .class
                    .as_deref()
                    .unwrap_or("Unknown")
            ),
            escape_csv(resolve_provider(parsed)),
            escape_csv(
                parsed
                    .raw_version_info
                    .driver_version
                    .as_deref()
                    .unwrap_or("Unknown")
            ),
            escape_csv(
                parsed
                    .raw_version_info
                    .driver_date
                    .as_deref()
                    .unwrap_or("Unknown")
            ),
            parsed.drivers.len(),
            escape_csv(&device_names),
            escape_csv(&hwids),
        ));
    }

    write_csv(output_path, &csv_content)?;
    println!("\nExported to: {}", output_path.display());
    Ok(())
}

/// Export the backup summary (one row per INF file, including folder path).
pub fn export_backup_summary_csv(
    parsed_files: &[ParsedInfFile],
    backup_dir: &Path,
    output_path: &Path,
) -> Result<()> {
    let mut csv_content = String::from(
        "INF File,Device Class,Provider,Driver Version,Driver Date,Device Count,Folder Name,Device Names,Hardware IDs\n",
    );
    for parsed in parsed_files {
        let device_names = join_opt(parsed.drivers.iter().map(|d| d.device_name.clone()));
        let hwids = join_opt(parsed.drivers.iter().map(|d| d.hardware_id.clone()));
        let folder_name = parsed
            .file_path
            .parent()
            .and_then(|p| p.strip_prefix(backup_dir).ok())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown".to_string());

        csv_content.push_str(&format!(
            "{},{},{},{},{},{},{},{},{}\n",
            escape_csv(&parsed.file_name),
            escape_csv(
                parsed
                    .raw_version_info
                    .class
                    .as_deref()
                    .unwrap_or("Unknown")
            ),
            escape_csv(resolve_provider(parsed)),
            escape_csv(
                parsed
                    .raw_version_info
                    .driver_version
                    .as_deref()
                    .unwrap_or("Unknown")
            ),
            escape_csv(
                parsed
                    .raw_version_info
                    .driver_date
                    .as_deref()
                    .unwrap_or("Unknown")
            ),
            parsed.drivers.len(),
            escape_csv(&folder_name),
            escape_csv(&device_names),
            escape_csv(&hwids),
        ));
    }

    write_csv(output_path, &csv_content)
}

fn join_opt(values: impl Iterator<Item = Option<String>>) -> String {
    values.flatten().collect::<Vec<_>>().join("; ")
}
