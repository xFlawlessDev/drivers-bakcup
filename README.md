# Driver Backup Tool

A Rust-based tool to backup, inspect, and manage non-Microsoft drivers on Windows systems.

## Features

- **Backup**: Export third-party drivers with organized folder structure
- **Inspect**: Extract driver information from installer packages (.exe, .zip, .7z)
- **Scan**: Identify and list all INF files in a folder with detailed summary

## Commands Overview

```
driver-backup.exe <COMMAND>

Commands:
  backup   Backup drivers to specified directory
  inspect  Inspect driver package (exe/zip/folder) to extract driver information
  scan     Scan a folder to identify and list all INF files with summary
```

---

## 1. Backup Command

Export all non-Microsoft drivers from the system.

**Requires Administrator privileges.**

### Usage

```powershell
# Basic backup (default: driver_backup folder)
.\driver-backup.exe backup

# Custom output directory
.\driver-backup.exe backup -o "D:\MyBackups"

# With verbose logging
.\driver-backup.exe backup -o "D:\MyBackups" -v

# Preview without actually exporting (dry run)
.\driver-backup.exe backup -d -v
```

### Options

| Option | Long | Description |
|--------|------|-------------|
| `-o` | `--output <PATH>` | Output directory (default: `driver_backup`) |
| `-v` | `--verbose` | Enable verbose output |
| `-d` | `--dry-run` | Preview operations without executing |

### Output Structure

```
driver_backup/
└── drivers_YYYYMMDD_HHMMSS/
    ├── Display/
    │   └── NVIDIA GeForce RTX 3080_30.0.15.1179 Package/
    │       ├── driver_info.csv
    │       └── [exported driver files]
    ├── Net/
    │   └── Intel Ethernet I219-V_12.19.2.45 Package/
    │       ├── driver_info.csv
    │       └── [exported driver files]
    ├── all_drivers.csv
    └── driver_backup_summary.txt
```

---

## 2. Inspect Command

Extract driver information from installer packages or folders. Useful for analyzing driver packages before installation.

### Usage

```powershell
# Inspect a folder containing INF files
.\driver-backup.exe inspect -p "C:\Downloads\DriverPackage"

# Inspect an installer (.exe, .zip, .7z, .rar)
.\driver-backup.exe inspect -p "C:\Downloads\Intel_Graphics_Driver.exe"

# Inspect and export to CSV
.\driver-backup.exe inspect -p "C:\Downloads\DriverPackage" -o "drivers.csv"

# With verbose output
.\driver-backup.exe inspect -p "C:\Downloads\DriverPackage" -v
```

### Options

| Option | Long | Description |
|--------|------|-------------|
| `-p` | `--path <PATH>` | Path to driver installer or folder (required) |
| `-o` | `--output <CSV>` | Output CSV file path (optional) |
| `-v` | `--verbose` | Show detailed output |

### Supported Formats

- **Folders**: Directly scan for INF files
- **Archives**: `.exe`, `.zip`, `.7z`, `.rar` (requires 7-Zip for non-zip formats)
- **Single INF**: Direct INF file path

### Output Example

```
========================================
       Driver Package Inspection
========================================

Found 3 INF files with 45 device entries

----------------------------------------
INF File: igdlh64.inf
Path: C:\Downloads\Intel\Graphics\igdlh64.inf
Device Class: Display
Class GUID: {4d36e968-e325-11ce-bfc1-08002be10318}
Driver Version: 31.0.101.5590
Driver Date: 12/01/2024
Provider: Intel Corporation

Supported Devices (15):
  1. Intel(R) UHD Graphics 630
     Hardware ID: PCI\VEN_8086&DEV_3E92
  2. Intel(R) UHD Graphics 620
     Hardware ID: PCI\VEN_8086&DEV_5917
  ...
```

---

## 3. Scan Command

Scan a folder to identify and list all INF files with summary information.

### Usage

```powershell
# Scan current folder only
.\driver-backup.exe scan -p "C:\Drivers"

# Scan including all subfolders (recursive)
.\driver-backup.exe scan -p "C:\Drivers" -r

# Group results by device class
.\driver-backup.exe scan -p "C:\Drivers" -g

# Export results to CSV
.\driver-backup.exe scan -p "C:\Drivers" -o "scan_results.csv"

# Full options: recursive, verbose, grouped, with CSV export
.\driver-backup.exe scan -p "C:\Drivers" -r -v -g -o "scan_results.csv"
```

### Options

| Option | Long | Description |
|--------|------|-------------|
| `-p` | `--path <PATH>` | Path to folder (required) |
| `-o` | `--output <CSV>` | Output CSV file path (optional) |
| `-v` | `--verbose` | Show detailed info including all Hardware IDs |
| `-g` | `--group` | Group results by device class |
| `-r` | `--recursive` | Scan subfolders recursively |

### Output Example (List Mode)

```
========================================
         INF Folder Scan Results
========================================

Folder: C:\Drivers
Total INF files found: 5
Successfully parsed: 5
Total device entries: 128

----------------------------------------
INF Files Summary:
----------------------------------------

1. igdlh64.inf
   Class: Display
   Version: 31.0.101.5590
   Date: 12/01/2024
   Provider: Intel Corporation
   Devices: 45 entries

2. e1d65x64.inf
   Class: Net
   Version: 12.19.2.45
   Date: 10/15/2024
   Provider: Intel Corporation
   Devices: 12 entries
```

### Output Example (Grouped Mode with `-g`)

```
----------------------------------------
INF Files by Device Class:
----------------------------------------

[Display] (2 INF files)
  - igdlh64.inf (v31.0.101.5590, 45 devices)
  - nvdmi.inf (v546.33, 38 devices)

[Net] (2 INF files)
  - e1d65x64.inf (v12.19.2.45, 12 devices)
  - rt640x64.inf (v10.050.1021.2021, 8 devices)

[Media] (1 INF files)
  - hdxrt.inf (v6.0.9346.1, 25 devices)
```

### CSV Export Format

```csv
INF File,Device Class,Provider,Driver Version,Driver Date,Device Count,Device Names,Hardware IDs
igdlh64.inf,Display,Intel Corporation,31.0.101.5590,12/01/2024,45,"Intel UHD 630; Intel UHD 620","PCI\VEN_8086&DEV_3E92; PCI\VEN_8086&DEV_5917"
```

---

## Driver Information Captured

All commands capture the following information from INF files:

| Field | Description |
|-------|-------------|
| Device Name | Friendly name of the device |
| Driver Version | Version number (e.g., 31.0.101.5590) |
| Driver Date | Release date |
| Hardware ID | PCI\VEN_xxxx&DEV_xxxx format |
| Device Class | Display, Net, Media, etc. |
| Class GUID | Windows device class GUID |
| Provider | Driver publisher (Intel, NVIDIA, etc.) |
| INF Name | INF file name |
| Manufacturer | Device manufacturer |

---

## Requirements

- **Windows 10/11** (Server 2016+ supported)
- **Administrator Rights** - Required for `backup` command
- **7-Zip** (optional) - For extracting `.exe`, `.7z`, `.rar` in `inspect` command

## Installation

### From Release
Download the latest release and run `driver-backup.exe`.

### Build from Source
```powershell
# Clone and build
git clone <repository>
cd driver-backup
cargo build --release

# Binary located at target/release/driver-backup.exe
```

---

## Examples

### Backup all third-party drivers
```powershell
.\driver-backup.exe backup -o "D:\DriverBackup" -v
```

### Analyze a downloaded driver package
```powershell
.\driver-backup.exe inspect -p "C:\Downloads\NVIDIA_Driver_546.33.exe" -o "nvidia_info.csv"
```

### Find all drivers in a folder tree
```powershell
.\driver-backup.exe scan -p "D:\DriverRepository" -r -g -o "inventory.csv"
```

### Preview backup without exporting
```powershell
.\driver-backup.exe backup -d -v
```

---

## Troubleshooting

| Issue | Solution |
|-------|----------|
| "Admin privileges required" | Run as Administrator (backup command) |
| "Failed to export driver" | Driver may be protected; check verbose output |
| "7-Zip not found" | Install 7-Zip or use .zip format for inspect |
| "No INF files found" | Check path; use `-r` for recursive scan |
| "Path too long" | Use shorter output path |

---

## Version History

- **v2.3** - Added `scan` and `inspect` commands with recursive support
- **v2.2** - Database-ready CSV export, improved folder structure
- **v2.1** - INF-based grouping, device class organization
- **v2.0** - Initial release with backup functionality

---

**Version**: 2.3  
**Last Updated**: 2025-12-09  
**Compatible**: Windows 10/11, Server 2016+
