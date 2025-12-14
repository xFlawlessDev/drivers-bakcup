use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::collections::HashMap;
use wmi::{COMLibrary, WMIConnection};

// Struct for parsed INF driver information (mirrors PnPSignedDriver structure)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct InfDriverInfo {
    device_name: Option<String>,
    description: Option<String>,
    device_class: Option<String>,
    class_guid: Option<String>,
    driver_version: Option<String>,
    driver_date: Option<String>,
    driver_provider_name: Option<String>,
    hardware_id: Option<String>,
    inf_name: Option<String>,
    catalog_file: Option<String>,
    manufacturer: Option<String>,
}

// Struct for parsed INF file
#[derive(Debug, Clone)]
struct ParsedInfFile {
    file_path: PathBuf,
    file_name: String,
    drivers: Vec<InfDriverInfo>,
    raw_version_info: InfVersionInfo,
}

#[derive(Debug, Clone, Default)]
struct InfVersionInfo {
    driver_version: Option<String>,
    driver_date: Option<String>,
    class: Option<String>,
    class_guid: Option<String>,
    provider: Option<String>,
    catalog_file: Option<String>,
}

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

        println!("\nDriver export completed!");
        println!("Successfully exported: {} driver packages", backed_up_count);
        if failed_count > 0 {
            println!("Failed to export: {} drivers", failed_count);
        }

        if let Some(Commands::Backup { dry_run, verbose, .. }) = &self.args.command {
            if !dry_run {
                println!("\nScanning exported drivers to create summary...");
                
                // Use InfParser to scan the backup folder and create summary CSV
                let csv_path = base_backup_dir.join("all_drivers.csv");
                InfParser::scan_and_export(&base_backup_dir, &csv_path, *verbose)?;
                
                println!("\nBackup location: {}", base_backup_dir.display());
            }
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

    /// Build lookup table for OEM INF to actual INF name mapping
    fn build_inf_lookup() -> HashMap<String, String> {
        let mut lookup = HashMap::new();
        
        println!("Building INF name lookup table...");
        
        let output = Command::new("pnputil")
            .arg("/enum-drivers")
            .output();
        
        if let Ok(result) = output {
            let stdout = String::from_utf8_lossy(&result.stdout);
            let mut current_oem: Option<String> = None;
            let mut current_original: Option<String> = None;
            
            for line in stdout.lines() {
                let line_lower = line.to_lowercase();
                
                if line_lower.contains("published name") {
                    if let Some(val) = line.split(':').nth(1) {
                        current_oem = Some(val.trim().to_lowercase());
                    }
                }
                if line_lower.contains("original name") {
                    if let Some(val) = line.split(':').nth(1) {
                        current_original = Some(val.trim().to_string());
                    }
                }
                
                // Save mapping when we have both
                if let (Some(ref oem), Some(ref original)) = (&current_oem, &current_original) {
                    lookup.insert(oem.clone(), original.clone());
                    current_oem = None;
                    current_original = None;
                }
            }
        }
        
        println!("Found {} INF mappings", lookup.len());
        lookup
    }

    /// Export WMI driver info to CSV, grouped by driver version (collection)
    fn export_wmi_drivers_csv_static(drivers: &[PnPSignedDriver], output_path: &Path, verbose: bool) -> Result<()> {
        let escape_csv = |s: &str| -> String {
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.to_string()
            }
        };

        // Build INF lookup table once
        let inf_lookup = Self::build_inf_lookup();

        // Group drivers by driver version (collection)
        let mut grouped: HashMap<String, Vec<&PnPSignedDriver>> = HashMap::new();
        for driver in drivers {
            let version = driver.driver_version.as_deref().unwrap_or("Unknown").to_string();
            grouped.entry(version).or_default().push(driver);
        }

        let mut csv_content = String::new();
        csv_content.push_str("Collection,Device Class,Provider,Driver Version,Driver Date,Device Count,Actual INFs,Device Names,Hardware IDs\n");

        // Sort by provider then version
        let mut sorted_keys: Vec<_> = grouped.keys().cloned().collect();
        sorted_keys.sort();

        for version in &sorted_keys {
            if let Some(drivers_for_version) = grouped.get(version) {
                let first = drivers_for_version.first().unwrap();
                
                let driver_date = first.driver_date.as_ref()
                    .map(|d| {
                        if d.len() >= 8 && d[0..8].chars().all(|c| c.is_ascii_digit()) {
                            format!("{}-{}-{}", &d[0..4], &d[4..6], &d[6..8])
                        } else {
                            d.clone()
                        }
                    })
                    .unwrap_or_else(|| "Unknown".to_string());

                // Collect unique actual INF names
                let mut actual_infs: Vec<String> = drivers_for_version.iter()
                    .filter_map(|d| {
                        let oem = d.inf_name.as_deref()?.to_lowercase();
                        Some(inf_lookup.get(&oem).cloned().unwrap_or(oem))
                    })
                    .collect();
                actual_infs.sort();
                actual_infs.dedup();

                // Collect device names and hardware IDs
                let device_names: Vec<String> = drivers_for_version.iter()
                    .filter_map(|d| d.device_name.clone())
                    .collect();
                let hardware_ids: Vec<String> = drivers_for_version.iter()
                    .filter_map(|d| d.hardware_id.clone())
                    .collect();

                // Create collection name from provider + version
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
        }

        fs::write(output_path, &csv_content)
            .with_context(|| format!("Failed to write CSV file: {}", output_path.display()))?;

        println!("CSV created: {}", output_path.display());
        println!("Total collections: {}", grouped.len());
        println!("Total devices: {}", drivers.len());

        if verbose {
            println!("\nDriver collections exported:");
            for version in &sorted_keys {
                if let Some(drivers_for_version) = grouped.get(version) {
                    let first = drivers_for_version.first().unwrap();
                    let provider = first.driver_provider_name.as_deref().unwrap_or("Unknown");
                    println!("\n  {} {} - {} devices", provider, version, drivers_for_version.len());
                    for driver in drivers_for_version {
                        let oem = driver.inf_name.as_deref().unwrap_or("unknown").to_lowercase();
                        let actual = inf_lookup.get(&oem).map(|s| s.as_str()).unwrap_or(&oem);
                        println!("    - {} | {} | {}", 
                            driver.device_name.as_deref().unwrap_or("Unknown"),
                            driver.hardware_id.as_deref().unwrap_or("Unknown"),
                            actual);
                    }
                }
            }
        }

        Ok(())
    }
}

// INF Parser for extracting driver information from INF files
struct InfParser;

impl InfParser {
    /// Extract driver package from installer (.exe, .zip) or use folder directly
    fn extract_or_use_path(path: &Path, verbose: bool) -> Result<(PathBuf, bool)> {
        if path.is_dir() {
            return Ok((path.to_path_buf(), false));
        }

        let extension = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        match extension.as_str() {
            "exe" | "zip" | "7z" | "rar" => {
                let temp_dir = std::env::temp_dir().join(format!("driver_inspect_{}", std::process::id()));
                fs::create_dir_all(&temp_dir)?;

                if verbose {
                    println!("Extracting {} to {}...", path.display(), temp_dir.display());
                }

                // Try 7z first, then fall back to other methods
                let extract_result = Self::extract_with_7z(path, &temp_dir)
                    .or_else(|_| Self::extract_with_powershell(path, &temp_dir));

                match extract_result {
                    Ok(_) => {
                        if verbose {
                            println!("Successfully extracted to {}", temp_dir.display());
                        }
                        Ok((temp_dir, true))
                    }
                    Err(e) => {
                        let _ = fs::remove_dir_all(&temp_dir);
                        Err(e)
                    }
                }
            }
            "inf" => {
                // Single INF file - use parent directory
                Ok((path.parent().unwrap_or(Path::new(".")).to_path_buf(), false))
            }
            _ => anyhow::bail!("Unsupported file type: {}", extension)
        }
    }

    fn extract_with_7z(archive: &Path, dest: &Path) -> Result<()> {
        // Try common 7z locations
        let seven_zip_paths = [
            "7z",
            "C:\\Program Files\\7-Zip\\7z.exe",
            "C:\\Program Files (x86)\\7-Zip\\7z.exe",
        ];

        for seven_zip in &seven_zip_paths {
            let output = Command::new(seven_zip)
                .arg("x")
                .arg("-y")
                .arg(format!("-o{}", dest.display()))
                .arg(archive)
                .output();

            if let Ok(result) = output {
                if result.status.success() {
                    return Ok(());
                }
            }
        }

        anyhow::bail!("7-Zip not found or extraction failed")
    }

    fn extract_with_powershell(archive: &Path, dest: &Path) -> Result<()> {
        let extension = archive.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        if extension == "zip" {
            let output = Command::new("powershell")
                .arg("-Command")
                .arg(format!(
                    "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                    archive.display(),
                    dest.display()
                ))
                .output()?;

            if output.status.success() {
                return Ok(());
            }
        }

        anyhow::bail!("PowerShell extraction failed or unsupported format")
    }

    /// Find all INF files in a directory recursively
    fn find_inf_files(dir: &Path) -> Result<Vec<PathBuf>> {
        let mut inf_files = Vec::new();
        Self::find_inf_files_recursive(dir, &mut inf_files)?;
        inf_files.sort();
        Ok(inf_files)
    }

    /// Find INF files in a single folder (non-recursive)
    fn find_inf_files_in_folder(dir: &Path) -> Result<Vec<PathBuf>> {
        let mut inf_files = Vec::new();
        
        if !dir.is_dir() {
            return Ok(inf_files);
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext.to_string_lossy().to_lowercase() == "inf" {
                        inf_files.push(path);
                    }
                }
            }
        }

        inf_files.sort();
        Ok(inf_files)
    }

    fn find_inf_files_recursive(dir: &Path, inf_files: &mut Vec<PathBuf>) -> Result<()> {
        if !dir.is_dir() {
            return Ok(());
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                Self::find_inf_files_recursive(&path, inf_files)?;
            } else if let Some(ext) = path.extension() {
                if ext.to_string_lossy().to_lowercase() == "inf" {
                    inf_files.push(path);
                }
            }
        }

        Ok(())
    }

    /// Parse a single INF file
    fn parse_inf_file(inf_path: &Path) -> Result<ParsedInfFile> {
        // Try different encodings (INF files can be UTF-8, UTF-16, or ANSI)
        let content = Self::read_inf_content(inf_path)?;
        
        let file_name = inf_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.inf")
            .to_string();

        let mut version_info = InfVersionInfo::default();
        let mut manufacturers: HashMap<String, String> = HashMap::new();
        let mut device_sections: HashMap<String, Vec<(String, String)>> = HashMap::new();
        let mut string_table: HashMap<String, String> = HashMap::new();
        let mut current_section = String::new();

        for line in content.lines() {
            let line = line.trim();
            
            // Skip empty lines and comments
            if line.is_empty() || line.starts_with(';') {
                continue;
            }

            // Section header
            if line.starts_with('[') && line.ends_with(']') {
                current_section = line[1..line.len()-1].to_lowercase();
                continue;
            }

            // Parse based on current section
            match current_section.as_str() {
                "version" => Self::parse_version_line(line, &mut version_info),
                "manufacturer" => Self::parse_manufacturer_line(line, &mut manufacturers),
                "strings" => Self::parse_strings_line(line, &mut string_table),
                section if manufacturers.values().any(|v| {
                    let sec_lower = section.to_lowercase();
                    v.to_lowercase().starts_with(&sec_lower) || sec_lower.starts_with(&v.to_lowercase())
                }) => {
                    Self::parse_device_line(line, &current_section, &mut device_sections);
                }
                _ => {
                    // Check if this is a device section
                    for mfg_section in manufacturers.values() {
                        let base_section = mfg_section.split(',').next().unwrap_or(mfg_section);
                        if current_section.to_lowercase().starts_with(&base_section.to_lowercase()) {
                            Self::parse_device_line(line, &current_section, &mut device_sections);
                            break;
                        }
                    }
                }
            }
        }

        // Build driver info list
        let mut drivers = Vec::new();
        
        for (section_name, devices) in &device_sections {
            for (device_desc, hardware_id) in devices {
                // Resolve string references
                let resolved_desc = Self::resolve_string(device_desc, &string_table);
                let resolved_provider = version_info.provider.as_ref()
                    .map(|p| Self::resolve_string(p, &string_table));

                // Find manufacturer for this section
                let manufacturer = manufacturers.iter()
                    .find(|(_, sec)| {
                        let base = sec.split(',').next().unwrap_or(sec);
                        section_name.to_lowercase().starts_with(&base.to_lowercase())
                    })
                    .map(|(name, _)| Self::resolve_string(name, &string_table));

                let driver_info = InfDriverInfo {
                    device_name: Some(resolved_desc.clone()),
                    description: Some(resolved_desc),
                    device_class: version_info.class.clone(),
                    class_guid: version_info.class_guid.clone(),
                    driver_version: version_info.driver_version.clone(),
                    driver_date: version_info.driver_date.clone(),
                    driver_provider_name: resolved_provider,
                    hardware_id: Some(hardware_id.clone()),
                    inf_name: Some(file_name.clone()),
                    catalog_file: version_info.catalog_file.clone(),
                    manufacturer,
                };

                drivers.push(driver_info);
            }
        }

        Ok(ParsedInfFile {
            file_path: inf_path.to_path_buf(),
            file_name,
            drivers,
            raw_version_info: version_info,
        })
    }

    fn read_inf_content(path: &Path) -> Result<String> {
        // First try reading as bytes and detect encoding
        let bytes = fs::read(path)?;
        
        // Check for UTF-16 LE BOM
        if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
            let utf16_chars: Vec<u16> = bytes[2..]
                .chunks(2)
                .filter_map(|chunk| {
                    if chunk.len() == 2 {
                        Some(u16::from_le_bytes([chunk[0], chunk[1]]))
                    } else {
                        None
                    }
                })
                .collect();
            return Ok(String::from_utf16_lossy(&utf16_chars));
        }
        
        // Check for UTF-16 BE BOM
        if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
            let utf16_chars: Vec<u16> = bytes[2..]
                .chunks(2)
                .filter_map(|chunk| {
                    if chunk.len() == 2 {
                        Some(u16::from_be_bytes([chunk[0], chunk[1]]))
                    } else {
                        None
                    }
                })
                .collect();
            return Ok(String::from_utf16_lossy(&utf16_chars));
        }

        // Check for UTF-8 BOM
        if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
            return Ok(String::from_utf8_lossy(&bytes[3..]).to_string());
        }

        // Try UTF-8, fall back to Windows-1252/Latin-1
        match String::from_utf8(bytes.clone()) {
            Ok(s) => Ok(s),
            Err(_) => Ok(bytes.iter().map(|&b| b as char).collect())
        }
    }

    fn parse_version_line(line: &str, version_info: &mut InfVersionInfo) {
        let parts: Vec<&str> = line.splitn(2, '=').collect();
        if parts.len() != 2 {
            return;
        }

        let key = parts[0].trim().to_lowercase();
        let value = parts[1].trim().trim_matches('"').to_string();

        match key.as_str() {
            "driverver" => {
                // Format: MM/DD/YYYY, version or YYYY/MM/DD, version
                let dv_parts: Vec<&str> = value.splitn(2, ',').collect();
                if !dv_parts.is_empty() {
                    version_info.driver_date = Some(dv_parts[0].trim().to_string());
                }
                if dv_parts.len() > 1 {
                    version_info.driver_version = Some(dv_parts[1].trim().to_string());
                }
            }
            "class" => version_info.class = Some(value),
            "classguid" => version_info.class_guid = Some(value),
            "provider" => version_info.provider = Some(value),
            "catalogfile" | "catalogfile.nt" | "catalogfile.ntamd64" | "catalogfile.ntx86" => {
                version_info.catalog_file = Some(value);
            }
            _ => {}
        }
    }

    fn parse_manufacturer_line(line: &str, manufacturers: &mut HashMap<String, String>) {
        let parts: Vec<&str> = line.splitn(2, '=').collect();
        if parts.len() != 2 {
            return;
        }

        let name = parts[0].trim().to_string();
        let section = parts[1].trim().to_string();
        manufacturers.insert(name, section);
    }

    fn parse_device_line(line: &str, section: &str, device_sections: &mut HashMap<String, Vec<(String, String)>>) {
        let parts: Vec<&str> = line.splitn(2, '=').collect();
        if parts.len() != 2 {
            return;
        }

        let device_desc = parts[0].trim().to_string();
        let right_side = parts[1].trim();
        
        // Format: InstallSection, HardwareID [, CompatibleID, ...]
        let hw_parts: Vec<&str> = right_side.split(',').collect();
        if hw_parts.len() >= 2 {
            let hardware_id = hw_parts[1].trim().to_string();
            if !hardware_id.is_empty() && (
                hardware_id.to_uppercase().starts_with("PCI\\") ||
                hardware_id.to_uppercase().starts_with("USB\\") ||
                hardware_id.to_uppercase().starts_with("HDAUDIO\\") ||
                hardware_id.to_uppercase().starts_with("ACPI\\") ||
                hardware_id.to_uppercase().starts_with("HID\\") ||
                hardware_id.to_uppercase().starts_with("SWD\\") ||
                hardware_id.to_uppercase().starts_with("ROOT\\") ||
                hardware_id.to_uppercase().contains("VEN_") ||
                hardware_id.to_uppercase().contains("DEV_")
            ) {
                device_sections
                    .entry(section.to_string())
                    .or_default()
                    .push((device_desc, hardware_id));
            }
        }
    }

    fn parse_strings_line(line: &str, string_table: &mut HashMap<String, String>) {
        let parts: Vec<&str> = line.splitn(2, '=').collect();
        if parts.len() != 2 {
            return;
        }

        let key = parts[0].trim().to_string();
        let value = parts[1].trim().trim_matches('"').to_string();
        string_table.insert(key, value);
    }

    fn resolve_string(s: &str, string_table: &HashMap<String, String>) -> String {
        if s.starts_with('%') && s.ends_with('%') && s.len() > 2 {
            let key = &s[1..s.len()-1];
            string_table.get(key).cloned().unwrap_or_else(|| s.to_string())
        } else {
            s.to_string()
        }
    }

    /// Display parsed driver information
    fn display_results(parsed_files: &[ParsedInfFile], verbose: bool) {
        println!("\n========================================");
        println!("       Driver Package Inspection");
        println!("========================================\n");

        let total_drivers: usize = parsed_files.iter().map(|f| f.drivers.len()).sum();
        println!("Found {} INF files with {} device entries\n", parsed_files.len(), total_drivers);

        for parsed in parsed_files {
            println!("----------------------------------------");
            println!("INF File: {}", parsed.file_name);
            println!("Path: {}", parsed.file_path.display());
            
            if let Some(ref class) = parsed.raw_version_info.class {
                println!("Device Class: {}", class);
            }
            if let Some(ref guid) = parsed.raw_version_info.class_guid {
                println!("Class GUID: {}", guid);
            }
            if let Some(ref version) = parsed.raw_version_info.driver_version {
                println!("Driver Version: {}", version);
            }
            if let Some(ref date) = parsed.raw_version_info.driver_date {
                println!("Driver Date: {}", date);
            }
            if let Some(ref provider) = parsed.raw_version_info.provider {
                println!("Provider: {}", provider);
            }
            if let Some(ref catalog) = parsed.raw_version_info.catalog_file {
                println!("Catalog File: {}", catalog);
            }

            if !parsed.drivers.is_empty() {
                println!("\nSupported Devices ({}):", parsed.drivers.len());
                for (idx, driver) in parsed.drivers.iter().enumerate() {
                    println!("\n  {}. {}", idx + 1, driver.device_name.as_deref().unwrap_or("Unknown"));
                    println!("     Hardware ID: {}", driver.hardware_id.as_deref().unwrap_or("Unknown"));
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
            } else {
                println!("\nNo device entries found in this INF file.");
            }
            println!();
        }
    }

    /// Export results to CSV
    fn export_to_csv(parsed_files: &[ParsedInfFile], output_path: &Path) -> Result<()> {
        let mut csv_content = String::new();
        
        // CSV Header matching PnPSignedDriver structure
        csv_content.push_str("Device Name,Driver Version,Driver Date,Hardware ID,INF Name,Description,Provider,Device Class,Class GUID,Catalog File,Manufacturer\n");
        
        let escape_csv = |s: &str| -> String {
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace("\"", "\"\""))
            } else {
                s.to_string()
            }
        };

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

        fs::write(output_path, csv_content)
            .with_context(|| format!("Failed to write CSV file: {}", output_path.display()))?;

        println!("Exported to: {}", output_path.display());
        Ok(())
    }

    /// Main inspect function
    fn inspect(path: &Path, output: Option<&Path>, verbose: bool) -> Result<()> {
        println!("Inspecting driver package: {}", path.display());

        // Extract or use path directly
        let (work_dir, needs_cleanup) = Self::extract_or_use_path(path, verbose)?;

        // Find all INF files
        let inf_files = Self::find_inf_files(&work_dir)?;

        if inf_files.is_empty() {
            if needs_cleanup {
                let _ = fs::remove_dir_all(&work_dir);
            }
            anyhow::bail!("No INF files found in the specified path");
        }

        if verbose {
            println!("Found {} INF files", inf_files.len());
        }

        // Parse all INF files
        let mut parsed_files = Vec::new();
        for inf_path in &inf_files {
            match Self::parse_inf_file(inf_path) {
                Ok(parsed) => parsed_files.push(parsed),
                Err(e) => {
                    if verbose {
                        eprintln!("Warning: Failed to parse {}: {}", inf_path.display(), e);
                    }
                }
            }
        }

        // Display results
        Self::display_results(&parsed_files, verbose);

        // Export to CSV if requested
        if let Some(csv_path) = output {
            Self::export_to_csv(&parsed_files, csv_path)?;
        }

        // Cleanup temp directory if needed
        if needs_cleanup {
            if verbose {
                println!("Cleaning up temporary files...");
            }
            let _ = fs::remove_dir_all(&work_dir);
        }

        Ok(())
    }

    /// Scan folder and display INF summary
    fn scan_folder(path: &Path, output: Option<&Path>, verbose: bool, group_by_class: bool, recursive: bool) -> Result<()> {
        if !path.is_dir() {
            anyhow::bail!("Path must be a directory: {}", path.display());
        }

        println!("Scanning folder: {}", path.display());
        if recursive {
            println!("Mode: Recursive (including subfolders)");
        }
        println!();

        // Find all INF files
        let inf_files = if recursive {
            Self::find_inf_files(path)?
        } else {
            Self::find_inf_files_in_folder(path)?
        };

        if inf_files.is_empty() {
            println!("No INF files found.");
            return Ok(());
        }

        // Parse all INF files
        let mut parsed_files: Vec<ParsedInfFile> = Vec::new();
        let mut parse_errors: Vec<(PathBuf, String)> = Vec::new();

        for inf_path in &inf_files {
            match Self::parse_inf_file(inf_path) {
                Ok(parsed) => parsed_files.push(parsed),
                Err(e) => parse_errors.push((inf_path.clone(), e.to_string())),
            }
        }

        // Display summary
        println!("========================================");
        println!("         INF Folder Scan Results");
        println!("========================================");
        println!();
        println!("Folder: {}", path.display());
        println!("Total INF files found: {}", inf_files.len());
        println!("Successfully parsed: {}", parsed_files.len());
        if !parse_errors.is_empty() {
            println!("Failed to parse: {}", parse_errors.len());
        }
        
        let total_devices: usize = parsed_files.iter().map(|f| f.drivers.len()).sum();
        println!("Total device entries: {}", total_devices);
        println!();

        if group_by_class {
            Self::display_scan_grouped(&parsed_files, verbose);
        } else {
            Self::display_scan_list(&parsed_files, verbose);
        }

        // Show parse errors if verbose
        if verbose && !parse_errors.is_empty() {
            println!("\n----------------------------------------");
            println!("Parse Errors:");
            for (path, error) in &parse_errors {
                println!("  - {}: {}", path.file_name().unwrap_or_default().to_string_lossy(), error);
            }
        }

        // Export to CSV if requested
        if let Some(csv_path) = output {
            Self::export_scan_csv(&parsed_files, csv_path)?;
        }

        Ok(())
    }

    /// Display scan results as a simple list
    fn display_scan_list(parsed_files: &[ParsedInfFile], verbose: bool) {
        println!("----------------------------------------");
        println!("INF Files Summary:");
        println!("----------------------------------------");
        
        for (idx, parsed) in parsed_files.iter().enumerate() {
            println!("\n{}. {}", idx + 1, parsed.file_name);
            
            if let Some(ref class) = parsed.raw_version_info.class {
                println!("   Class: {}", class);
            }
            if let Some(ref version) = parsed.raw_version_info.driver_version {
                println!("   Version: {}", version);
            }
            if let Some(ref date) = parsed.raw_version_info.driver_date {
                println!("   Date: {}", date);
            }
            if let Some(ref provider) = parsed.raw_version_info.provider {
                // Resolve provider string if it's a reference
                let provider_display = if provider.starts_with('%') && provider.ends_with('%') {
                    // Try to find in first driver's manufacturer or use as-is
                    parsed.drivers.first()
                        .and_then(|d| d.driver_provider_name.as_ref())
                        .map(|s| s.as_str())
                        .unwrap_or(provider)
                } else {
                    provider
                };
                println!("   Provider: {}", provider_display);
            }
            println!("   Devices: {} entries", parsed.drivers.len());

            if verbose && !parsed.drivers.is_empty() {
                println!("   Hardware IDs:");
                for driver in &parsed.drivers {
                    if let Some(ref hwid) = driver.hardware_id {
                        let device_name = driver.device_name.as_deref().unwrap_or("Unknown");
                        println!("     - {} ({})", hwid, device_name);
                    }
                }
            }
        }
    }

    /// Display scan results grouped by device class
    fn display_scan_grouped(parsed_files: &[ParsedInfFile], verbose: bool) {
        // Group by device class
        let mut by_class: HashMap<String, Vec<&ParsedInfFile>> = HashMap::new();
        
        for parsed in parsed_files {
            let class = parsed.raw_version_info.class
                .as_deref()
                .unwrap_or("Unknown")
                .to_string();
            by_class.entry(class).or_default().push(parsed);
        }

        // Sort classes
        let mut classes: Vec<_> = by_class.keys().cloned().collect();
        classes.sort();

        println!("----------------------------------------");
        println!("INF Files by Device Class:");
        println!("----------------------------------------");

        for class in classes {
            if let Some(files) = by_class.get(&class) {
                println!("\n[{}] ({} INF files)", class, files.len());
                
                for parsed in files {
                    let version = parsed.raw_version_info.driver_version
                        .as_deref()
                        .unwrap_or("?");
                    let devices = parsed.drivers.len();
                    
                    println!("  - {} (v{}, {} devices)", parsed.file_name, version, devices);
                    
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

    /// Export scan results to CSV
    fn export_scan_csv(parsed_files: &[ParsedInfFile], output_path: &Path) -> Result<()> {
        let mut csv_content = String::new();
        
        // CSV Header - summary format with device names
        csv_content.push_str("INF File,Device Class,Provider,Driver Version,Driver Date,Device Count,Device Names,Hardware IDs\n");
        
        let escape_csv = |s: &str| -> String {
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.to_string()
            }
        };

        for parsed in parsed_files {
            // Collect device names
            let device_names: Vec<String> = parsed.drivers
                .iter()
                .filter_map(|d| d.device_name.clone())
                .collect();
            let device_names_str = device_names.join("; ");

            // Collect hardware IDs
            let hwids: Vec<String> = parsed.drivers
                .iter()
                .filter_map(|d| d.hardware_id.clone())
                .collect();
            let hwids_str = hwids.join("; ");

            // Resolve provider - try to get from parsed drivers first
            let provider = parsed.raw_version_info.provider.as_deref().unwrap_or("Unknown");
            let resolved_provider = if provider.starts_with('%') && provider.ends_with('%') {
                // Get resolved provider from first driver
                parsed.drivers.first()
                    .and_then(|d| d.driver_provider_name.as_deref())
                    .unwrap_or(provider)
            } else {
                provider
            };

            csv_content.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                escape_csv(&parsed.file_name),
                escape_csv(parsed.raw_version_info.class.as_deref().unwrap_or("Unknown")),
                escape_csv(resolved_provider),
                escape_csv(parsed.raw_version_info.driver_version.as_deref().unwrap_or("Unknown")),
                escape_csv(parsed.raw_version_info.driver_date.as_deref().unwrap_or("Unknown")),
                parsed.drivers.len(),
                escape_csv(&device_names_str),
                escape_csv(&hwids_str),
            ));
        }

        fs::write(output_path, csv_content)
            .with_context(|| format!("Failed to write CSV file: {}", output_path.display()))?;

        println!("\nExported to: {}", output_path.display());
        Ok(())
    }

    /// Scan backup folder recursively and export summary CSV (used by backup command)
    fn scan_and_export(backup_dir: &Path, output_csv: &Path, verbose: bool) -> Result<()> {
        // Find all INF files recursively in the backup folder
        let inf_files = Self::find_inf_files(backup_dir)?;

        if inf_files.is_empty() {
            println!("No INF files found in backup folder.");
            return Ok(());
        }

        if verbose {
            println!("Found {} INF files in backup", inf_files.len());
        }

        // Parse all INF files
        let mut parsed_files: Vec<ParsedInfFile> = Vec::new();
        for inf_path in &inf_files {
            match Self::parse_inf_file(inf_path) {
                Ok(parsed) => parsed_files.push(parsed),
                Err(e) => {
                    if verbose {
                        eprintln!("Warning: Failed to parse {}: {}", inf_path.display(), e);
                    }
                }
            }
        }

        if parsed_files.is_empty() {
            println!("No valid INF files parsed.");
            return Ok(());
        }

        // Export to CSV with folder name
        Self::export_backup_summary_csv(&parsed_files, backup_dir, output_csv)?;

        println!("Summary CSV created: {}", output_csv.display());
        println!("Total INF files: {}", parsed_files.len());
        
        let total_devices: usize = parsed_files.iter().map(|f| f.drivers.len()).sum();
        println!("Total device entries: {}", total_devices);

        Ok(())
    }

    /// Export backup summary to CSV with relative folder paths
    fn export_backup_summary_csv(parsed_files: &[ParsedInfFile], backup_dir: &Path, output_path: &Path) -> Result<()> {
        let mut csv_content = String::new();
        
        // CSV Header - includes Folder Name for backup
        csv_content.push_str("INF File,Device Class,Provider,Driver Version,Driver Date,Device Count,Folder Name,Device Names,Hardware IDs\n");
        
        let escape_csv = |s: &str| -> String {
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.to_string()
            }
        };

        for parsed in parsed_files {
            // Collect device names
            let device_names: Vec<String> = parsed.drivers
                .iter()
                .filter_map(|d| d.device_name.clone())
                .collect();
            let device_names_str = device_names.join("; ");

            // Collect hardware IDs
            let hwids: Vec<String> = parsed.drivers
                .iter()
                .filter_map(|d| d.hardware_id.clone())
                .collect();
            let hwids_str = hwids.join("; ");

            // Resolve provider
            let provider = parsed.raw_version_info.provider.as_deref().unwrap_or("Unknown");
            let resolved_provider = if provider.starts_with('%') && provider.ends_with('%') {
                parsed.drivers.first()
                    .and_then(|d| d.driver_provider_name.as_deref())
                    .unwrap_or(provider)
            } else {
                provider
            };

            // Get relative folder path from backup_dir
            let folder_name = parsed.file_path.parent()
                .and_then(|p| p.strip_prefix(backup_dir).ok())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".to_string());

            csv_content.push_str(&format!(
                "{},{},{},{},{},{},{},{},{}\n",
                escape_csv(&parsed.file_name),
                escape_csv(parsed.raw_version_info.class.as_deref().unwrap_or("Unknown")),
                escape_csv(resolved_provider),
                escape_csv(parsed.raw_version_info.driver_version.as_deref().unwrap_or("Unknown")),
                escape_csv(parsed.raw_version_info.driver_date.as_deref().unwrap_or("Unknown")),
                parsed.drivers.len(),
                escape_csv(&folder_name),
                escape_csv(&device_names_str),
                escape_csv(&hwids_str),
            ));
        }

        fs::write(output_path, csv_content)
            .with_context(|| format!("Failed to write CSV file: {}", output_path.display()))?;

        Ok(())
    }
}

// Add CLI arguments for backup functionality
#[derive(Parser)]
#[command(name = "driver-backup")]
#[command(version = "2.3")]
#[command(about = "A tool to backup, inspect, and manage non-Microsoft drivers")]
#[command(long_about = "Driver Backup Tool v2.3\n\n\
    Commands:\n  \
    backup   - Export all non-Microsoft drivers from the system (requires Admin)\n  \
    inspect  - Extract driver info from installer packages (.exe, .zip, .7z, folder)\n  \
    scan     - Identify and list all INF files in a folder\n\n\
    Examples:\n  \
    driver-backup backup -o D:\\Backup -v\n  \
    driver-backup inspect -p C:\\Downloads\\driver.exe -o info.csv\n  \
    driver-backup scan -p C:\\Drivers -r -g -o inventory.csv")]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
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
        Commands::Inspect { path, output, verbose } => {
            if verbose {
                println!("Driver Package Inspector");
                println!("========================");
                println!("Input path: {}", path.display());
                if let Some(ref out) = output {
                    println!("Output CSV: {}", out.display());
                }
                println!();
            }

            // Run the inspect process
            InfParser::inspect(&path, output.as_deref(), verbose)?;
        }
        Commands::Scan { path, output, verbose, group, recursive } => {
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

            // Run the scan process
            InfParser::scan_folder(&path, output.as_deref(), verbose, group, recursive)?;
        }
        Commands::Export { output, all, verbose, files } => {
            println!("Hardware Inventory Export");
            println!("=========================");
            
            // Query WMI for connected devices
            let com_con = COMLibrary::new().context("Failed to initialize COM library")?;
            let wmi_con = WMIConnection::new(com_con.into()).context("Failed to create WMI connection")?;
            
            let drivers: Vec<PnPSignedDriver> = wmi_con.query()
                .context("Failed to query WMI for PnP signed drivers")?;
            
            // Filter Microsoft drivers unless --all is specified
            let filtered_drivers: Vec<PnPSignedDriver> = if all {
                drivers
            } else {
                drivers.into_iter()
                    .filter(|d| {
                        d.driver_provider_name.as_ref()
                            .map(|p| !p.to_lowercase().contains("microsoft"))
                            .unwrap_or(true)
                    })
                    .collect()
            };
            
            println!("Found {} connected devices", filtered_drivers.len());

            // Export driver files if --files flag is set
            if files {
                let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
                let backup_dir = if output.extension().map(|e| e == "csv").unwrap_or(false) {
                    output.parent().unwrap_or(Path::new(".")).join(format!("drivers_{}", timestamp))
                } else {
                    output.join(format!("drivers_{}", timestamp))
                };
                
                fs::create_dir_all(&backup_dir)
                    .with_context(|| format!("Failed to create backup directory: {}", backup_dir.display()))?;

                println!("\nExporting driver files to: {}", backup_dir.display());

                // Group drivers by INF and export
                let mut exported_infs: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut success_count = 0;
                let mut fail_count = 0;

                for driver in &filtered_drivers {
                    if let Some(inf_name) = &driver.inf_name {
                        let inf_lower = inf_name.to_lowercase();
                        if inf_lower.starts_with("oem") && !exported_infs.contains(&inf_lower) {
                            exported_infs.insert(inf_lower.clone());

                            // Create folder for this driver
                            let device_class = driver.device_class.as_deref().unwrap_or("Unknown");
                            let version = driver.driver_version.as_deref().unwrap_or("Unknown");
                            let provider = driver.driver_provider_name.as_deref().unwrap_or("Unknown");
                            
                            let folder_name = format!("{}_{}_{}",device_class, provider, version)
                                .chars()
                                .map(|c| if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
                                .collect::<String>();

                            let driver_dir = backup_dir.join(&folder_name);
                            fs::create_dir_all(&driver_dir).ok();

                            if verbose {
                                println!("  Exporting {} -> {}", inf_name, folder_name);
                            }

                            let status = Command::new("pnputil")
                                .arg("/export-driver")
                                .arg(inf_name)
                                .arg(&driver_dir)
                                .output();

                            match status {
                                Ok(result) if result.status.success() => {
                                    success_count += 1;
                                }
                                _ => {
                                    fail_count += 1;
                                    if verbose {
                                        eprintln!("    Failed to export {}", inf_name);
                                    }
                                }
                            }
                        }
                    }
                }

                println!("Driver files exported: {} success, {} failed", success_count, fail_count);

                // Create CSV in backup directory
                let csv_path = backup_dir.join("all_drivers.csv");
                DriverBackup::export_wmi_drivers_csv_static(&filtered_drivers, &csv_path, verbose)?;
                
                println!("\nBackup location: {}", backup_dir.display());
            } else {
                // Just export CSV
                DriverBackup::export_wmi_drivers_csv_static(&filtered_drivers, &output, verbose)?;
                println!("\nExported to: {}", output.display());
            }
        }
    }

    // Add pause before closing
    println!("\nPress Enter to close...");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).expect("Failed to read line");

    Ok(())
}
