# Update v2.1 - INF-Based Grouping

## What Changed

### Previous Behavior (v2.0)

Drivers were grouped by **Device Name + Version**:

```
drivers_20250110_143025/
├── NVIDIA_GeForce_RTX_3080_30.0.15.1179/
├── NVIDIA_GeForce_RTX_3070_30.0.15.1179/
└── NVIDIA_GeForce_RTX_3060_30.0.15.1179/
```

**Problem**: These three devices likely use the **same driver package** (same INF file), but were split into separate folders.

### New Behavior (v2.1)

Drivers are grouped by **INF File** (the actual driver package):

```
drivers_20250110_143025/
└── NVIDIA GeForce RTX 3080 Package/
    ├── driver_info.csv (contains all 3 GPU models)
    ├── oem1.inf
    └── [driver files supporting all 3 GPUs]
```

**Benefit**: All devices using the same driver package are now in **one folder**.

## Why This Change?

### Understanding Driver Packages

A single driver package (INF file) often supports multiple devices:

**Example 1 - NVIDIA Graphics**:

- INF: `oem1.inf`
- Supports: RTX 3060, RTX 3070, RTX 3080, RTX 3090
- **Before**: 4 separate folders
- **After**: 1 folder with all 4 devices listed in CSV

**Example 2 - Intel Network**:

- INF: `oem5.inf`
- Supports: Multiple Intel Ethernet adapters
- **Before**: Multiple folders for each adapter
- **After**: 1 folder for the Intel network driver package

**Example 3 - Realtek Audio**:

- INF: `oem3.inf`
- Supports: Various Realtek audio codecs
- **Before**: Separate folders per codec
- **After**: 1 folder for Realtek audio package

### Benefits

✅ **More Accurate**: Groups drivers exactly as Windows does
✅ **Less Redundancy**: No duplicate driver packages
✅ **Easier Upload**: One zip file per driver package
✅ **Better Restore**: Install complete driver package at once
✅ **Database Clarity**: Multiple devices linked to one driver package

## New Folder Naming

### Format

```
[Primary Device Name] Package
```

### Examples

- `NVIDIA GeForce RTX 3080 Package`
- `Realtek High Definition Audio Package`
- `Intel(R) Wireless-AC 9560 Package`
- `AMD Radeon RX 6800 Package`

### Why "Package"?

- Clearly indicates this is a driver package
- Distinguishes from individual device folders
- Makes it obvious that multiple devices may be inside

## CSV Changes

### driver_info.csv (per folder)

Now contains **all devices** in that driver package:

```csv
Device Name,Driver Version,Driver Date,Hardware ID,...
NVIDIA GeForce RTX 3080,30.0.15.1179,2023-08-15,PCI\VEN_10DE&DEV_2206,...
NVIDIA GeForce RTX 3070,30.0.15.1179,2023-08-15,PCI\VEN_10DE&DEV_2484,...
NVIDIA GeForce RTX 3060,30.0.15.1179,2023-08-15,PCI\VEN_10DE&DEV_2503,...
```

All three GPUs share the same:

- INF file (oem1.inf)
- Driver version
- Driver date
- Driver provider

### all_drivers.csv

The "Folder Name" column now shows the package name:

```csv
Device Name,...,Folder Name
NVIDIA GeForce RTX 3080,...,NVIDIA GeForce RTX 3080 Package
NVIDIA GeForce RTX 3070,...,NVIDIA GeForce RTX 3080 Package
NVIDIA GeForce RTX 3060,...,NVIDIA GeForce RTX 3080 Package
```

**Note**: The folder name uses the **first device's name** + "Package", but contains all devices from that INF.

## Summary File Changes

Now organized by INF file:

```
Driver Export Summary
=====================

Drivers by Package (INF File):
==============================

1. oem1.inf (3 devices in package):
   Folder: NVIDIA GeForce RTX 3080 Package
   Provider: NVIDIA
   Version: 30.0.15.1179
   Date: 2023-08-15
   Class: Display

   Devices in this package:
   1. NVIDIA GeForce RTX 3080
      Hardware ID: PCI\VEN_10DE&DEV_2206
      Device ID: PCI\VEN_10DE&DEV_2206&SUBSYS_...

   2. NVIDIA GeForce RTX 3070
      Hardware ID: PCI\VEN_10DE&DEV_2484
      ...
```

## Database Implications

### Recommended Schema Update

Consider adding an `inf_name` field to track the driver package:

```sql
ALTER TABLE drivers ADD COLUMN inf_name VARCHAR(100);

-- Create index for faster lookups
CREATE INDEX idx_inf_name ON drivers(inf_name);
```

### Query Examples

```sql
-- Find all devices using the same driver package
SELECT device_name, hardware_id
FROM drivers
WHERE inf_name = 'oem1.inf';

-- Count devices per driver package
SELECT inf_name, COUNT(*) as device_count
FROM drivers
GROUP BY inf_name
ORDER BY device_count DESC;

-- Find driver packages with multiple devices
SELECT inf_name, folder_name, COUNT(*) as device_count
FROM drivers
GROUP BY inf_name, folder_name
HAVING COUNT(*) > 1;
```

## Migration from v2.0 to v2.1

### If You Already Ran v2.0

**Option 1: Fresh Backup (Recommended)**

```powershell
# Just run the new version
.\driver-backup.exe backup --output "D:\Backups_v2.1" --verbose
```

**Option 2: Keep Both**

- v2.0 backups: Organized by device name/version
- v2.1 backups: Organized by INF (driver package)
- Both are valid, just different organization

### Database Migration

If you already imported v2.0 data:

```sql
-- Add new column
ALTER TABLE drivers ADD COLUMN inf_name VARCHAR(100);

-- Re-import from new all_drivers.csv to populate inf_name
-- Or manually update based on device information
```

## Technical Details

### Code Changes

1. **Grouping Strategy**:

   - **Before**: `HashMap<(DeviceName, Version), Vec<Driver>>`
   - **After**: `HashMap<INFName, Vec<Driver>>`

2. **Folder Naming**:

   - **Before**: `create_driver_folder_name()` used device + version
   - **After**: `format!("{} Package", primary_device_name)`

3. **Export Optimization**:

   - **Before**: Loop through devices, export each (with duplicate detection)
   - **After**: Export once per INF file (more efficient)

4. **Summary Organization**:
   - **Before**: Grouped by device name/version
   - **After**: Grouped by INF file

### Performance Impact

✅ **Faster**: Fewer export operations (one per INF instead of one per device)
✅ **Less Storage**: No duplicate driver packages
✅ **Cleaner**: More logical folder structure

## Examples

### Before (v2.0)

```
drivers_20250110_143025/
├── NVIDIA_GeForce_RTX_3080_30.0.15.1179/
│   └── [driver files]
├── NVIDIA_GeForce_RTX_3070_30.0.15.1179/
│   └── [same driver files - duplicate!]
├── Intel_Ethernet_I219-V_12.19.2.45/
│   └── [driver files]
└── Intel_Ethernet_I218-V_12.19.2.45/
    └── [same driver files - duplicate!]
```

### After (v2.1)

```
drivers_20250110_143025/
├── NVIDIA GeForce RTX 3080 Package/
│   ├── driver_info.csv (contains both RTX 3080 & 3070)
│   └── [driver files - stored once]
└── Intel Ethernet I219-V Package/
    ├── driver_info.csv (contains both I219-V & I218-V)
    └── [driver files - stored once]
```

**Storage Saved**: ~50% in this example (no duplicates)

## Workflow Impact

### Database Upload

**No change in workflow**, just more accurate data:

1. Import `all_drivers.csv` or individual `driver_info.csv` files
2. Zip each folder (now contains complete driver package)
3. Upload zips to storage
4. Link database records to zip files

**Advantage**: Multiple database records can point to the same driver package zip file.

### Restore Process

**Improved**:

- Unzip a driver package folder
- Install using the INF file
- **All devices** in that package are automatically supported

## Summary

### Key Improvements

✅ Drivers grouped by actual driver package (INF file)
✅ No duplicate driver packages
✅ More accurate representation of Windows driver structure
✅ Clearer folder names with "Package" suffix
✅ Better for backup, restore, and database management
✅ More efficient storage and export process

### Version Comparison

| Feature        | v2.0                  | v2.1                 |
| -------------- | --------------------- | -------------------- |
| Grouping       | Device Name + Version | INF File             |
| Folder Name    | `DeviceName_Version`  | `DeviceName Package` |
| Duplicates     | Possible              | None                 |
| CSV per folder | ✅                    | ✅                   |
| Master CSV     | ✅                    | ✅                   |
| Hardware ID    | ✅                    | ✅                   |
| Device ID      | ✅                    | ✅                   |
| Accuracy       | Good                  | Excellent            |

---

**Bottom Line**: v2.1 provides more accurate driver package organization by grouping drivers exactly as Windows does - by their INF file.
