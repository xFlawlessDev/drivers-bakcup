# Changelog

## v2.5.2 - Pack Merging, Flat Layout, Hardware IDs

### Changed

- `pack` now **merges** packages that describe the same driver, keyed by device
  class + device name + driver version. This matches the `export` command's
  behavior: nine `ibtusb` OEM folders (`oem37`..`oem45`) collapse into one
  entry with the union of their hardware IDs, instead of nine separate rows.
  The other source folders are recorded in a new `Merged From` column.
- Output is now a **flat** layout, `<pack_slug>/<Class>_<Provider>_<Version>_<InfName>/`,
  matching `export` (no more `Drivers/<Category>/` nesting). A numeric suffix is
  added when two packages would otherwise resolve to the same folder name.
- Provider references (`%ManufacturerName%`, `%PROVIDER_NAME%`) are now resolved
  against the INF `[Strings]` table in the parser, so folder names and the
  manifest show `Intel` rather than a placeholder.

### Added

- `Hardware ID Count` and `Hardware IDs` columns in `manifest.csv`, listing the
  union of unique hardware IDs across merged packages (capped at 10 with
  `+N more`).

## v2.5.1 - Device Names in Pack Manifest

### Added

- `pack` manifest now includes `Device Name Count` and `Device Names` columns,
  listing the unique friendly device names each INF supports (capped at 10 with
  a `+N more` suffix so GPU/chipset packages with hundreds of devices stay
  readable).

### Fixed

- INF string values that concatenate adjacent quoted segments (e.g.
  `"Intel(R) UHD Graphics" "710"`) are now joined correctly, so stray quotes no
  longer leak into device names.

## v2.5 - Driver Pack Normalizer

### Added

- New `pack` command that normalizes a vendor driver pack into one consistent,
  upload-ready layout plus a manifest. Handles the structural differences
  between ODMs (nested category folders vs flat per-INF folders), per-OS
  duplicate folders, mixed class casing, and INF-less installer folders.
  - Discovers every INF package recursively.
  - Groups by existing category folder, or by device class for flat packs.
  - Canonicalizes device class casing (`net` -> `Net`) and cleans `stampinf`
    version placeholders.
  - Deduplicates by INF SHA-256 (e.g. 483 packages -> 179 unique on the
    reference Clevo pack).
  - Copies full package sub-trees (`Config/`, `ifwi/`, ...) while excluding
    sibling packages.
  - Emits `manifest.csv` (one row per package, with duplicate tracking) and
    `summary.csv` (totals and per-class counts).
  - Auto-detects ODM/model/revision from `autoins.ini` and folder names, with
    CLI overrides.
- Split the pack logic into `pack`, `pack_discover`, and `pack_meta` modules.

## v2.4 - Driver Store Coverage

### Internal Refactor

- Split the single `main.rs` (≈1900 lines) into focused modules: `cli`,
  `models`, `backup`, `driver_store`, `inf_parser`, `inf_extract`, `scan`,
  `csv_export`, `pnputil`, `wmi_query`, and `util`. `main.rs` is now a thin
  entry point.
- Removed duplicated CSV-escaping, folder-name sanitizing, driver-store
  enumeration, and `pnputil` invocation code by centralizing them.
- Added unit tests for CSV escaping, date formatting, name sanitizing, driver
  store parsing, and INF parsing.

### Problem

The backup only enumerated `Win32_PnPSignedDriver`, which returns one entry per
*currently present* PnP device. Driver packages installed in the Windows driver
store but not bound to a present device were silently skipped.

The clearest example is display audio ("HD Audio Driver for Display Audio",
`intcdaud.inf` / `mshdadac.inf`). Its HDAudio function device only enumerates
while a display link (HDMI/DisplayPort) is active, so the driver was only backed
up when a monitor was connected. Unplugging the cable before backing up meant the
audio driver was lost.

### Changes

- Added `DriverStorePackage` and `DriverBackup::get_driver_store_packages()`,
  enumerating every installed package via `pnputil /enum-drivers`.
- `backup_drivers()` now runs a second pass that exports non-Microsoft packages
  installed but not bound to a present device. They are placed under the same
  `DeviceClass/` structure, named from the original INF
  (e.g. `MEDIA/intcdaud_11.2.0.15 Package`) since OEM numbers are reassigned
  between machines.
- `export --files` applies the same logic.
- Backup no longer aborts when WMI reports no present third-party devices; it
  still checks the driver store.
- Added `device_class_legend.csv`, mapping each device class to its class GUID
  with present-device and installed-package counts. This also uses the
  previously unused `ClassGuid` field from `Win32_PnPSignedDriver`.
- On the reference machine this increased backed-up packages from 17 to 41,
  recovering display audio and 20+ other non-present packages.

### Notes

- Microsoft-provided packages remain excluded unless `export --all` is used.
- Microsoft inbox INFs (e.g. `hdaudio.inf`) are not in the driver store and are
  not backed up; they ship with Windows.

## v2.3 - Scan and Inspect Commands

- Added `scan` command to list INF files in a folder, with grouping by device
  class (`-g`) and recursive scanning (`-r`).
- Added `inspect` command to extract driver info from `.exe`, `.zip`, `.7z`,
  `.rar`, folders, or a single INF.
- Added `export` command for connected-device hardware inventory.
- `inspect` and `scan` support CSV export.

## v2.2 - Device Class Organization

- Drivers are now organized into folders by **device class** first, then by
  driver package, mirroring Windows Device Manager:

  ```
  drivers_TIMESTAMP/
  ├── Display/
  │   └── NVIDIA GeForce RTX 3080_30.0.15.1179 Package/
  ├── Media/
  │   └── Realtek High Definition Audio_6.0.9346.1 Package/
  └── Net/
      └── Intel Ethernet I219-V_12.19.2.45 Package/
  ```

- Folder names now include the driver version: `DeviceName_Version Package`.

### Common Device Classes

| Class | Description | Examples |
|-------|-------------|----------|
| Display | Graphics cards | NVIDIA, AMD, Intel GPUs |
| Net | Network adapters | Ethernet, Wi-Fi |
| Media | Audio devices | Sound cards, audio codecs |
| HIDClass | Human interface devices | Keyboards, mice, controllers |
| USB | USB controllers | Hubs, root hubs |
| System | System devices | Chipset, ACPI |
| SCSIAdapter | Storage controllers | SATA, NVMe |
| Bluetooth | Bluetooth adapters | Bluetooth radios |
| Printer | Printing devices | Printers, scanners |
| Monitor | Display monitors | Monitor drivers |

## v2.1 - INF-Based Grouping

Drivers are grouped by **INF file** (the actual driver package) instead of by
device model. A single package often supports many devices, so before this
change one package could produce several near-identical folders.

```
drivers_TIMESTAMP/
└── NVIDIA GeForce RTX 3080 Package/
    ├── oem1.inf
    └── [files supporting RTX 3060/3070/3080/3090]
```

## v2.0 - Database-Ready Output

- Added `Hardware ID` and `Device ID` capture from WMI.
- Reorganized output into a device-based folder structure.
- Added `all_drivers.csv` master file for bulk database import.
- Added human-readable summary output.

### Breaking Change

The folder structure changed from the early GUID-based layout to device-based
folders. Old backups remain valid but use a different organization.
