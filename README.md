# Driver Backup Tool

A Rust-based tool to backup and manage non-Microsoft drivers on Windows systems.

## Features

- 🔍 **Automatic Driver Detection**: Queries WMI for all installed PnP signed drivers
- 🚫 **Smart Filtering**: Excludes Microsoft drivers, backing up only third-party drivers
- 📁 **Organized by Device Class**: Groups drivers by type (Display, Net, Media, etc.)
- 📦 **INF-Based Packaging**: Groups devices sharing the same driver package
- 📊 **Database-Ready**: Generates CSV files for easy database import
- 📝 **Comprehensive Logging**: Detailed summary files and per-driver information
- 🔧 **Dry Run Mode**: Preview without actually exporting
- ⏰ **Timestamped Backups**: Unique timestamp for each backup session

## Driver Information Captured

- Device Name, Driver Version, Driver Date
- Hardware ID, Device ID, INF Name
- Description, Provider, Device Class, Class GUID

## Folder Structure

The backup creates the following structure:

```
driver_backup/
└── drivers_YYYYMMDD_HHMMSS/
    ├── Display/
    │   ├── NVIDIA GeForce RTX 3080_30.0.15.1179 Package/
    │   │   ├── driver_info.csv
    │   │   └── [exported driver files]
    │   └── AMD Radeon RX 6800_21.10.2 Package/
    │       ├── driver_info.csv
    │       └── [exported driver files]
    ├── Net/
    │   ├── Intel Ethernet I219-V_12.19.2.45 Package/
    │   │   ├── driver_info.csv
    │   │   └── [exported driver files]
    │   └── Realtek PCIe GbE_10.050.1021.2021 Package/
    │       ├── driver_info.csv
    │       └── [exported driver files]
    ├── Media/
    │   └── Realtek High Definition Audio_6.0.9346.1 Package/
    │       ├── driver_info.csv
    │       └── [exported driver files]
    ├── all_drivers.csv              # Master CSV with all drivers
    └── driver_backup_summary.txt    # Human-readable summary
```

### Key Features:

- **Device Class Organization**: Display, Net, Media folders mirror Windows Device Manager
- **Version in Folder Names**: `DeviceName_Version Package` format for easy identification
- **INF Grouping**: Devices sharing the same driver package combined in one folder
- **CSV per Package**: Each folder has `driver_info.csv` for database upload
- **Master CSV**: `all_drivers.csv` with all drivers and folder paths
- **Organized Summary**: Text summary grouped by device class

### Why This Structure?

**Device Classes**: Similar devices grouped together (Display, Net, Media) for easier navigation and restoration by priority (display first, then network, etc.)

**INF Grouping**: One driver package (INF) often supports multiple devices. Example: One NVIDIA INF covers RTX 3060, 3070, 3080. All are grouped in one folder for efficient backup and restore.

## Usage

**⚠️ Administrator rights required** - Run as Administrator or you'll get an error.

```powershell
# Basic backup
.\driver-backup.exe

# Custom output directory with verbose logging
.\driver-backup.exe backup --output "D:\MyBackups" --verbose

# Preview without backing up
.\driver-backup.exe backup --dry-run --verbose
```

### Command Line Options

```
-o, --output <PATH>    Output directory (default: "driver_backup")
-v, --verbose          Enable verbose output
-d, --dry-run          Preview operations without executing
-h, --help             Print help
```

## CSV File Format

**Columns**: Device Name, Driver Version, Driver Date, Hardware ID, Device ID, INF Name, Description, Provider, Device Class, Class GUID, Folder Name (master CSV only)

Designed for easy import into database systems.

## Requirements & Installation

- **Windows 10/11** (Server 2016+ supported)
- **Administrator Rights** - Required for WMI access and pnputil
- **Rust** - To build from source

```powershell
# Build release version
cargo build --release

# Run directly
cargo run --release -- backup --verbose
```

## How It Works

1. Queries WMI's `Win32_PnPSignedDriver` class
2. Filters out Microsoft-provided drivers
3. Organizes by Device Class (Display, Net, Media, etc.)
4. Groups by INF file (same package → one folder)
5. Exports using `pnputil /export-driver`
6. Creates CSV files and summary for each package

## Troubleshooting

| Issue                       | Solution                                                  |
| --------------------------- | --------------------------------------------------------- |
| "Admin privileges required" | Right-click → Run as Administrator                        |
| "Failed to export driver"   | Check verbose output; driver may be protected/corrupted   |
| "Path too long"             | Use shorter output path (e.g., `C:\Backup`)               |
| "No drivers found"          | Only third-party drivers are backed up; check WMI service |

## Common Use Cases

- **System Migration**: Backup before OS reinstall
- **Driver Management**: Maintain database of third-party drivers
- **IT Support**: Pre-deployment preparation, troubleshooting
- **Compliance**: Audit trail for driver versions

---

**Version**: 2.2  
**Last Updated**: 2025-11-10  
**Compatible**: Windows 10/11, Server 2016+
