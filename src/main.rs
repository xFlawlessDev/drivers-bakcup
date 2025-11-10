use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::collections::HashMap;
use wmi::{COMLibrary, WMIConnection};

// Original driver struct
#[derive(Deserialize, Debug, Clone)]
#[serde(rename = "Win32_PnPSignedDriver")]
struct PnPSignedDriver {
    #[serde(rename = "ClassGuid")]
    class_guid: Option<String>,

    #[serde(rename = "Description")]
    description: Option<String>,

    #[serde(rename = "DeviceClass")]
    device_class: Option<String>,

    #[serde(rename = "DeviceName")]
    device_name: Option<String>,

    #[serde(rename = "DriverDate")]
    driver_date: Option<String>,

    #[serde(rename = "DriverProviderName")]
    driver_provider_name: Option<String>,

    #[serde(rename = "DriverVersion")]
    driver_version: Option<String>,

    #[serde(rename = "InfName")]
    inf_name: Option<String>,

    #[serde(rename = "HardwareID")]
    hardware_id: Option<String>,

    #[serde(rename = "DeviceID")]
    device_id: Option<String>,
}

struct DriverBackup {
    wmi_con: WMIConnection,
    args: Args,
}

impl DriverBackup {
    fn new(args: Args) -> Result<Self> {
        // Validate administrative privileges
        Self::check_admin_privileges()?;

        // Validate output directory path for backup commands
        if let Some(Commands::Backup { output, .. }) = &args.command {
            Self::validate_output_directory(output)?;
        }

        let com_con = COMLibrary::new().context("Failed to initialize COM library")?;
        let wmi_con = WMIConnection::new(com_con.into()).context("Failed to create WMI connection")?;

        Ok(Self { wmi_con, args })
    }

    /// Check if the program is running with administrative privileges
    fn check_admin_privileges() -> Result<()> {
        let test_path = Path::new("C:\\Windows\\Temp\\driver_backup_admin_test");
        match fs::write(test_path, "test") {
            Ok(_) => {
                let _ = fs::remove_file(test_path);
                Ok(())
            }
            Err(_) => anyhow::bail!(
                "This program requires administrative privileges to access driver information. \
                 Please run as Administrator."
            ),
        }
    }

    /// Validate that the output directory exists or can be created
    fn validate_output_directory(output: &PathBuf) -> Result<()> {
        if output.exists() && !output.is_dir() {
            anyhow::bail!("Output path exists but is not a directory: {}", output.display());
        }

        if !output.exists() {
            fs::create_dir_all(output)
                .with_context(|| format!("Failed to create output directory: {}", output.display()))?;
        }

        // Test write permissions
        let test_file = output.join("write_test.tmp");
        fs::write(&test_file, "test")
            .with_context(|| format!("Cannot write to output directory: {}", output.display()))?;
        fs::remove_file(&test_file).ok();

        Ok(())
    }

    /// Get all signed drivers from WMI
    async fn get_drivers(&self) -> Result<Vec<PnPSignedDriver>> {
        let drivers: Vec<PnPSignedDriver> = self.wmi_con.query()
            .context("Failed to query WMI for PnP signed drivers")?;

        Ok(drivers)
    }

    /// Check if a driver is from Microsoft
    fn is_microsoft_driver(&self, driver: &PnPSignedDriver) -> bool {
        if let Some(ref provider) = driver.driver_provider_name {
            provider.to_lowercase().contains("microsoft")
        } else {
            false
        }
    }

    /// Filter out Microsoft drivers, keeping only third-party drivers
    fn filter_non_microsoft_drivers(&self, drivers: Vec<PnPSignedDriver>) -> Vec<PnPSignedDriver> {
        drivers.into_iter()
            .filter(|driver| !self.is_microsoft_driver(driver))
            .collect()
    }

    /// Create the main backup directory structure
    fn create_base_backup_directory(&self, output: &PathBuf) -> Result<PathBuf> {
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let backup_dir = output.join(format!("drivers_{}", timestamp));

        fs::create_dir_all(&backup_dir)
            .with_context(|| format!("Failed to create backup directory: {}", backup_dir.display()))?;

        Ok(backup_dir)
    }

    /// Format driver date to a readable format
    fn format_driver_date(&self, driver_date: &Option<String>) -> String {
        match driver_date {
            Some(date_str) => {
                if date_str.len() >= 8 {
                    if date_str[0..8].chars().all(|c| c.is_ascii_digit()) {
                        let year = &date_str[0..4];
                        let month = &date_str[4..6];
                        let day = &date_str[6..8];
                        if let (Ok(month_num), Ok(day_num)) = (month.parse::<u32>(), day.parse::<u32>()) {
                            if month_num >= 1 && month_num <= 12 && day_num >= 1 && day_num <= 31 {
                                return format!("{}-{}-{}", year, month, day);
                            }
                        }
                    }
                    date_str.clone()
                } else {
                    date_str.clone()
                }
            }
            None => "Unknown".to_string()
        }
    }

    /// Extract OEM INF name from driver
    fn extract_oem_inf_name(&self, inf_name: &str) -> Option<String> {
        let inf_lower = inf_name.to_lowercase();
        if inf_lower.starts_with("oem") && inf_lower.ends_with(".inf") {
            // Validate characters
            if inf_lower.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') {
                Some(inf_lower)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Backup drivers to the specified directory
    async fn backup_drivers(&self, drivers: Vec<PnPSignedDriver>) -> Result<()> {
        let output_path = match &self.args.command {
            Some(Commands::Backup { output, .. }) => output.clone(),
            _ => PathBuf::from("driver_backup")
        };
        let base_backup_dir = self.create_base_backup_directory(&output_path)?;
        let mut backed_up_count = 0;
        let mut failed_count = 0;
        let mut driver_info = Vec::new();

        // Group drivers by Device Class, then by INF file name
        let mut drivers_by_class_inf: HashMap<String, HashMap<String, Vec<PnPSignedDriver>>> = HashMap::new();

        for driver in drivers {
            if let Some(inf_name) = &driver.inf_name {
                if let Some(oem_inf) = self.extract_oem_inf_name(inf_name) {
                    let device_class = driver.device_class.as_deref().unwrap_or("Unknown_Class").to_string();
                    
                    drivers_by_class_inf
                        .entry(device_class)
                        .or_default()
                        .entry(oem_inf)
                        .or_default()
                        .push(driver);
                } else if matches!(self.args.command, Some(Commands::Backup { verbose, .. }) if verbose) {
                    println!("Skipping non-OEM INF: {}", inf_name);
                }
            }
        }

        // Sort by device class for consistent order
        let mut sorted_class_keys: Vec<_> = drivers_by_class_inf.keys().cloned().collect();
        sorted_class_keys.sort();

        for device_class in sorted_class_keys {
            if let Some(infs_in_class) = drivers_by_class_inf.get(&device_class) {
                // Create device class folder
                let class_folder_name = device_class
                    .chars()
                    .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' })
                    .collect::<String>();
                let class_backup_dir = base_backup_dir.join(&class_folder_name);

                if matches!(self.args.command, Some(Commands::Backup { verbose, .. }) if verbose) {
                    println!("Processing Device Class: {}", device_class);
                    println!("  Class Folder: {}", class_folder_name);
                    println!("  Number of driver packages in this class: {}", infs_in_class.len());
                    println!();
                }

                if let Some(Commands::Backup { dry_run, .. }) = &self.args.command {
                    if !dry_run {
                        fs::create_dir_all(&class_backup_dir)
                            .with_context(|| format!("Failed to create class directory: {}", class_backup_dir.display()))?;
                    }
                }

                // Sort INF names within this class
                let mut sorted_inf_keys: Vec<_> = infs_in_class.keys().cloned().collect();
                sorted_inf_keys.sort();

                for oem_inf in sorted_inf_keys {
                    if let Some(drivers_for_package) = infs_in_class.get(&oem_inf) {
                        // Get the primary device name and version for folder naming
                        let primary_device_name = drivers_for_package
                            .first()
                            .and_then(|d| d.device_name.as_deref())
                            .unwrap_or("Unknown_Device");
                        
                        let driver_version = drivers_for_package
                            .first()
                            .and_then(|d| d.driver_version.as_deref())
                            .unwrap_or("Unknown_Version");
                        
                        // Create folder name: "DeviceName_Version Package"
                        let folder_name = format!("{}_{} Package", primary_device_name, driver_version)
                            .chars()
                            .map(|c| if c.is_alphanumeric() || c == ' ' || c == '.' || c == '-' || c == '_' || c == '(' || c == ')' { c } else { '_' })
                            .collect::<String>();

                        let driver_backup_dir = class_backup_dir.join(&folder_name);

                        if matches!(self.args.command, Some(Commands::Backup { verbose, .. }) if verbose) {
                            println!("  Processing driver package: {} v{} ({})", primary_device_name, driver_version, oem_inf);
                            println!("    Folder: {}", folder_name);
                            println!("    Number of devices in this package: {}", drivers_for_package.len());
                            println!();
                            for (index, driver) in drivers_for_package.iter().enumerate() {
                                println!("      {}. Device: {}", index + 1, driver.device_name.as_deref().unwrap_or("Unknown"));
                                println!("         INF: {}", driver.inf_name.as_deref().unwrap_or("Unknown"));
                                println!("         Hardware ID: {}", driver.hardware_id.as_deref().unwrap_or("Unknown"));
                                println!("         Device ID: {}", driver.device_id.as_deref().unwrap_or("Unknown"));
                                println!("         Description: {}", driver.description.as_deref().unwrap_or("Unknown"));
                                println!("         Provider: {}", driver.driver_provider_name.as_deref().unwrap_or("Unknown"));
                                println!("         Version: {}", driver.driver_version.as_deref().unwrap_or("Unknown"));
                                println!("         Date: {}", self.format_driver_date(&driver.driver_date));
                                println!();
                            }
                        }

                        if let Some(Commands::Backup { dry_run, .. }) = &self.args.command {
                            if !dry_run {
                                fs::create_dir_all(&driver_backup_dir)
                                    .with_context(|| format!("Failed to create driver directory: {}", driver_backup_dir.display()))?;
                                if !driver_backup_dir.exists() {
                                    anyhow::bail!("Failed to create driver directory: {}", driver_backup_dir.display());
                                }
                                if matches!(self.args.command, Some(Commands::Backup { verbose, .. }) if verbose) {
                                    println!("      Created folder: {}", driver_backup_dir.display());
                                }

                                // Export the driver package (only need to export once per INF)
                                let backup_dir_str = driver_backup_dir.to_string_lossy();
                                if backup_dir_str.contains("..") || backup_dir_str.contains("%") {
                                    eprintln!("Skipping export due to unsafe path: {}", backup_dir_str);
                                    failed_count += 1;
                                    continue;
                                }

                                if matches!(self.args.command, Some(Commands::Backup { verbose, .. }) if verbose) {
                                    println!("        Exporting {} to {}...", oem_inf, driver_backup_dir.display());
                                }

                                let status = Command::new("pnputil")
                                    .arg("/export-driver")
                                    .arg(&oem_inf)
                                    .arg(&driver_backup_dir)
                                    .output();

                                match status {
                                    Ok(output) => {
                                        if output.status.success() {
                                            backed_up_count += 1;
                                            driver_info.extend(drivers_for_package.clone());
                                            if matches!(self.args.command, Some(Commands::Backup { verbose, .. }) if verbose) {
                                                println!("        ✓ Successfully exported: {}", oem_inf);
                                            }
                                            
                                            // Create CSV file for this driver package
                                            self.create_driver_csv(&driver_backup_dir, drivers_for_package)?;
                                        } else {
                                            let stdout = String::from_utf8_lossy(&output.stdout);
                                            let stderr = String::from_utf8_lossy(&output.stderr);
                                            
                                            eprintln!("✗ Failed to export {}:", oem_inf);
                                            if !stdout.is_empty() {
                                                eprintln!("  stdout: {}", stdout.trim());
                                            }
                                            if !stderr.is_empty() {
                                                eprintln!("  stderr: {}", stderr.trim());
                                            }
                                            let exit_code = output.status.code().unwrap_or(-1);
                                            let stderr_lower = stderr.to_lowercase();
                                            let stdout_lower = stdout.to_lowercase();

                                            if stderr_lower.contains("access") || stderr_lower.contains("denied") {
                                                eprintln!("  → This might be a permissions issue. Try running as Administrator.");
                                            } else if stderr_lower.contains("not found") || stderr_lower.contains("cannot find") {
                                                eprintln!("  → Driver package might be corrupted or already removed.");
                                            } else if stdout_lower.contains("missing or invalid target directory") || exit_code == 87 {
                                                eprintln!("  → Path too long or invalid. Using shorter path and retrying...");
                                            } else if stdout_lower.contains("the data is invalid") || exit_code == 13 {
                                                eprintln!("  → This driver may be protected or corrupted. Skipping.");
                                            }

                                            failed_count += 1;
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("✗ Failed to execute pnputil for {}:", oem_inf);
                                        eprintln!("  Error: {}", e);
                                        eprintln!("  → Make sure pnputil is in your PATH and you have administrative privileges.");
                                        failed_count += 1;
                                    }
                                }
                            } else {
                                backed_up_count += 1;
                                driver_info.extend(drivers_for_package.clone());
                            }
                        }
                    }
                }
            }
        }

        if let Some(Commands::Backup { dry_run, .. }) = &self.args.command {
            if !dry_run {
                self.create_summary_file(&base_backup_dir, &driver_info)?;
                self.create_master_csv(&base_backup_dir, &driver_info)?;
            }
        }

        println!("Driver backup process completed!");
        println!("Successfully exported: {} driver packages", backed_up_count);
        if failed_count > 0 {
            println!("Failed to export: {} drivers", failed_count);
        }

        if let Some(Commands::Backup { dry_run, output, .. }) = &self.args.command {
            if !dry_run {
                println!("Backup location: {}", output.display());
            }
        }

        Ok(())
    }

    /// Create a CSV file for a specific driver package (for database upload)
    fn create_driver_csv(&self, driver_dir: &Path, drivers: &[PnPSignedDriver]) -> Result<()> {
        let csv_path = driver_dir.join("driver_info.csv");
        let mut csv_content = String::new();
        
        // CSV Header
        csv_content.push_str("Device Name,Driver Version,Driver Date,Hardware ID,Device ID,INF Name,Description,Provider,Device Class,Class GUID\n");
        
        for driver in drivers {
            let device_name = driver.device_name.as_deref().unwrap_or("Unknown");
            let driver_version = driver.driver_version.as_deref().unwrap_or("Unknown");
            let driver_date = self.format_driver_date(&driver.driver_date);
            let hardware_id = driver.hardware_id.as_deref().unwrap_or("Unknown");
            let device_id = driver.device_id.as_deref().unwrap_or("Unknown");
            let inf_name = driver.inf_name.as_deref().unwrap_or("Unknown");
            let description = driver.description.as_deref().unwrap_or("Unknown");
            let provider = driver.driver_provider_name.as_deref().unwrap_or("Unknown");
            let device_class = driver.device_class.as_deref().unwrap_or("Unknown");
            let class_guid = driver.class_guid.as_deref().unwrap_or("Unknown");
            
            // Escape CSV fields that might contain commas or quotes
            let escape_csv = |s: &str| -> String {
                if s.contains(',') || s.contains('"') || s.contains('\n') {
                    format!("\"{}\"", s.replace("\"", "\"\""))
                } else {
                    s.to_string()
                }
            };
            
            csv_content.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{}\n",
                escape_csv(device_name),
                escape_csv(driver_version),
                escape_csv(&driver_date),
                escape_csv(hardware_id),
                escape_csv(device_id),
                escape_csv(inf_name),
                escape_csv(description),
                escape_csv(provider),
                escape_csv(device_class),
                escape_csv(class_guid)
            ));
        }
        
        fs::write(&csv_path, csv_content)
            .with_context(|| format!("Failed to write CSV file: {}", csv_path.display()))?;
        
        if matches!(self.args.command, Some(Commands::Backup { verbose, .. }) if verbose) {
            println!("      Created CSV file: {}", csv_path.display());
        }
        
        Ok(())
    }

    /// Create a master CSV file with all driver information
    fn create_master_csv(&self, backup_dir: &Path, drivers: &[PnPSignedDriver]) -> Result<()> {
        let csv_path = backup_dir.join("all_drivers.csv");
        let mut csv_content = String::new();
        
        // CSV Header
        csv_content.push_str("Device Name,Driver Version,Driver Date,Hardware ID,Device ID,INF Name,Description,Provider,Device Class,Class GUID,Folder Name\n");
        
        for driver in drivers {
            let device_name = driver.device_name.as_deref().unwrap_or("Unknown");
            let driver_version = driver.driver_version.as_deref().unwrap_or("Unknown");
            let driver_date = self.format_driver_date(&driver.driver_date);
            let hardware_id = driver.hardware_id.as_deref().unwrap_or("Unknown");
            let device_id = driver.device_id.as_deref().unwrap_or("Unknown");
            let inf_name = driver.inf_name.as_deref().unwrap_or("Unknown");
            let description = driver.description.as_deref().unwrap_or("Unknown");
            let provider = driver.driver_provider_name.as_deref().unwrap_or("Unknown");
            let device_class = driver.device_class.as_deref().unwrap_or("Unknown");
            let class_guid = driver.class_guid.as_deref().unwrap_or("Unknown");
            
            // Create folder name based on device class/device name_version Package
            let class_folder = device_class
                .chars()
                .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' })
                .collect::<String>();
            
            let driver_folder = format!("{}_{} Package", device_name, driver_version)
                .chars()
                .map(|c| if c.is_alphanumeric() || c == ' ' || c == '.' || c == '-' || c == '_' || c == '(' || c == ')' { c } else { '_' })
                .collect::<String>();
            
            let folder_name = format!("{}/{}", class_folder, driver_folder);
            
            // Escape CSV fields that might contain commas or quotes
            let escape_csv = |s: &str| -> String {
                if s.contains(',') || s.contains('"') || s.contains('\n') {
                    format!("\"{}\"", s.replace("\"", "\"\""))
                } else {
                    s.to_string()
                }
            };
            
            csv_content.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{}\n",
                escape_csv(device_name),
                escape_csv(driver_version),
                escape_csv(&driver_date),
                escape_csv(hardware_id),
                escape_csv(device_id),
                escape_csv(inf_name),
                escape_csv(description),
                escape_csv(provider),
                escape_csv(device_class),
                escape_csv(class_guid),
                escape_csv(&folder_name)
            ));
        }
        
        fs::write(&csv_path, csv_content)
            .with_context(|| format!("Failed to write master CSV file: {}", csv_path.display()))?;
        
        if matches!(self.args.command, Some(Commands::Backup { verbose, .. }) if verbose) {
            println!("Created master CSV file: {}", csv_path.display());
        }
        
        Ok(())
    }

    /// Create a summary file with driver information
    fn create_summary_file(&self, backup_dir: &Path, drivers: &[PnPSignedDriver]) -> Result<()> {
        let summary_path = backup_dir.join("driver_backup_summary.txt");
        let estimated_size = drivers.len() * 300 + 500;
        let mut summary = String::with_capacity(estimated_size);

        summary.push_str("Driver Export Summary\n");
        summary.push_str(&format!("Generated: {}\n", Utc::now().format("%Y-%m-%d %H:%M:%S UTC")));
        summary.push_str(&format!("Total drivers exported: {}\n\n", drivers.len()));

        // Group drivers by Device Class, then by INF file
        let mut drivers_by_class_inf: HashMap<String, HashMap<String, Vec<&PnPSignedDriver>>> = HashMap::new();
        for driver in drivers {
            let device_class = driver.device_class.as_deref().unwrap_or("Unknown").to_string();
            let inf = driver.inf_name.as_deref().unwrap_or("Unknown").to_string();
            drivers_by_class_inf
                .entry(device_class)
                .or_default()
                .entry(inf)
                .or_default()
                .push(driver);
        }

        // Sort device classes for consistent order
        let mut sorted_classes: Vec<_> = drivers_by_class_inf.keys().collect();
        sorted_classes.sort();

        summary.push_str("Drivers by Device Class and Package:\n");
        summary.push_str("=====================================\n\n");

        let mut global_counter = 1;
        for device_class in sorted_classes {
            if let Some(infs_in_class) = drivers_by_class_inf.get(device_class) {
                summary.push_str(&format!("=== {} ({} packages) ===\n\n", device_class, infs_in_class.len()));
                
                // Sort INF names within class
                let mut sorted_infs: Vec<_> = infs_in_class.keys().collect();
                sorted_infs.sort();
                
                for inf_name in sorted_infs {
                    if let Some(inf_drivers) = infs_in_class.get(inf_name) {
                        let primary_device = inf_drivers.first()
                            .and_then(|d| d.device_name.as_deref())
                            .unwrap_or("Unknown");
                        let driver_version = inf_drivers.first()
                            .and_then(|d| d.driver_version.as_deref())
                            .unwrap_or("Unknown");
                        
                        let folder_name = format!("{}_{} Package", primary_device, driver_version)
                            .chars()
                            .map(|c| if c.is_alphanumeric() || c == ' ' || c == '.' || c == '-' || c == '_' || c == '(' || c == ')' { c } else { '_' })
                            .collect::<String>();
                        
                        summary.push_str(&format!("{}. {} ({} devices in package):\n", global_counter, inf_name, inf_drivers.len()));
                        summary.push_str(&format!("   Folder: {}/{}\n", device_class, folder_name));
                        
                        if let Some(first_driver) = inf_drivers.first() {
                            summary.push_str(&format!("   Provider: {}\n", first_driver.driver_provider_name.as_deref().unwrap_or("Unknown")));
                            summary.push_str(&format!("   Version: {}\n", first_driver.driver_version.as_deref().unwrap_or("Unknown")));
                            summary.push_str(&format!("   Date: {}\n", self.format_driver_date(&first_driver.driver_date)));
                        }
                        
                        summary.push_str("\n   Devices in this package:\n");
                        for (idx, driver) in inf_drivers.iter().enumerate() {
                            summary.push_str(&format!("   {}. {}\n", idx + 1, driver.device_name.as_deref().unwrap_or("Unknown")));
                            summary.push_str(&format!("      Hardware ID: {}\n", driver.hardware_id.as_deref().unwrap_or("Unknown")));
                            summary.push_str(&format!("      Device ID: {}\n", driver.device_id.as_deref().unwrap_or("Unknown")));
                            summary.push_str(&format!("      Description: {}\n", driver.description.as_deref().unwrap_or("Unknown")));
                        }
                        summary.push('\n');
                        global_counter += 1;
                    }
                }
                summary.push('\n');
            }
        }

        fs::write(&summary_path, summary)
            .with_context(|| format!("Failed to write summary file: {}", summary_path.display()))?;

        if !summary_path.exists() {
            anyhow::bail!("Summary file was not created successfully: {}", summary_path.display());
        }

        if matches!(self.args.command, Some(Commands::Backup { verbose, .. }) if verbose) {
            println!("Created summary file: {}", summary_path.display());
        }

        Ok(())
    }

    /// Run the backup process
    async fn run(&self) -> Result<()> {
        println!("Starting driver export process...");

        let all_drivers = self.get_drivers().await?;

        let non_ms_drivers = self.filter_non_microsoft_drivers(all_drivers);

        if non_ms_drivers.is_empty() {
            println!("No non-Microsoft drivers found to export.");
            return Ok(());
        }

        self.backup_drivers(non_ms_drivers).await?;
        Ok(())
    }
}

// Add CLI arguments for backup functionality
#[derive(Parser)]
#[command(name = "driver-backup")]
#[command(about = "A tool to backup and manage non-Microsoft drivers")]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Backup drivers to specified directory
    Backup {
        #[arg(short, long, default_value = "driver_backup")]
        output: PathBuf,

        #[arg(short, long)]
        verbose: bool,

        #[arg(short, long)]
        dry_run: bool,
    },
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.command.unwrap_or(Commands::Backup {
        output: PathBuf::from("driver_backup"),
        verbose: false,
        dry_run: false,
    }) {
        Commands::Backup { output, verbose, dry_run } => {
            if verbose {
                println!("Driver Export Tool");
                println!("==================");
                println!("Output directory: {}", output.display());
                println!("Dry run: {}", dry_run);
                println!();
            }

            // Create args for DriverBackup
            let backup_args = Args {
                command: Some(Commands::Backup {
                    output: output.clone(),
                    verbose,
                    dry_run,
                })
            };

            // Initialize backup functionality
            let backup = DriverBackup::new(backup_args)?;

            // Run the backup process
            tokio::runtime::Runtime::new()?.block_on(backup.run())?;
        }
    }

    // Add pause before closing
    println!("\nPress Enter to close...");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).expect("Failed to read line");

    Ok(())
}
