//! Command-line interface definition.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "driver-backup")]
#[command(version = "2.5.2")]
#[command(about = "A tool to backup, inspect, and manage non-Microsoft drivers")]
#[command(long_about = "Driver Backup Tool v2.5.2\n\n\
    Commands:\n  \
    backup   - Export all non-Microsoft drivers from the system (requires Admin)\n  \
    inspect  - Extract driver info from installer packages (.exe, .zip, .7z, folder)\n  \
    scan     - Identify and list all INF files in a folder\n  \
    export   - Export connected device hardware IDs to CSV\n  \
    pack     - Normalize a vendor driver pack into an upload-ready layout\n\n\
    Examples:\n  \
    driver-backup backup -o D:\\Backup -v\n  \
    driver-backup inspect -p C:\\Downloads\\driver.exe -o info.csv\n  \
    driver-backup scan -p C:\\Drivers -r -g -o inventory.csv\n  \
    driver-backup pack -p D:\\Pack\\Drivers -o D:\\Normalized -v")]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Export all non-Microsoft drivers from the system (requires Administrator)
    Backup {
        /// Output directory for backup
        #[arg(short, long, default_value = "driver_backup")]
        output: PathBuf,

        /// Enable verbose output with detailed logging
        #[arg(short, long)]
        verbose: bool,

        /// Preview operations without actually exporting drivers
        #[arg(short, long)]
        dry_run: bool,
    },
    /// Extract driver information from installer package (.exe, .zip, .7z) or folder
    Inspect {
        /// Path to driver installer (.exe, .zip, .7z, .rar) or folder containing INF files
        #[arg(short, long)]
        path: PathBuf,

        /// Export results to CSV file
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Show detailed output including all device entries
        #[arg(short, long)]
        verbose: bool,
    },
    /// Scan a folder to identify and list all INF files with summary
    Scan {
        /// Path to folder containing INF files
        #[arg(short, long)]
        path: PathBuf,

        /// Export results to CSV file
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Show detailed information including all Hardware IDs
        #[arg(short, long)]
        verbose: bool,

        /// Group results by device class (Display, Net, Media, etc.)
        #[arg(short, long)]
        group: bool,

        /// Include all subfolders in scan (recursive)
        #[arg(short, long)]
        recursive: bool,
    },
    /// Export connected device hardware IDs to CSV (no driver backup, just inventory)
    Export {
        /// Output directory (for driver files) or CSV file path
        #[arg(short, long, default_value = "hardware_inventory.csv")]
        output: PathBuf,

        /// Include Microsoft drivers in export
        #[arg(short, long)]
        all: bool,

        /// Show detailed output
        #[arg(short, long)]
        verbose: bool,

        /// Also export driver files (like backup command)
        #[arg(short, long)]
        files: bool,
    },
    /// Normalize a vendor driver pack into a consistent, upload-ready layout
    Pack {
        /// Source folder containing the driver pack (e.g. a `Drivers` folder)
        #[arg(short, long)]
        path: PathBuf,

        /// Output directory for the normalized pack
        #[arg(short, long, default_value = "driver_pack")]
        output: PathBuf,

        /// Override the ODM name
        #[arg(long)]
        odm: Option<String>,

        /// Override the model identifier
        #[arg(long)]
        model: Option<String>,

        /// Override the pack revision
        #[arg(long)]
        revision: Option<String>,

        /// Keep duplicate packages instead of collapsing them
        #[arg(long)]
        keep_duplicates: bool,

        /// Only write the manifest; do not copy driver files
        #[arg(long)]
        manifest_only: bool,

        /// Preview the planned layout without writing anything
        #[arg(short, long)]
        dry_run: bool,

        /// Show detailed per-package output
        #[arg(short, long)]
        verbose: bool,
    },
}
