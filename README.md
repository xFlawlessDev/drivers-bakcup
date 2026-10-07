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
  pack     Normalize a vendor driver pack into an upload-ready layout
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
Driver Name,Device Class,Driver Version,Driver Date,Provider,Catalog,Device Count,Device Name Count,Device Names,Hardware ID Count,Hardware IDs,Normalized Folder,INF SHA256,Status
```

The columns are aligned with the `pack` manifest so either file can feed the same
driver-upload pipeline. `Device Names` and `Hardware IDs` are written in full
(never truncated), and `Status` is `unique` or `duplicate`.

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

## 5. Pack Command

Normalizes a vendor driver pack into one consistent layout, then writes a
manifest. Use this when ODM packs arrive with different structures (different
nesting, per-OS sub-folders, mixed casing, duplicate INFs) and you need one
predictable format for upload.

```powershell
# Normalize a pack, auto-detecting ODM/model/revision
.\driver-backup.exe pack -p "D:\Drivers" -o "D:\Normalized"

# Override the detected metadata
.\driver-backup.exe pack -p "D:\Drivers" -o "D:\Normalized" --odm Clevo --model 765 --revision V2xxRNP_1.08.06_0911

# Zip each package (folders removed) for direct upload
.\driver-backup.exe pack -p "D:\Drivers" -o "D:\Normalized" --zip

# Zip but keep the folders too
.\driver-backup.exe pack -p "D:\Drivers" -o "D:\Normalized" --zip --keep-folders

# Preview the planned layout without writing anything
.\driver-backup.exe pack -p "D:\Drivers" -o "D:\Normalized" --dry-run

# Only write the manifest, do not copy driver files
.\driver-backup.exe pack -p "D:\Drivers" -o "D:\Normalized" --manifest-only
```

| Option | Long | Description |
|--------|------|-------------|
| `-p` | `--path <PATH>` | Source driver pack folder (required) |
| `-o` | `--output <PATH>` | Output directory (default: `driver_pack`) |
| | `--odm <NAME>` | Override the ODM name |
| | `--model <ID>` | Override the model identifier |
| | `--revision <REV>` | Override the pack revision |
| | `--keep-duplicates` | Keep duplicate packages instead of collapsing them |
| | `--manifest-only` | Write only the manifest; do not copy files |
| `-z` | `--zip` | Zip each package folder for upload |
| | `--keep-folders` | With `--zip`, keep the folders after archiving |
| `-d` | `--dry-run` | Preview the planned layout |
| `-v` | `--verbose` | Per-package output |

### What it does

1. **Discovers** every driver package (INF) under the source, recursively.
2. **Bundles** INFs that live in the same source folder into one package, so the
   shared payload (`ldl`, `.cat`, `.sys`) is copied once. Intel graphics ships
   `iigd_dch`, `iigd_dch_d`, `iigd_ext`, `iigd_ext_d`, `cui_dch` and `memcntrl`
   in one folder sharing `igdlh.cat`, so they become a single package.
3. **Merges** bundles that describe the same driver (INF stem + device name +
   version). A driver shipped as many single-INF folders — Intel Bluetooth
   `ibtusb` under `CCP`/`GAP`/`GFP`/... — becomes one package with the union of
   its hardware IDs, and the other folders are listed in `Merged From`.
4. **Normalizes** the device class casing (`net`/`NET` -> `Net`), resolves the
   provider reference (`%ManufacturerName%` -> `Intel`), and cleans `stampinf`
   version placeholders.
5. **Copies** each unique package to a flat folder named
   `<Class>_<Provider>_<Version>_<Bundle>` directly under the pack root, keeping
   package sub-folders (`Config/`, `ifwi/`, ...). A numeric suffix is appended
   when two packages would collide.
6. **Keeps** categories that have no INF (EXE/INI-only installers) as
   `installer-only` entries so nothing is lost.
7. **Writes** `manifest.csv` and `summary.csv`.

### Output Structure

Without `--zip`, each package is a folder:

```
driver_pack/
└── Clevo_765_V2xxRNP_1.08.06_0911/
    ├── manifest.csv
    ├── summary.csv
    ├── Display_Intel Corporation_32.0.101.6556_iigd_dch/
    ├── MEDIA_Intel(R) Corporation_10.29.00.10981_intcbtau/
    ├── Net_Realtek_10.074.1128.2024_rt640x64/
    └── 10_SBX/                    <- installer-only, no INF
```

With `--zip`, each package becomes `<folder>.zip` (folder removed unless
`--keep-folders`), so the pack is ready to upload as-is:

```
driver_pack/
└── Clevo_765_V2xxRNP_1.08.06_0911/
    ├── manifest.csv
    ├── summary.csv
    ├── Display_Intel Corporation_32.0.101.6556_iigd_dch.zip
    ├── MEDIA_Intel(R) Corporation_10.29.00.10981_intcbtau.zip
    └── 10_SBX.zip
```

### `manifest.csv`

One row per **merged** driver package.

| Column | Description |
|--------|-------------|
| `Pack` | Pack slug (`ODM_Model_Revision`) |
| `ODM` / `Model` / `Revision` | Pack metadata |
| `Category` | Source category or device class |
| `Device Class` | INF device class, canonicalized |
| `Driver Name` | INF file name without extension |
| `Driver Version` | Version from the INF |
| `Driver Date` | Driver date from the INF, normalized to `YYYY-MM-DD` |
| `Provider` | Driver publisher (references resolved) |
| `Catalog` | Catalog (`.cat`) file the INF is signed under |
| `Device Count` | Number of device entries across merged packages |
| `Device Name Count` | Number of unique friendly device names |
| `Device Names` | All unique device names, semicolon separated (never truncated) |
| `Hardware ID Count` | Number of unique hardware IDs |
| `Hardware IDs` | All unique hardware IDs, semicolon separated (never truncated) |
| `Normalized Folder` | Destination folder name |
| `Source Folder` | Representative source path (the copied package) |
| `Merged From` | Other source folders collapsed into this entry |
| `Archive` | Zip file name (with `--zip`), otherwise empty |
| `Superseded By` | Normalized folder of the newer version, when this one is outdated |
| `File Count` / `Total Bytes` | Size of the representative package |
| `INF SHA256` | Content hash of the representative INF |
| `Duplicate Of` | Set when another member had identical INF content |
| `Status` | `unique`, `package`, `superseded`, or `installer-only` |

### `summary.csv`

Totals (packages found, unique, duplicates, size) and a per-class breakdown.

### Metadata detection

ODM, model, and revision are detected automatically, in order:

1. CLI overrides (`--odm`, `--model`, `--revision`).
2. `autoins.ini` at the pack root (e.g. `CDVersion`).
3. Folder names, e.g. `765\V2xxRNP_1.08.06_0911\Drivers` or
   `GemOne238TJ_270XJ_Drivers\Drivers`.

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
