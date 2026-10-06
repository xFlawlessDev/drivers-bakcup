# Driver Backup Tool

A Rust-based tool to backup, inspect, and manage non-Microsoft drivers on
Windows. Designed to produce database-ready output.

- **Backup** - export third-party drivers with an organized folder structure
- **Inspect** - extract driver information from installer packages
- **Scan** - list all INF files in a folder with a detailed summary
- **Export** - dump connected-device inventory to CSV

---

## Requirements

- **Windows 10/11** (Server 2016+ supported)
- **Administrator rights** - required for `backup` and `export`
- **7-Zip** (optional) - needed to extract `.exe`, `.7z`, `.rar` in `inspect`

## Installation

From a release:

```powershell
.\driver-backup.exe --help
```

From source:

```powershell
cargo build --release
# binary at target\release\driver-backup.exe
```

---

## Commands Overview

```
driver-backup.exe <COMMAND>

Commands:
  backup   Backup drivers to a specified directory
  inspect  Inspect a driver package (exe/zip/folder) for driver information
  scan     Scan a folder to identify and list all INF files
  export   Export connected device hardware IDs to CSV
```

---

## 1. Backup Command

Exports all non-Microsoft drivers. **Requires Administrator.**

```powershell
# Basic backup (default: .\driver_backup)
.\driver-backup.exe backup

# Custom output directory, verbose
.\driver-backup.exe backup -o "D:\Backups" -v

# Preview without exporting
.\driver-backup.exe backup -d -v
```

| Option | Long | Description |
|--------|------|-------------|
| `-o` | `--output <PATH>` | Output directory (default: `driver_backup`) |
| `-v` | `--verbose` | Detailed per-package logging |
| `-d` | `--dry-run` | Preview without exporting |

### What gets backed up

The backup covers two sources:

1. **Present devices** - drivers bound to hardware that Windows reports right
   now (queried from WMI).
2. **Driver store packages** - every non-Microsoft package installed in the
   driver store (`pnputil /enum-drivers`), including packages whose device is
   *not currently connected*.

The second source matters for hardware that only enumerates while attached. The
classic example is **display audio** (`intcdaud.inf`, `mshdadac.inf`): its device
appears only while an HDMI/DisplayPort link is active. Without the driver-store
pass, that audio driver would be silently skipped unless a cable was connected at
backup time.

### Output Structure

```
driver_backup/
└── drivers_YYYYMMDD_HHMMSS/
    ├── Display/
    │   └── NVIDIA GeForce RTX 3080_30.0.15.1179 Package/
    │       └── [exported driver files]
    ├── Media/
    │   ├── Realtek High Definition Audio_6.0.9346.1 Package/
    │   └── intcdaud_11.2.0.15 Package/          <- unbound, name from INF
    ├── Net/
    │   └── Intel Ethernet I219-V_12.19.2.45 Package/
    ├── all_drivers.csv
    └── device_class_legend.csv
```

Folders are grouped by device class, then package. Packages that are not bound
to a present device are named from their original INF (e.g. `intcdaud`) because
OEM numbers (`oemN.inf`) are reassigned between machines.

### `all_drivers.csv`

Master CSV with one row per INF package found in the backup:

```csv
INF File,Device Class,Provider,Driver Version,Driver Date,Device Count,Folder Name,Device Names,Hardware IDs
```

### `device_class_legend.csv`

Maps each Windows device class to its class GUID, for building database mappings
and correlating `all_drivers.csv` rows.

```csv
Device Class,Class GUID,Present Devices,Installed Packages
DISPLAY,{4d36e968-e325-11ce-bfc1-08002be10318},1,1
MEDIA,{4d36e96c-e325-11ce-bfc1-08002be10318},2,4
NET,{4d36e972-e325-11ce-bfc1-08002be10318},3,3
```

- **Present Devices** - bound to hardware now (from WMI)
- **Installed Packages** - present in the driver store, including packages whose
  device is not currently connected

---

## 2. Inspect Command

Extracts driver information from an installer package or folder without
installing anything.

```powershell
# A folder containing INF files
.\driver-backup.exe inspect -p "C:\Downloads\DriverPackage"

# An installer archive
.\driver-backup.exe inspect -p "C:\Downloads\Intel_Graphics.exe"

# Export findings to CSV
.\driver-backup.exe inspect -p "C:\Downloads\DriverPackage" -o "info.csv"
```

| Option | Long | Description |
|--------|------|-------------|
| `-p` | `--path <PATH>` | Installer or folder (required) |
| `-o` | `--output <CSV>` | Output CSV file (optional) |
| `-v` | `--verbose` | Show all device entries |

Supported inputs: folders, `.exe`, `.zip`, `.7z`, `.rar`, and single `.inf`
files.

---

## 3. Scan Command

Lists all INF files in a folder with a summary.

```powershell
# Current folder only
.\driver-backup.exe scan -p "C:\Drivers"

# Recursive, grouped by class, export to CSV
.\driver-backup.exe scan -p "C:\Drivers" -r -g -o "inventory.csv"
```

| Option | Long | Description |
|--------|------|-------------|
| `-p` | `--path <PATH>` | Folder to scan (required) |
| `-o` | `--output <CSV>` | Output CSV file (optional) |
| `-v` | `--verbose` | Show all Hardware IDs |
| `-g` | `--group` | Group results by device class |
| `-r` | `--recursive` | Include subfolders |

---

## 4. Export Command

Inventory of connected devices. Without `-f` it writes a CSV only; with `-f` it
also exports driver files (same behavior as `backup`).

```powershell
# CSV inventory of connected devices
.\driver-backup.exe export -o "hardware_inventory.csv"

# Include Microsoft drivers
.\driver-backup.exe export -o "inventory.csv" --all

# Also export driver files + legend
.\driver-backup.exe export -o "D:\Inventory" -f
```

| Option | Long | Description |
|--------|------|-------------|
| `-o` | `--output <PATH>` | CSV path or output directory |
| `--all` | `--all` | Include Microsoft drivers |
| `-v` | `--verbose` | Detailed output |
| `-f` | `--files` | Also export driver files |

---

## Driver Information Captured

| Field | Description |
|-------|-------------|
| Device Name | Friendly name of the device |
| Driver Version | Version number (e.g. `31.0.101.5590`) |
| Driver Date | Release date |
| Hardware ID | `PCI\VEN_xxxx&DEV_xxxx` format |
| Device Class | Display, Net, Media, etc. |
| Class GUID | Windows device class GUID |
| Provider | Driver publisher (Intel, NVIDIA, etc.) |
| INF Name | INF file name |
| Manufacturer | Device manufacturer |

---

## Database Workflow

Individual driver packages can be uploaded one at a time, or the whole backup
can be imported in bulk.

1. Run `backup` and review `all_drivers.csv` plus `device_class_legend.csv`.
2. Import `all_drivers.csv` into your database (bulk), using the `Folder Name`
   column to locate each package.
3. Zip each package folder and upload to file storage.
4. Store the zip path in the database record.

Example schema:

```sql
CREATE TABLE drivers (
    id            INTEGER PRIMARY KEY,
    device_name   VARCHAR(255),
    driver_version VARCHAR(50),
    driver_date   DATE,
    hardware_id   VARCHAR(255),
    inf_name      VARCHAR(100),
    provider      VARCHAR(100),
    device_class  VARCHAR(50),
    class_guid    VARCHAR(50),
    backup_folder VARCHAR(255),
    zip_file_path VARCHAR(500),
    backup_date   TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_device_name ON drivers(device_name);
CREATE INDEX idx_provider    ON drivers(provider);
CREATE INDEX idx_driver_date ON drivers(driver_date);
```

---

## Troubleshooting

| Issue | Solution |
|-------|----------|
| "requires administrative privileges" | Run as Administrator |
| "Failed to export driver" | Driver may be protected or in use; check `-v` output |
| "7-Zip not found" | Install 7-Zip, or use `.zip` for `inspect` |
| "No INF files found" | Check the path; add `-r` for recursive scan |
| "Path too long" | Use a shorter output path, e.g. `C:\Backup` |
| A driver is missing from the backup | Confirm it is installed in the driver store with `pnputil /enum-drivers`. Microsoft inbox INFs (e.g. `hdaudio.inf`) ship with Windows and are intentionally excluded. |

---

## Version

See [CHANGELOG.md](CHANGELOG.md) for the full history.

Current: **v2.4**
