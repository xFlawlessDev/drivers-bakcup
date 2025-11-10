# Driver Backup Tool - Usage Guide

## Quick Start

### 1. Run as Administrator

**Important**: This tool MUST be run with administrator privileges.

Right-click on `driver-backup.exe` → Select "Run as Administrator"

### 2. Basic Backup

```powershell
# Simple backup to default folder
.\driver-backup.exe
```

This will:

- Create a folder named `driver_backup/drivers_YYYYMMDD_HHMMSS/`
- Export all non-Microsoft drivers
- Group them by device name and version
- Create CSV files for database import

### 3. Custom Output Location

```powershell
.\driver-backup.exe backup --output "D:\DriverBackups"
```

### 4. Verbose Mode (Recommended for First Run)

```powershell
.\driver-backup.exe backup --verbose
```

Shows detailed progress:

- Which drivers are being processed
- Export status for each driver
- CSV file creation
- Any errors or warnings

### 5. Dry Run (Preview Only)

```powershell
.\driver-backup.exe backup --dry-run --verbose
```

Perfect for:

- Checking which drivers will be backed up
- Estimating backup size
- Testing without actually exporting

## Understanding the Output

### Folder Structure

After running, you'll see:

```
driver_backup/
└── drivers_20250110_143025/
    ├── Display/
    │   ├── NVIDIA GeForce RTX 3080_30.0.15.1179 Package/
    │   │   ├── driver_info.csv
    │   │   ├── oem1.inf
    │   │   └── [other driver files]
    │   └── AMD Radeon RX 6800_21.10.2 Package/
    │       ├── driver_info.csv
    │       └── [driver files]
    ├── Net/
    │   └── Intel Ethernet I219-V_12.19.2.45 Package/
    │       ├── driver_info.csv
    │       └── [driver files]
    ├── Media/
    │   └── Realtek High Definition Audio_6.0.9346.1 Package/
    │       ├── driver_info.csv
    │       ├── oem2.inf
    │       └── [other driver files]
    ├── all_drivers.csv
    └── driver_backup_summary.txt
```

### Files Created

#### 1. `driver_info.csv` (in each driver folder)

Contains all details for that specific driver package:

- Device Name
- Driver Version
- Driver Date
- Hardware ID
- Device ID
- INF Name
- Provider, Class, etc.

**Use this for**: Uploading individual driver info to your database

#### 2. `all_drivers.csv` (in root backup folder)

Master CSV with ALL backed-up drivers, includes an extra column for the folder name.

**Use this for**:

- Getting an overview of all drivers
- Bulk database import
- Generating reports

#### 3. `driver_backup_summary.txt`

Human-readable summary with:

- Total drivers backed up
- Grouped by device name
- All hardware IDs and device IDs
- Folder locations

**Use this for**: Quick review and documentation

## Database Upload Workflow

### Option 1: Individual Driver Upload

1. Navigate to a driver folder (e.g., `Display/NVIDIA GeForce RTX 3080_30.0.15.1179 Package/`)
2. Open `driver_info.csv`
3. Import to your database table
4. Zip the folder and upload to file storage
5. Link the database record to the zip file

### Option 2: Bulk Upload

1. Open `all_drivers.csv` from the root backup folder
2. Import all records to your database in one operation
3. Use the "Folder Name" column to locate and zip each driver folder
4. Create file storage links in bulk

## CSV Format

```csv
Device Name,Driver Version,Driver Date,Hardware ID,Device ID,INF Name,Description,Provider,Device Class,Class GUID
NVIDIA GeForce RTX 3080,30.0.15.1179,2023-08-15,PCI\VEN_10DE&DEV_2206,PCI\VEN_10DE...,oem1.inf,NVIDIA Graphics,NVIDIA,Display,{4d36e968...}
```

### Fields Explained

| Field          | Description             | Example                   |
| -------------- | ----------------------- | ------------------------- |
| Device Name    | Display name of device  | "NVIDIA GeForce RTX 3080" |
| Driver Version | Version number          | "30.0.15.1179"            |
| Driver Date    | When driver was created | "2023-08-15"              |
| Hardware ID    | Hardware identifier     | "PCI\VEN_10DE&DEV_2206"   |
| Device ID      | Unique device ID        | Full PCI path             |
| INF Name       | INF file name           | "oem1.inf"                |
| Description    | Driver description      | "NVIDIA Graphics Driver"  |
| Provider       | Manufacturer            | "NVIDIA"                  |
| Device Class   | Type of device          | "Display"                 |
| Class GUID     | Device class GUID       | "{4d36e968-...}"          |

## Advanced Usage

### Filtering in Database

After import, you can filter by:

- **Device Class**: Find all "Display" drivers
- **Provider**: List drivers by manufacturer
- **Driver Date**: Find outdated drivers
- **Device Name**: Track specific devices

### Creating a Driver Repository

1. Run backup tool regularly (weekly/monthly)
2. Import new CSV data to database
3. Track driver version changes over time
4. Maintain historical records

### Example Database Schema

```sql
CREATE TABLE drivers (
    id INTEGER PRIMARY KEY,
    device_name VARCHAR(255),
    driver_version VARCHAR(50),
    driver_date DATE,
    hardware_id VARCHAR(255),
    device_id VARCHAR(255),
    inf_name VARCHAR(100),
    description TEXT,
    provider VARCHAR(100),
    device_class VARCHAR(50),
    class_guid VARCHAR(50),
    backup_folder VARCHAR(255),
    backup_date TIMESTAMP,
    zip_file_path VARCHAR(500)
);
```

## Troubleshooting

### Error: "This program requires administrative privileges"

**Solution**: Right-click executable → Run as Administrator

### Error: "Failed to export driver"

**Cause**: Driver may be:

- Protected by Windows
- Corrupted
- Already removed

**Action**: Check verbose output for specific error. These can usually be ignored.

### Warning: "Path too long"

**Solution**: Use shorter output path:

```powershell
.\driver-backup.exe backup --output "C:\Backup"
```

### No drivers found

**Possible reasons**:

- Only Microsoft drivers installed (unlikely)
- WMI service not running
- Insufficient permissions

**Check**: Run with `--verbose` flag for details

## Best Practices

1. **Run as Admin**: Always required
2. **Use Verbose Mode**: Helps identify issues
3. **Regular Backups**: Schedule monthly backups
4. **Version Control**: Keep historical CSV files
5. **Test Restore**: Verify backup completeness
6. **Database Indexing**: Index on device_name, provider, driver_date

## Example Workflow for IT Admin

```powershell
# 1. Run backup with verbose logging
.\driver-backup.exe backup --output "\\server\drivers\backups" --verbose

# 2. Review summary file
notepad "\\server\drivers\backups\drivers_*\driver_backup_summary.txt"

# 3. Import master CSV to database
# (Use your database import tool)

# 4. Zip each driver folder for storage
# (Can be automated with PowerShell script)

# 5. Update database with zip file paths
# (SQL UPDATE statements)
```

## Automation Script Example

```powershell
# automated_backup.ps1
$backupPath = "D:\DriverBackups"
$timestamp = Get-Date -Format "yyyyMMdd_HHmmss"

# Run backup
& ".\driver-backup.exe" backup --output $backupPath --verbose

# Find the latest backup folder
$latestBackup = Get-ChildItem $backupPath | Sort-Object LastWriteTime -Descending | Select-Object -First 1

# Import CSV to database
# Your database import logic here

# Zip driver folders
Get-ChildItem $latestBackup.FullName -Directory | ForEach-Object {
    $zipPath = "$($_.FullName).zip"
    Compress-Archive -Path $_.FullName -DestinationPath $zipPath
}

Write-Host "Backup completed: $($latestBackup.FullName)"
```

## Support

For issues or questions:

- Check verbose output first
- Review error messages in summary file
- Ensure administrative privileges
- Verify WMI service is running

---

**Remember**: Always run as Administrator! 🛡️
