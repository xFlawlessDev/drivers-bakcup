//! The `inspect` and `scan` commands.

use crate::csv_export::{export_inspect_csv, export_scan_csv};
use crate::inf_extract::extract_or_use_path;
use crate::inf_parser::{find_inf_files, find_inf_files_in_folder, parse_inf_file};
use crate::models::ParsedInfFile;
use anyhow::{bail, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Entry point for the `inspect` command.
pub fn run_inspect(path: &Path, output: Option<&Path>, verbose: bool) -> Result<()> {
    println!("Inspecting driver package: {}", path.display());

    let (work_dir, needs_cleanup) = extract_or_use_path(path, verbose)?;

    let inf_files = find_inf_files(&work_dir)?;
    if inf_files.is_empty() {
        if needs_cleanup {
            let _ = fs::remove_dir_all(&work_dir);
        }
        bail!("No INF files found in the specified path");
    }

    if verbose {
        println!("Found {} INF files", inf_files.len());
    }

    let parsed_files = parse_all(&inf_files, verbose);
    display_results(&parsed_files, verbose);

    if let Some(csv_path) = output {
        export_inspect_csv(&parsed_files, csv_path)?;
    }

    if needs_cleanup {
        if verbose {
            println!("Cleaning up temporary files...");
        }
        let _ = fs::remove_dir_all(&work_dir);
    }

    Ok(())
}

/// Entry point for the `scan` command.
pub fn run_scan(
    path: &Path,
    output: Option<&Path>,
    verbose: bool,
    group_by_class: bool,
    recursive: bool,
) -> Result<()> {
    if !path.is_dir() {
        bail!("Path must be a directory: {}", path.display());
    }

    println!("Scanning folder: {}", path.display());
    if recursive {
        println!("Mode: Recursive (including subfolders)");
    }
    println!();

    let inf_files = if recursive {
        find_inf_files(path)?
    } else {
        find_inf_files_in_folder(path)?
    };

    if inf_files.is_empty() {
        println!("No INF files found.");
        return Ok(());
    }

    let mut parsed_files: Vec<ParsedInfFile> = Vec::new();
    let mut parse_errors: Vec<(PathBuf, String)> = Vec::new();
    for inf_path in &inf_files {
        match parse_inf_file(inf_path) {
            Ok(parsed) => parsed_files.push(parsed),
            Err(e) => parse_errors.push((inf_path.clone(), e.to_string())),
        }
    }

    print_scan_header(path, inf_files.len(), &parsed_files, parse_errors.len());

    if group_by_class {
        display_scan_grouped(&parsed_files, verbose);
    } else {
        display_scan_list(&parsed_files, verbose);
    }

    if verbose && !parse_errors.is_empty() {
        println!("\n----------------------------------------");
        println!("Parse Errors:");
        for (path, error) in &parse_errors {
            println!(
                "  - {}: {}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                error
            );
        }
    }

    if let Some(csv_path) = output {
        export_scan_csv(&parsed_files, csv_path)?;
    }

    Ok(())
}

fn parse_all(inf_files: &[PathBuf], verbose: bool) -> Vec<ParsedInfFile> {
    let mut parsed_files = Vec::new();
    for inf_path in inf_files {
        match parse_inf_file(inf_path) {
            Ok(parsed) => parsed_files.push(parsed),
            Err(e) => {
                if verbose {
                    eprintln!("Warning: Failed to parse {}: {}", inf_path.display(), e);
                }
            }
        }
    }
    parsed_files
}

fn print_scan_header(path: &Path, total: usize, parsed: &[ParsedInfFile], failed: usize) {
    println!("========================================");
    println!("         INF Folder Scan Results");
    println!("========================================");
    println!();
    println!("Folder: {}", path.display());
    println!("Total INF files found: {}", total);
    println!("Successfully parsed: {}", parsed.len());
    if failed > 0 {
        println!("Failed to parse: {}", failed);
    }
    let total_devices: usize = parsed.iter().map(|f| f.drivers.len()).sum();
    println!("Total device entries: {}", total_devices);
    println!();
}

fn display_results(parsed_files: &[ParsedInfFile], verbose: bool) {
    println!("\n========================================");
    println!("       Driver Package Inspection");
    println!("========================================\n");

    let total_drivers: usize = parsed_files.iter().map(|f| f.drivers.len()).sum();
    println!(
        "Found {} INF files with {} device entries\n",
        parsed_files.len(),
        total_drivers
    );

    for parsed in parsed_files {
        println!("----------------------------------------");
        println!("INF File: {}", parsed.file_name);
        println!("Path: {}", parsed.file_path.display());

        let v = &parsed.raw_version_info;
        if let Some(ref class) = v.class {
            println!("Device Class: {}", class);
        }
        if let Some(ref guid) = v.class_guid {
            println!("Class GUID: {}", guid);
        }
        if let Some(ref version) = v.driver_version {
            println!("Driver Version: {}", version);
        }
        if let Some(ref date) = v.driver_date {
            println!("Driver Date: {}", date);
        }
        if let Some(ref provider) = v.provider {
            println!("Provider: {}", provider);
        }
        if let Some(ref catalog) = v.catalog_file {
            println!("Catalog File: {}", catalog);
        }

        if parsed.drivers.is_empty() {
            println!("\nNo device entries found in this INF file.");
        } else {
            println!("\nSupported Devices ({}):", parsed.drivers.len());
            for (idx, driver) in parsed.drivers.iter().enumerate() {
                println!(
                    "\n  {}. {}",
                    idx + 1,
                    driver.device_name.as_deref().unwrap_or("Unknown")
                );
                println!(
                    "     Hardware ID: {}",
                    driver.hardware_id.as_deref().unwrap_or("Unknown")
                );
                if verbose {
                    if let Some(ref mfg) = driver.manufacturer {
                        println!("     Manufacturer: {}", mfg);
                    }
                    if let Some(ref desc) = driver.description {
                        if desc != driver.device_name.as_deref().unwrap_or("") {
                            println!("     Description: {}", desc);
                        }
                    }
                }
            }
        }
        println!();
    }
}

fn display_scan_list(parsed_files: &[ParsedInfFile], verbose: bool) {
    println!("----------------------------------------");
    println!("INF Files Summary:");
    println!("----------------------------------------");

    for (idx, parsed) in parsed_files.iter().enumerate() {
        println!("\n{}. {}", idx + 1, parsed.file_name);
        let v = &parsed.raw_version_info;
        if let Some(ref class) = v.class {
            println!("   Class: {}", class);
        }
        if let Some(ref version) = v.driver_version {
            println!("   Version: {}", version);
        }
        if let Some(ref date) = v.driver_date {
            println!("   Date: {}", date);
        }
        if let Some(ref provider) = v.provider {
            let display = if provider.starts_with('%') && provider.ends_with('%') {
                parsed
                    .drivers
                    .first()
                    .and_then(|d| d.driver_provider_name.as_ref())
                    .map(|s| s.as_str())
                    .unwrap_or(provider)
            } else {
                provider
            };
            println!("   Provider: {}", display);
        }
        println!("   Devices: {} entries", parsed.drivers.len());

        if verbose && !parsed.drivers.is_empty() {
            println!("   Hardware IDs:");
            for driver in &parsed.drivers {
                if let Some(ref hwid) = driver.hardware_id {
                    println!(
                        "     - {} ({})",
                        hwid,
                        driver.device_name.as_deref().unwrap_or("Unknown")
                    );
                }
            }
        }
    }
}

fn display_scan_grouped(parsed_files: &[ParsedInfFile], verbose: bool) {
    let mut by_class: HashMap<String, Vec<&ParsedInfFile>> = HashMap::new();
    for parsed in parsed_files {
        let class = parsed
            .raw_version_info
            .class
            .as_deref()
            .unwrap_or("Unknown")
            .to_string();
        by_class.entry(class).or_default().push(parsed);
    }

    let mut classes: Vec<_> = by_class.keys().cloned().collect();
    classes.sort();

    println!("----------------------------------------");
    println!("INF Files by Device Class:");
    println!("----------------------------------------");

    for class in classes {
        if let Some(files) = by_class.get(&class) {
            println!("\n[{}] ({} INF files)", class, files.len());
            for parsed in files {
                let version = parsed
                    .raw_version_info
                    .driver_version
                    .as_deref()
                    .unwrap_or("?");
                println!(
                    "  - {} (v{}, {} devices)",
                    parsed.file_name,
                    version,
                    parsed.drivers.len()
                );
                if verbose {
                    for driver in &parsed.drivers {
                        if let Some(ref hwid) = driver.hardware_id {
                            println!("      HWID: {}", hwid);
                        }
                    }
                }
            }
        }
    }
}
