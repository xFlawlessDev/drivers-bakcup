# Quick Reference Card

## Essential Commands

```powershell
# Basic backup (default folder)
.\driver-backup.exe

# Custom location
.\driver-backup.exe backup --output "D:\Backups"

# Preview (no actual backup)
.\driver-backup.exe backup --dry-run --verbose

# Detailed logging
.\driver-backup.exe backup --verbose
```

## Output Structure

```
driver_backup/drivers_YYYYMMDD_HHMMSS/
├── Display/
│   └── NVIDIA GeForce RTX 3080_30.0.15.1179 Package/
│       ├── driver_info.csv      ← Import to DB
│       └── [driver files]       ← Zip this folder
├── Net/
│   └── Intel Ethernet_12.19.2.45 Package/
│       ├── driver_info.csv
│       └── [driver files]
├── Media/
│   └── Realtek Audio_6.0.9346.1 Package/
│       ├── driver_info.csv
│       └── [driver files]
├── all_drivers.csv              ← Master DB import
└── driver_backup_summary.txt    ← Review this first
```

## CSV Fields

| Field          | Database Type | Example                                |
| -------------- | ------------- | -------------------------------------- |
| Device Name    | VARCHAR(255)  | "NVIDIA GeForce RTX 3080"              |
| Driver Version | VARCHAR(50)   | "30.0.15.1179"                         |
| Driver Date    | DATE          | "2023-08-15"                           |
| Hardware ID    | VARCHAR(255)  | "PCI\VEN_10DE&DEV_2206"                |
| Device ID      | VARCHAR(255)  | "PCI\VEN*10DE&DEV*..."                 |
| INF Name       | VARCHAR(100)  | "oem1.inf"                             |
| Description    | TEXT          | "NVIDIA Graphics Driver"               |
| Provider       | VARCHAR(100)  | "NVIDIA"                               |
| Device Class   | VARCHAR(50)   | "Display"                              |
| Class GUID     | VARCHAR(50)   | "{4d36e968-...}"                       |
| Folder Name\*  | VARCHAR(255)  | "NVIDIA_GeForce_RTX_3080_30.0.15.1179" |

\*Only in all_drivers.csv

## Database Upload Workflow

### Option A: Bulk Upload

1. Import `all_drivers.csv` → database
2. For each row, zip the folder in "Folder Name" column
3. Upload zips to file storage
4. Update DB with zip paths

### Option B: Individual Upload

1. Select a driver folder
2. Import its `driver_info.csv` → database
3. Zip the folder
4. Upload zip and link to DB record

## Common Database Queries

```sql
-- List all drivers by provider
SELECT device_name, driver_version, provider
FROM drivers
ORDER BY provider, device_name;

-- Find outdated drivers (older than 1 year)
SELECT device_name, driver_version, driver_date
FROM drivers
WHERE driver_date < DATE_SUB(NOW(), INTERVAL 1 YEAR);

-- Count drivers by class
SELECT device_class, COUNT(*) as count
FROM drivers
GROUP BY device_class
ORDER BY count DESC;

-- Find specific device drivers
SELECT * FROM drivers
WHERE device_name LIKE '%NVIDIA%';
```

## Troubleshooting

| Issue                  | Solution                                      |
| ---------------------- | --------------------------------------------- |
| Admin privileges error | Right-click → Run as Administrator            |
| Export failed          | Check verbose output; may be protected driver |
| Path too long          | Use shorter output path like `C:\Backup`      |
| No drivers found       | Run with --verbose; check WMI service         |

## Best Practices

✅ Always run as Administrator
✅ Use --verbose for first run
✅ Review summary.txt before DB import
✅ Test with --dry-run first
✅ Keep historical CSVs
✅ Index DB on device_name, provider, driver_date

## Automation Template

```powershell
# Run backup
.\driver-backup.exe backup --output "D:\Backups" --verbose

# Find latest backup
$latest = Get-ChildItem "D:\Backups" | Sort LastWriteTime -Desc | Select -First 1

# Import to database (your tool here)
Import-CSV "$latest\all_drivers.csv" | Import-ToDatabase

# Zip folders
Get-ChildItem $latest -Directory | ForEach {
    Compress-Archive -Path $_.FullName -Dest "$($_.FullName).zip"
}
```

## File Sizes (Estimates)

- Per driver: 1-50 MB (varies widely)
- CSV files: < 1 MB typically
- Summary: < 500 KB
- Total backup: 100 MB - 5 GB (depends on driver count)

## Support Checklist

Before seeking help:

- [ ] Running as Administrator?
- [ ] Checked --verbose output?
- [ ] Reviewed summary.txt?
- [ ] WMI service running?
- [ ] Sufficient disk space?

---

**Remember**: Administrator rights required! 🛡️
**Tip**: Use `--dry-run --verbose` to preview before actual backup
