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

    /// Get readable name for device class GUID
    fn get_guid_readable_name(&self, guid: &str) -> String {
        match guid {
            "{4d36e968-e325-11ce-bfc1-08002be10318}" => "Display".to_string(),
            "{4d36e96c-e325-11ce-bfc1-08002be10318}" => "Media".to_string(),
            "{4d36e972-e325-11ce-bfc1-08002be10318}" => "Net".to_string(),
            "{4d36e97d-e325-11ce-bfc1-08002be10318}" => "System".to_string(),
            "{5c4c3332-344d-483c-8739-259e934c9cc8}" => "SoftwareComponent".to_string(),
            "{4d36e971-e325-11ce-bfc1-08002be10318}" => "SCSIAdapter".to_string(),
            "{4d36e969-e325-11ce-bfc1-08002be10318}" => "Keyboard".to_string(),
            "{4d36e96f-e325-11ce-bfc1-08002be10318}" => "Mouse".to_string(),
            "{4d36e96b-e325-11ce-bfc1-08002be10318}" => "USB".to_string(),
            "{4d36e96a-e325-11ce-bfc1-08002be10318}" => "HIDClass".to_string(),
            "{4d36e96e-e325-11ce-bfc1-08002be10318}" => " hdc".to_string(),
            _ => {
                if guid.starts_with('{') && guid.len() >= 36 {
                    format!("Unknown_{}", &guid[1..9])
                } else {
                    format!("UnknownClass_{}", guid)
                }
            }
        }
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

    /// Create a safe folder name for driver backup
    fn create_driver_folder_name(&self, driver: &PnPSignedDriver) -> String {
        let device_name = driver.device_name.as_deref().unwrap_or("Unknown_Device");
        let provider = driver.driver_provider_name.as_deref().unwrap_or("Unknown");
        let version = driver.driver_version.as_deref().unwrap_or("Unknown");
        let date = self.format_driver_date(&driver.driver_date);

        let estimated_capacity = device_name.len() + provider.len() + version.len() + date.len() + 3;
        let mut result = String::with_capacity(estimated_capacity);

        let mut append_cleaned = |text: &str, max_len: usize, add_underscore: bool| {
            let mut len = 0;
            for c in text.chars() {
                if len >= max_len { break; }
                if c.is_alphanumeric() || c == ' ' || c == '.' || c == '-' || c == '(' || c == ')' || c == '[' || c == ']' {
                    result.push(c);
                } else {
                    result.push('_');
                }
                len += 1;
            }
            if add_underscore {
                result.push('_');
            }
        };

        // Format: DeviceName_Provider_Version_Date
        append_cleaned(device_name, 35, true);    // Device name gets more space
        append_cleaned(provider, 20, true);      // Provider gets less space
        append_cleaned(version, 15, true);       // Version gets less space
        append_cleaned(&date, 10, false);         // Date

        if result.ends_with('_') {
            result.pop();
        }

        // Increased max length to accommodate device name
        const MAX_FOLDER_NAME_LEN: usize = 100;
        if result.len() > MAX_FOLDER_NAME_LEN {
            result.truncate(MAX_FOLDER_NAME_LEN);
            while result.ends_with('_') {
                result.pop();
            }
        }

        result
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

        // Group drivers by DriverVersion within each GUID
        let mut drivers_by_guid: HashMap<String, HashMap<String, Vec<PnPSignedDriver>>> = HashMap::new();

        for driver in drivers {
            if let Some(inf_name) = &driver.inf_name {
                if let Some(_oem_inf) = self.extract_oem_inf_name(inf_name) {
                    let class_guid = driver.class_guid.as_deref().unwrap_or("UnknownClass").to_string();
                    let driver_version = driver.driver_version.as_deref().unwrap_or("Unknown_Version").to_string();

                    let guid_entry = drivers_by_guid.entry(class_guid.clone()).or_default();
                    guid_entry.entry(driver_version).or_default().push(driver);
                } else if matches!(self.args.command, Some(Commands::Backup { verbose, .. }) if verbose) {
                    println!("Skipping non-OEM INF: {}", inf_name);
                }
            }
        }

        for (class_guid, versions_in_guid) in drivers_by_guid {
            let guid_readable_name = self.get_guid_readable_name(&class_guid);
            let guid_folder_name = format!("{}_{}",
                guid_readable_name,
                class_guid.replace("{", "").replace("}", "").replace("-", "_")
            );
            let guid_backup_dir = base_backup_dir.join(&guid_folder_name);

            if matches!(self.args.command, Some(Commands::Backup { verbose, .. }) if verbose) {
                println!("Processing GUID: {} ({})", class_guid, guid_readable_name);
                println!("  GUID Folder: {}", guid_folder_name);
                println!("  Number of driver versions in this GUID: {}", versions_in_guid.len());
                println!();
            }

            if let Some(Commands::Backup { dry_run, .. }) = &self.args.command {
                if !dry_run {
                    fs::create_dir_all(&guid_backup_dir)
                        .with_context(|| format!("Failed to create GUID directory: {}", guid_backup_dir.display()))?;
                    if !guid_backup_dir.exists() {
                        anyhow::bail!("Failed to create GUID directory: {}", guid_backup_dir.display());
                    }
                }
            }

            for (driver_version, drivers_for_version) in versions_in_guid {
                let version_folder_name = if let Some(first_driver) = drivers_for_version.first() {
                    self.create_driver_folder_name(first_driver)
                } else {
                    let clean_version = driver_version.chars()
                        .take(30)
                        .map(|c| if c.is_alphanumeric() || c == '.' || c == '-' { c } else { '_' })
                        .collect::<String>();
                    format!("Version_{}", clean_version)
                };

                let driver_backup_dir = guid_backup_dir.join(&version_folder_name);

                if matches!(self.args.command, Some(Commands::Backup { verbose, .. }) if verbose) {
                    println!("  Processing driver version: {}", driver_version);
                    println!("    Version Folder: {}", version_folder_name);
                    println!("    Number of devices with this version: {}", drivers_for_version.len());
                    println!();
                    for (index, driver) in drivers_for_version.iter().enumerate() {
                        println!("    {}. Device: {}", index + 1, driver.device_name.as_deref().unwrap_or("Unknown"));
                        println!("       INF: {}", driver.inf_name.as_deref().unwrap_or("Unknown"));
                        println!("       Description: {}", driver.description.as_deref().unwrap_or("Unknown"));
                        println!("       Provider: {}", driver.driver_provider_name.as_deref().unwrap_or("Unknown"));
                        println!("       Version: {}", driver.driver_version.as_deref().unwrap_or("Unknown"));
                        println!("       Date: {}", self.format_driver_date(&driver.driver_date));
                        println!();
                    }
                }

                if let Some(Commands::Backup { dry_run, .. }) = &self.args.command {
                    if !dry_run {
                        fs::create_dir_all(&driver_backup_dir)
                            .with_context(|| format!("Failed to create version directory: {}", driver_backup_dir.display()))?;
                        if !driver_backup_dir.exists() {
                            anyhow::bail!("Failed to create version directory: {}", driver_backup_dir.display());
                        }
                        if matches!(self.args.command, Some(Commands::Backup { verbose, .. }) if verbose) {
                            println!("    Created version folder: {}", driver_backup_dir.display());
                        }

                        let mut version_exported_count = 0;
                        for driver in &drivers_for_version {
                            if let Some(inf_name) = &driver.inf_name {
                                if let Some(oem_inf) = self.extract_oem_inf_name(inf_name) {
                                    let backup_dir_str = driver_backup_dir.to_string_lossy();
                                    if backup_dir_str.contains("..") || backup_dir_str.contains("%") {
                                        eprintln!("Skipping export due to unsafe path: {}", backup_dir_str);
                                        failed_count += 1;
                                        continue;
                                    }

                                    if matches!(self.args.command, Some(Commands::Backup { verbose, .. }) if verbose) {
                                        println!("      Exporting {} to {}...", oem_inf, driver_backup_dir.display());
                                    }

                                    let status = Command::new("pnputil")
                                        .arg("/export-driver")
                                        .arg(&oem_inf)
                                        .arg(&driver_backup_dir)
                                        .output();

                                    match status {
                                        Ok(output) => {
                                            if output.status.success() {
                                                version_exported_count += 1;
                                                driver_info.push(driver.clone());
                                                if matches!(self.args.command, Some(Commands::Backup { verbose, .. }) if verbose) {
                                                    println!("      ✓ Successfully exported: {}", oem_inf);
                                                }
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

                                                if !stdout_lower.contains("missing or invalid target directory") && exit_code != 87 {
                                                    failed_count += 1;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            eprintln!("✗ Failed to execute pnputil for {}:", oem_inf);
                                            eprintln!("  Error: {}", e);
                                            eprintln!("  → Make sure pnputil is in your PATH and you have administrative privileges.");
                                            failed_count += 1;
                                        }
                                    }
                                }
                            }
                        }
                        backed_up_count += version_exported_count;
                    } else {
                        backed_up_count += drivers_for_version.len();
                        driver_info.extend(drivers_for_version);
                    }
                }
            }
        }

        if let Some(Commands::Backup { dry_run, .. }) = &self.args.command {
            if !dry_run {
                self.create_summary_file(&base_backup_dir, &driver_info)?;
            }
        }

        println!("Driver backup process completed!");
        println!("Successfully exported: {} drivers", backed_up_count);
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

    /// Create a summary file with driver information
    fn create_summary_file(&self, backup_dir: &Path, drivers: &[PnPSignedDriver]) -> Result<()> {
        let summary_path = backup_dir.join("driver_backup_summary.txt");
        let estimated_size = drivers.len() * 200 + 500;
        let mut summary = String::with_capacity(estimated_size);

        summary.push_str("Driver Export Summary\n");
        summary.push_str(&format!("Generated: {}\n", Utc::now().format("%Y-%m-%d %H:%M:%S UTC")));
        summary.push_str(&format!("Total drivers exported: {}\n\n", drivers.len()));

        // Group drivers by device class for sequential numbering
        let mut drivers_by_class: HashMap<String, Vec<&PnPSignedDriver>> = HashMap::new();
        for driver in drivers {
            let class = driver.device_class.as_deref().unwrap_or("Unknown").to_string();
            drivers_by_class.entry(class).or_default().push(driver);
        }

        // Sort classes for consistent order
        let mut sorted_classes: Vec<_> = drivers_by_class.keys().collect();
        sorted_classes.sort();

        summary.push_str("Drivers by Class:\n");
        summary.push_str("=================\n\n");

        let mut global_counter = 1;
        for class_name in sorted_classes {
            if let Some(class_drivers) = drivers_by_class.get(class_name) {
                summary.push_str(&format!("{} ({} drivers):\n", class_name, class_drivers.len()));
                for driver in class_drivers {
                    let folder_name = self.create_driver_folder_name(driver);
                    summary.push_str(&format!("{}. {}\n", global_counter, driver.inf_name.as_deref().unwrap_or("Unknown")));
                    summary.push_str(&format!("   Device: {}\n", driver.device_name.as_deref().unwrap_or("Unknown")));
                    summary.push_str(&format!("   Provider: {}\n", driver.driver_provider_name.as_deref().unwrap_or("Unknown")));
                    summary.push_str(&format!("   Description: {}\n", driver.description.as_deref().unwrap_or("Unknown")));
                    summary.push_str(&format!("   Version: {}\n", driver.driver_version.as_deref().unwrap_or("Unknown")));
                    summary.push_str(&format!("   Date: {}\n", self.format_driver_date(&driver.driver_date)));
                    summary.push_str(&format!("   Folder: {}\n", folder_name));
                    summary.push_str(&format!("   Class GUID: {}\n", driver.class_guid.as_deref().unwrap_or("Unknown")));
                    summary.push('\n');
                    global_counter += 1;
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
