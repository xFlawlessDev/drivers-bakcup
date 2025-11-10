# Changelog

## Version 2.0.0 - Database-Ready Driver Backup

### Major Changes

#### 1. Enhanced Driver Information Capture

- **Added Hardware ID field**: Captures the hardware identifier for each device
- **Added Device ID field**: Captures the unique device identifier
- Both fields are now available in WMI queries and included in all outputs

#### 2. Reorganized Folder Structure

**Previous Structure** (GUID-based):

```
drivers_TIMESTAMP/
├── Display_{guid}/
│   └── DeviceName_Provider_Version_Date/
```

**New Structure** (Device-based):

```
drivers_TIMESTAMP/
├── DeviceName_Version/
│   ├── driver_info.csv
│   └── [driver files]
```

**Benefits**:

- Simpler folder names: `DeviceName_Version` format
- Drivers from the same package are grouped together
- Directly matches device name and version
- Ready for zipping and upload

#### 3. CSV File Generation

**Per-Driver CSV** (`driver_info.csv` in each folder):

- Contains all devices in that driver package
- Ready for individual database upload
- Includes: Device Name, Version, Date, Hardware ID, Device ID, INF Name, Description, Provider, Class, GUID

**Master CSV** (`all_drivers.csv` in root):

- Contains ALL backed-up drivers in one file
- Additional "Folder Name" column for reference
- Perfect for bulk database import
- Same fields as per-driver CSV plus folder location

#### 4. Improved Summary File

- Now groups by device name and version
- Shows all hardware IDs and device IDs
- Lists all instances of each driver
- More detailed device information

### Technical Changes

#### Code Modifications

1. **Updated `PnPSignedDriver` struct**:

   ```rust
   // Added fields:
   hardware_id: Option<String>,
   device_id: Option<String>,
   ```

2. **Simplified `create_driver_folder_name()`**:

   - Changed from: `DeviceName_Provider_Version_Date`
   - Changed to: `DeviceName_Version`
   - More concise and focused on key identifiers

3. **Rewrote `backup_drivers()` function**:

   - Changed grouping strategy from GUID → Version to DeviceName → Version
   - Drivers with same device name and version are in one folder
   - Handles duplicate INF exports gracefully
   - Creates CSV files for each driver package

4. **Added `create_driver_csv()` method**:

   - Generates CSV for each driver folder
   - Properly escapes CSV fields
   - Includes all required database fields

5. **Added `create_master_csv()` method**:

   - Creates comprehensive CSV with all drivers
   - Includes folder name for file management
   - Enables bulk database operations

6. **Updated `create_summary_file()`**:

   - Groups by device name and version
   - Shows all hardware IDs
   - More detailed device information

7. **Removed `get_guid_readable_name()`**:
   - No longer needed with device-based organization
   - Simplified code structure

### Database Integration

The new structure is optimized for database upload:

**Suggested Database Schema**:

```sql
CREATE TABLE drivers (
    id INTEGER PRIMARY KEY AUTO_INCREMENT,
    device_name VARCHAR(255) NOT NULL,
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
    backup_date TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    zip_file_path VARCHAR(500),
    INDEX idx_device_name (device_name),
    INDEX idx_provider (provider),
    INDEX idx_driver_date (driver_date)
);
```

### Workflow Improvements

**For Database Administrators**:

1. Run backup tool
2. Import `all_drivers.csv` to database (one operation)
3. Zip each folder using the "Folder Name" column
4. Update database with zip file paths
5. Upload zip files to storage

**For Individual Driver Upload**:

1. Navigate to specific driver folder
2. Import that folder's `driver_info.csv`
3. Zip the folder
4. Link database record to zip file

### File Outputs

Each backup now creates:

1. **Multiple driver folders** named `DeviceName_Version`
2. **driver_info.csv** in each folder (for that driver package)
3. **all_drivers.csv** in root (master list)
4. **driver_backup_summary.txt** (human-readable documentation)

### Backward Compatibility

⚠️ **Breaking Changes**:

- Folder structure has changed significantly
- Previous GUID-based organization no longer used
- Folder names are now simpler and device-focused

### Benefits Summary

✅ **Database Ready**: CSV files are structured for direct import
✅ **Better Organization**: Drivers grouped by device name and version
✅ **Complete Information**: Includes Hardware ID and Device ID
✅ **Easier Management**: Simple folder names, ready to zip
✅ **Flexible Upload**: Support both individual and bulk database operations
✅ **Comprehensive Docs**: Multiple output formats for different needs

### Migration Guide

If you have existing backups with the old structure:

1. The new version creates a completely new structure
2. Old backups remain valid but use different organization
3. Consider running a fresh backup for new structure benefits
4. Old and new backups can coexist in different folders

### Testing Recommendations

Before production use:

1. Run with `--dry-run --verbose` to preview
2. Verify CSV format matches your database schema
3. Test import with a small subset
4. Validate zip file creation workflow
5. Ensure database indices are created

---

## Version 1.0.0 - Initial Release

- Basic driver backup functionality
- GUID-based folder organization
- Text summary output
- WMI driver detection
- Non-Microsoft driver filtering
