//! Driver Backup Tool - entry point.

mod backup;
mod cli;
mod csv_export;
mod driver_store;
mod inf_extract;
mod inf_parser;
mod models;
mod pack;
mod pack_discover;
mod pack_meta;
mod pnputil;
mod scan;
mod util;
mod wmi_query;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use cli::{Args, Commands};
use std::fs;
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    let args = Args::parse();
    let command = args.command.unwrap_or(Commands::Backup {
        output: PathBuf::from("driver_backup"),
        verbose: false,
        dry_run: false,
    });

    run(command)?;

    // Add pause before closing
    println!("\nPress Enter to close...");
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .expect("Failed to read line");

    Ok(())
}

fn run(command: Commands) -> Result<()> {
    match command {
        Commands::Backup {
            output,
            verbose,
            dry_run,
        } => {
            print_backup_banner(&output, verbose, dry_run);
            backup::run_backup(&output, verbose, dry_run)?;
        }
        Commands::Inspect {
            path,
            output,
            verbose,
        } => {
            if verbose {
                println!("Driver Package Inspector");
                println!("========================");
                println!("Input path: {}", path.display());
                if let Some(ref out) = output {
                    println!("Output CSV: {}", out.display());
                }
                println!();
            }
            scan::run_inspect(&path, output.as_deref(), verbose)?;
        }
        Commands::Scan {
            path,
            output,
            verbose,
            group,
            recursive,
        } => {
            if verbose {
                println!("INF Folder Scanner");
                println!("==================");
                println!("Folder: {}", path.display());
                if let Some(ref out) = output {
                    println!("Output CSV: {}", out.display());
                }
                println!("Group by class: {}", group);
                println!("Recursive: {}", recursive);
                println!();
            }
            scan::run_scan(&path, output.as_deref(), verbose, group, recursive)?;
        }
        Commands::Export {
            output,
            all,
            verbose,
            files,
        } => {
            run_export(&output, all, verbose, files)?;
        }
        Commands::Pack {
            path,
            output,
            odm,
            model,
            revision,
            keep_duplicates,
            manifest_only,
            dry_run,
            verbose,
        } => {
            run_pack(
                &path,
                &output,
                odm,
                model,
                revision,
                keep_duplicates,
                manifest_only,
                dry_run,
                verbose,
            )?;
        }
    }
    Ok(())
}

/// The `pack` command: normalize a vendor driver pack.
#[allow(clippy::too_many_arguments)]
fn run_pack(
    path: &Path,
    output: &Path,
    odm: Option<String>,
    model: Option<String>,
    revision: Option<String>,
    keep_duplicates: bool,
    manifest_only: bool,
    dry_run: bool,
    verbose: bool,
) -> Result<()> {
    // Start from best-effort detection, then apply explicit overrides.
    let mut metadata = pack_meta::detect_metadata(path);
    if odm.is_some() {
        metadata.odm = odm;
    }
    if model.is_some() {
        metadata.model = model;
    }
    if revision.is_some() {
        metadata.revision = revision;
    }

    let opts = pack::PackOptions {
        source: path.to_path_buf(),
        output: output.to_path_buf(),
        metadata,
        dedup: !keep_duplicates,
        copy_files: !manifest_only,
        verbose,
        dry_run,
    };

    let result = pack::run_pack(&opts)?;

    if !dry_run && result.unique > 0 {
        println!(
            "\nDone. {} unique package(s), {} duplicate(s) collapsed.",
            result.unique, result.duplicates
        );
    }
    Ok(())
}

fn print_backup_banner(output: &Path, verbose: bool, dry_run: bool) {
    if !verbose {
        return;
    }
    println!("Driver Export Tool");
    println!("==================");
    println!("Output directory: {}", output.display());
    println!("Dry run: {}", dry_run);
    println!();
}

/// The `export` command: connected-device inventory, optionally with files.
fn run_export(output: &Path, all: bool, verbose: bool, files: bool) -> Result<()> {
    println!("Hardware Inventory Export");
    println!("=========================");

    let wmi = wmi_query::WmiQuery::connect()?;
    let drivers = wmi.signed_drivers()?;
    let filtered: Vec<_> = if all {
        drivers
    } else {
        drivers
            .into_iter()
            .filter(|d| !util::is_microsoft_provider(d.driver_provider_name.as_deref()))
            .collect()
    };

    println!("Found {} connected devices", filtered.len());

    if !files {
        csv_export::export_wmi_drivers_csv(&filtered, output, verbose)?;
        println!("\nExported to: {}", output.display());
        return Ok(());
    }

    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let backup_dir = if output.extension().map(|e| e == "csv").unwrap_or(false) {
        output
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!("drivers_{}", timestamp))
    } else {
        output.join(format!("drivers_{}", timestamp))
    };

    fs::create_dir_all(&backup_dir).with_context(|| {
        format!(
            "Failed to create backup directory: {}",
            backup_dir.display()
        )
    })?;

    println!("\nExporting driver files to: {}", backup_dir.display());

    let (success, failed) =
        backup::export_files_for_inventory(&filtered, &backup_dir, all, verbose)?;
    println!(
        "Driver files exported: {} success, {} failed",
        success, failed
    );

    let csv_path = backup_dir.join("all_drivers.csv");
    csv_export::export_wmi_drivers_csv(&filtered, &csv_path, verbose)?;

    let store_packages = driver_store::get_driver_store_packages();
    let legend = backup::build_device_class_legend(&filtered, &store_packages);
    let legend_path = backup_dir.join("device_class_legend.csv");
    csv_export::write_device_class_legend(&legend, &legend_path)?;
    println!("Device class legend written: {}", legend_path.display());

    println!("\nBackup location: {}", backup_dir.display());
    Ok(())
}
