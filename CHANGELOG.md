# Changelog

## v2.7 - Aligned Backup and Pack Output

### Changed (breaking for CSV consumers)

- `backup`'s `all_drivers.csv` now uses the same naming and extra columns as the
  `pack` manifest: `Driver Name` (was `INF File`), `Normalized Folder` (was
  `Folder Name`), plus `Catalog`, `Device Name Count`, `Hardware ID Count`,
  `INF SHA256`, and `Status`. Device names and hardware IDs are written in full.
- `export` and `scan` CSVs were aligned to the same naming (`Driver Name`,
  `Catalog`, and `* Count` columns).

The two outputs now share 14 core columns, so either can feed the same
driver-upload pipeline. `pack` adds pack-specific columns (`Pack`, `ODM`,
`Model`, `Revision`, `Merged From`, `Archive`, `Superseded By`, ...).

## v2.6.3 - Complete Hardware IDs

### Changed

- `pack` no longer truncates `Device Names` and `Hardware IDs` in the manifest.
  Every unique value is written in full, so the columns can be used directly as
  driver-upload metadata. Previously they were capped at 10 with a `+N more`
  suffix.
- This makes the `pack` manifest columns match the `backup` summary, which
  already wrote hardware IDs in full.

## v2.6.2 - Superseded Drivers

### Added

- The pack manifest reports when a older version of the same driver ships
  alongside a newer one. Rows get `Status=superseded` and a `Superseded By`
  column pointing at the newer package. Detection compares rows by INF name and
  hardware-ID overlap; the higher driver version wins. For example
  `mshdadac` 30.0.101.1371 is superseded by 31.0.101.2125, and `RealtekAPO`
  12.0.6000.1167 by 13.0.6000.1167. Both versions are still kept, since Windows
  ranks them by version.

## v2.6.1 - Clearer Package Labels

### Changed

- A single-INF package is now named after its INF (`..._iigd_dch`) instead of its
  catalog. Intel graphics ships `cui_dch`, `igcc_dch` and `iigd_dch` under the
  same `igdlh.cat`; naming by catalog produced `_igdlh` and `_igdlh_2`, which was
  misleading. Each remains its own package, matching how Windows stores them
  separately in `DriverStore\FileRepository` (`cui_dch.inf_amd64_*`,
  `iigd_dch.inf_amd64_*`).
- Added a `Catalog` column to `manifest.csv` so shared catalogs are visible.

## v2.6 - Installer Bundling

### Fixed

- `pack` no longer duplicates shared payload. INFs that live in the same source
  folder are treated as **one installer bundle** and copied once. Intel graphics
  previously produced four equal-size folders (7 GB from a 1.7 GB source); it now
  produces one package with all the INFs and shared files (`igdlh.cat`, 139 DLLs,
  the `.sys` drivers).
- Each INF now only claims the files it references, instead of every file in its
  folder.

### Changed

- Two-phase grouping:
  1. **Bundle** by source folder — INFs sharing a folder become one package.
  2. **Merge** bundles describing the same driver (INF stem + device name +
     version) — a driver shipped as many single-INF folders (Intel Bluetooth
     `ibtusb` under `CCP`/`GAP`/...) becomes one package.
- Folder naming reflects the grouping: a merged bundle is named after its INF
  (`..._ibtusb`), a folder bundle after the folder (`..._Drivers`), and a single
  INF after the catalog or INF stem.
- `Status` is `package` for combined output, `unique` for a single INF.

## v2.5.4 - Pack Zipping and Driver Date

### Added

- `pack --zip` (`-z`) creates a `<folder>.zip` archive per package and removes
  the folder, so the pack contains only archives plus the manifests and is ready
  to upload as-is. `--keep-folders` retains the folders after archiving.
  Zip creation uses `Compress-Archive` with a `System.IO.Compression.ZipFile`
  fallback.
- `Driver Date` column in `manifest.csv`, normalized to `YYYY-MM-DD`.
- `Archive` column in `manifest.csv`, listing the zip file name when `--zip` is
  used.

## v2.5.3 - Manifest Consistency

### Fixed

- `manifest.csv` now stores the **normalized** device class and driver version
  instead of the raw INF values. Placeholder classes such as
  `System ; TODO: edit Class` become `Unknown`, `stampinf` version suffixes are
  stripped, and class casing is canonical. This makes the CSV columns match the
  generated folder names.

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
