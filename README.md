# Driver Backup Tool

A Rust-based utility for backing up non-Microsoft device drivers from Windows systems using Windows Management Instrumentation (WMI) and `pnputil`.

## Overview

This tool scans your Windows system for all installed device drivers, filters out Microsoft drivers, and exports remaining third-party drivers to organized backup folders with device-specific naming. It creates a comprehensive summary of all exported drivers for easy reference and management.

## Features

- 🔍 **Automatic Driver Discovery**: Queries WMI to find all installed PnP drivers
- 🚫 **Microsoft Driver Filtering**: Excludes Microsoft and Microsoft Corporation drivers
- 🏷️ **Device Name Integration**: Folder names include actual device names for easy identification
- 📁 **Hierarchical Organization**: Groups drivers by device class GUID → version with device names
- 📊 **Detailed Summary**: Generates comprehensive driver inventory reports
- 🔧 **Dry Run Mode**: Preview what would be backed up without actually exporting
- 📝 **Verbose Logging**: Detailed output showing device information during backup
- ⏰ **Timestamped Backups**: Each backup session gets a unique timestamp
- 🔄 **Smart Deduplication**: Same driver versions for multiple devices grouped together

## Installation

### Prerequisites

- Windows operating system (Windows 10/11 recommended)
- Rust toolchain (latest stable version)
- Administrative privileges (required for `pnputil` access)
- Windows Management Instrumentation (WMI) service running

### Build from Source

```bash
git clone <repository-url>
cd driver-backup
cargo build --release
```

The compiled binary will be available at `target/release/driver-backup.exe`.

## Usage

### Command Syntax

```bash
driver-backup.exe backup [OPTIONS]
```

### Command Line Options

| Option | Short | Long | Description |
|--------|-------|------|-------------|
| Output directory | `-o` | `--output` | Directory where driver backups will be stored (default: `driver_backup`) |
| Verbose mode | `-v` | `--verbose` | Enable detailed logging output with device information |
| Dry run | `-d` | `--dry-run` | Show what would be backed up without actually exporting |
| Help | `-h` | `--help` | Display help information |

### Usage Examples

```bash
# Basic backup to default directory
driver-backup.exe backup

# Backup to specific directory with verbose output
driver-backup.exe backup --output "C:\DriverBackups" --verbose

# Dry run to see what would be backed up
driver-backup.exe backup --dry-run --verbose

# Quick backup with minimal output
driver-backup.exe backup -o "D:\MyDrivers"
```

## How It Works

1. **Driver Discovery**: Queries Windows WMI `Win32_PnPSignedDriver` class to enumerate all installed drivers
2. **Microsoft Filtering**: Identifies and excludes Microsoft and Microsoft Corporation drivers
3. **Device Grouping**: Organizes drivers by device class GUID for logical grouping
4. **Version Deduplication**: Groups same driver versions for multiple devices together
5. **Device Name Integration**: Includes actual device names in folder structure
6. **Export Process**: Uses Windows `pnputil /export-driver` to extract driver files
7. **Summary Generation**: Creates detailed driver inventory report

## Backup Structure

The tool creates a hierarchical folder structure that clearly identifies which device each driver belongs to:

```
driver_backup/
└── drivers_20251030_112428/                    # Main backup folder with timestamp
    ├── Display_4d36e968_e325_11ce_bfc1_08002be10318/    # Display devices
    │   └── Intel(R) UHD Graphics 770_Intel Corporation_32.0.101.7040_2025-09-19/
    │       ├── oem2.inf                                 # Driver installation file
    │       ├── igd_dch.sys                             # System driver
    │       └── igd_dch.cat                             # Security catalog
    ├── Net_4d36e972_e325_11ce_bfc1_08002be10318/      # Network devices
    │   ├── Realtek PCIe GbE Family Controller_Realtek_10.56.119.2022_2022-01-19/
    │   ├── VMware Virtual Ethernet Adapter for_VMware_ Inc._14.0.0.8_2023-02-11/
    │   └── TAP-Windows Adapter V9 for OpenVPN _TAP-Windows Provider_9.24.2.601_2019-09-27/
    ├── System_4d36e97d_e325_11ce-bfc1-08002be10318/    # System devices
    │   ├── Intel(R) Serial IO GPIO Host Contro_Intel Corporation_30.100.2417.30_2024-04-24/
    │   ├── Intel(R) SPI (flash) Controller - 7_INTEL_10.1.46.5_1968-07-18/
    │   └── AMD Special Tools Driver_Advanced Micro Devic_1.7.16.219_2022-06-22/
    ├── Media_4d36e96c_e325_11ce_bfc1_08002be10318/     # Media devices
    │   └── Realtek High Definition Audio_Realtek Semiconducto_6.0.9151.1_2021-04-13/
    └── driver_backup_summary.txt                      # Comprehensive driver report
```

### Folder Naming Convention

**Format**: `DeviceName_Provider_Version_Date`

- **DeviceName**: Actual hardware device name (truncated to 35 chars)
- **Provider**: Driver manufacturer/provider (truncated to 20 chars)
- **Version**: Driver version number (truncated to 15 chars)
- **Date**: Driver release date in YYYY-MM-DD format

**Examples**:
- `Intel(R) UHD Graphics 770_Intel Corporation_32.0.101.7040_2025-09-19`
- `Realtek PCIe GbE Family Controller_Realtek_10.56.119.2022_2022-01-19`
- `VMware Virtual Ethernet Adapter for_VMware_ Inc._14.0.0.8_2023-02-11`
- `HD Audio Driver for Display Audio_Intel Corporation_32.0.101.7040_2025-09-19`
- `Nefarius HidHide Device_Nefarius Software So_1.4.181.0_2023-10-31`

## Driver Information Captured

For each backed-up driver, tool records:

- **Sequential Number**: Global numbering across all drivers (1, 2, 3...)
- **Device Class**: Windows device class grouping (DISPLAY, NET, SYSTEM, etc.)
- **Device Name**: Hardware device name (primary identifier)
- **Provider/Manufacturer**: Driver publisher or manufacturer
- **Description**: Device description from Windows
- **Driver Version**: Specific version number
- **Release Date**: Driver release date
- **Class GUID**: Unique device class identifier
- **Original INF**: OEM INF filename
- **Folder Location**: Backup folder path

## Supported Device Classes

The tool recognizes and organizes drivers for these device classes (shown in summary order):

1. **DISPLAY**: Graphics cards, display adapters
2. **MEDIA**: Audio devices, sound cards
3. **MONITOR**: Display monitors
4. **NET**: Network adapters, Ethernet, WiFi, VPN
5. **SOFTWARECOMPONENT**: Software drivers and components
6. **SYSTEM**: Chipset, motherboard components, system devices
7. **HIDClass**: Human interface devices (keyboards, mice)
8. **USB**: USB controllers and devices
9. **Unknown**: Unrecognized device types

Each class appears in alphabetical order in the summary file with sequential numbering across all classes.

## Summary File Format

The summary file (`driver_backup_summary.txt`) contains a sequential list of all exported drivers organized by device class:

```
Driver Export Summary
Generated: 2025-10-30 11:33:38 UTC
Total drivers exported: 25

Drivers by Class:
=================

DISPLAY (1 drivers):
1. oem2.inf
   Device: Intel(R) UHD Graphics 770
   Provider: Intel Corporation
   Description: Intel(R) UHD Graphics 770
   Version: 32.0.101.7040
   Date: 2025-09-19
   Folder: Intel(R) UHD Graphics 770_Intel Corporation_32.0.101.7040_2025-09-19
   Class GUID: {4d36e968-e325-11ce-bfc1-08002be10318}

NET (4 drivers):
2. oem67.inf
   Device: VMware Virtual Ethernet Adapter for VMnet8
   Provider: VMware, Inc.
   Description: VMware Virtual Ethernet Adapter for VMnet8
   Version: 14.0.0.8
   Date: 2023-02-11
   Folder: VMware Virtual Ethernet Adapter for_VMware_ Inc._14.0.0.8_2023-02-11

3. oem67.inf
   Device: VMware Virtual Ethernet Adapter for VMnet1
   Provider: VMware, Inc.
   Description: VMware Virtual Ethernet Adapter for VMnet1
   Version: 14.0.0.8
   Date: 2023-02-11
   Folder: VMware Virtual Ethernet Adapter for_VMware_ Inc._14.0.0.8_2023-02-11

[... more drivers with sequential numbering ...]

SYSTEM (18 drivers):
[... system drivers with sequential numbering ...]
```

**Key Features:**
- **Sequential Numbering**: Global numbering from 1 to N across all drivers
- **Class-based Grouping**: Drivers grouped by device class (DISPLAY, NET, SYSTEM, etc.)
- **Alphabetical Class Order**: Classes appear in alphabetical order for consistency
- **Complete Metadata**: Each driver entry includes device name, provider, version, date, and folder location
- **Easy Reference**: Sequential numbers make it easy to reference specific drivers during troubleshooting or documentation

## Requirements

### System Requirements

- **Operating System**: Windows 10/11 (Server 2016+ also supported)
- **Privileges**: Administrative rights required
- **Services**: Windows Management Instrumentation (WMI) service must be running
- **Disk Space**: Sufficient space for driver files (typically 100MB-1GB depending on system)

### Technical Requirements

- **Windows API**: Access to WMI and driver management APIs
- **pnputil**: Windows PnP utility (built into modern Windows)
- **File System**: Write permissions to backup directory

## Dependencies

```toml
[dependencies]
wmi = "0.13"
tokio = { version = "1.47", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
anyhow = "1.0"
clap = { version = "4.5", features = ["derive"] }
chrono = "0.4"
```

## Error Handling

The tool includes comprehensive error handling for:

### WMI and Driver Access
- WMI connection failures
- Missing or corrupted driver information
- Permission denied errors

### File Operations
- Invalid backup directory paths
- Insufficient disk space
- File system permission issues

### Driver Export
- `pnputil` execution failures
- Protected or system drivers
- Path length limitations

### Common Error Messages and Solutions

| Error | Cause | Solution |
|-------|-------|----------|
| "Access Denied" | Insufficient privileges | Run as Administrator |
| "WMI Connection Failed" | WMI service not running | Start Windows Management Instrumentation service |
| "No non-Microsoft drivers found" | Only Microsoft drivers installed | Check for third-party hardware/drivers |
| "pnputil not found" | Older Windows version | Use Windows 10/11 or install Windows SDK |
| "Path too long" | Deep folder structure | Use shorter backup path or move to root |

## Security Considerations

- **Read-Only Operations**: Tool only reads driver information, doesn't modify system
- **No Configuration Changes**: Driver installations are not modified
- **Administrative Access**: Required only for reading driver information
- **Backup Files**: Only exports existing driver files from system
- **No Network Access**: Tool works completely offline

## Use Cases

### System Administrators
- Pre-deployment driver preparation
- System migration planning
- Driver inventory management
- Backup before system updates

### IT Support Professionals
- Driver troubleshooting preparation
- Hardware replacement scenarios
- Multi-system driver deployment
- Remote driver management

### Power Users
- System backup strategies
- Hardware upgrade preparation
- Dual-boot driver management
- Custom Windows installations

### OEM/PC Builders
- Driver bundle creation
- System image preparation
- Quality assurance testing
- Deployment automation

## Performance

### Typical Performance Metrics
- **Discovery Time**: 2-5 seconds for driver enumeration
- **Backup Time**: 30 seconds - 2 minutes depending on number of drivers
- **File Sizes**: 100MB - 1GB typical for modern systems
- **Memory Usage**: <50MB during operation

### Optimization Features
- Efficient WMI queries
- Parallel driver processing
- Smart folder naming to avoid path limits
- Memory-conscious file operations

## Troubleshooting

### Advanced Troubleshooting Steps

1. **Check WMI Service**: Run `services.msc` and ensure "Windows Management Instrumentation" is running
2. **Verify Permissions**: Confirm administrative privileges with `whoami /priv`
3. **Test pnputil**: Run `pnputil /enum-drivers` to verify tool availability
4. **Check Disk Space**: Ensure sufficient space in backup location
5. **Path Length**: Use shorter backup paths if experiencing path limit errors

### Debug Mode

For troubleshooting, use verbose mode:
```bash
driver-backup.exe backup --verbose --dry-run
```

This will show all driver discovery and processing without creating files.

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

### Development Setup

```bash
# Clone repository
git clone <repository-url>
cd driver-backup

# Install Rust toolchain
rustup update stable
rustup default stable

# Build in debug mode for development
cargo build

# Run tests
cargo test

# Build release version
cargo build --release
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Disclaimer

This tool is provided as-is for educational and backup purposes. 

**⚠️ Important Notes:**
- Always test driver restores in a controlled environment
- Backup your system before making any driver changes
- The authors are not responsible for system damage or data loss
- Use at your own risk and discretion

## Support

For issues, questions, or feature requests:
- Create an issue on the project repository
- Include system information (Windows version, hardware details)
- Provide verbose output when reporting problems
- Share error messages and steps to reproduce

---

**Version**: 1.0.0  
**Last Updated**: 2025-10-30  
**Compatible**: Windows 10/11, Server 2016+