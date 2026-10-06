//! INF file discovery and parsing.

use crate::models::{InfDriverInfo, InfVersionInfo, ParsedInfFile};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Find all INF files under `dir`, recursively, sorted for stable output.
pub fn find_inf_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut inf_files = Vec::new();
    find_inf_files_recursive(dir, &mut inf_files)?;
    inf_files.sort();
    Ok(inf_files)
}

/// Find INF files directly inside `dir` (non-recursive), sorted.
pub fn find_inf_files_in_folder(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut inf_files = Vec::new();

    if !dir.is_dir() {
        return Ok(inf_files);
    }

    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_file() && has_inf_extension(&path) {
            inf_files.push(path);
        }
    }

    inf_files.sort();
    Ok(inf_files)
}

fn find_inf_files_recursive(dir: &Path, inf_files: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            find_inf_files_recursive(&path, inf_files)?;
        } else if has_inf_extension(&path) {
            inf_files.push(path);
        }
    }

    Ok(())
}

fn has_inf_extension(path: &Path) -> bool {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_lowercase() == "inf")
        .unwrap_or(false)
}

/// Parse a single INF file into structured driver information.
pub fn parse_inf_file(inf_path: &Path) -> Result<ParsedInfFile> {
    let content = read_inf_content(inf_path)?;

    let file_name = inf_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown.inf")
        .to_string();

    let mut version_info = InfVersionInfo::default();
    let mut manufacturers: HashMap<String, String> = HashMap::new();
    let mut device_sections: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut string_table: HashMap<String, String> = HashMap::new();
    let mut current_section = String::new();

    for line in content.lines() {
        let line = line.trim();

        if line.is_empty() || line.starts_with(';') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].to_lowercase();
            continue;
        }

        match current_section.as_str() {
            "version" => parse_version_line(line, &mut version_info),
            "manufacturer" => parse_manufacturer_line(line, &mut manufacturers),
            "strings" => parse_strings_line(line, &mut string_table),
            section if is_device_section(section, &manufacturers) => {
                parse_device_line(line, &current_section, &mut device_sections);
            }
            _ => {}
        }
    }

    let drivers = build_driver_info(
        &device_sections,
        &manufacturers,
        &string_table,
        &version_info,
        &file_name,
    );

    Ok(ParsedInfFile {
        file_path: inf_path.to_path_buf(),
        file_name,
        drivers,
        raw_version_info: version_info,
    })
}

/// True when a section name belongs to one of the manufacturer-declared
/// install sections.
fn is_device_section(section: &str, manufacturers: &HashMap<String, String>) -> bool {
    let sec_lower = section.to_lowercase();
    manufacturers.values().any(|value| {
        let base = value.split(',').next().unwrap_or(value);
        let base_lower = base.to_lowercase();
        sec_lower.starts_with(&base_lower) || base_lower.starts_with(&sec_lower)
    })
}

fn build_driver_info(
    device_sections: &HashMap<String, Vec<(String, String)>>,
    manufacturers: &HashMap<String, String>,
    string_table: &HashMap<String, String>,
    version_info: &InfVersionInfo,
    file_name: &str,
) -> Vec<InfDriverInfo> {
    let mut drivers = Vec::new();

    for (section_name, devices) in device_sections {
        for (device_desc, hardware_id) in devices {
            let resolved_desc = resolve_string(device_desc, string_table);
            let resolved_provider = version_info
                .provider
                .as_ref()
                .map(|p| resolve_string(p, string_table));

            let manufacturer = manufacturers
                .iter()
                .find(|(_, sec)| {
                    let base = sec.split(',').next().unwrap_or(sec);
                    section_name
                        .to_lowercase()
                        .starts_with(&base.to_lowercase())
                })
                .map(|(name, _)| resolve_string(name, string_table));

            drivers.push(InfDriverInfo {
                device_name: Some(resolved_desc.clone()),
                description: Some(resolved_desc),
                device_class: version_info.class.clone(),
                class_guid: version_info.class_guid.clone(),
                driver_version: version_info.driver_version.clone(),
                driver_date: version_info.driver_date.clone(),
                driver_provider_name: resolved_provider,
                hardware_id: Some(hardware_id.clone()),
                inf_name: Some(file_name.to_string()),
                catalog_file: version_info.catalog_file.clone(),
                manufacturer,
            });
        }
    }

    drivers
}

/// Read an INF file, handling UTF-16 (LE/BE) and UTF-8 BOMs, and falling back
/// to Latin-1 for undecodable bytes.
fn read_inf_content(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("Failed to read {}", path.display()))?;

    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        return Ok(decode_utf16(&bytes[2..], u16::from_le_bytes));
    }
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        return Ok(decode_utf16(&bytes[2..], u16::from_be_bytes));
    }
    if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
        return Ok(String::from_utf8_lossy(&bytes[3..]).to_string());
    }

    match String::from_utf8(bytes.clone()) {
        Ok(s) => Ok(s),
        Err(_) => Ok(bytes.iter().map(|&b| b as char).collect()),
    }
}

fn decode_utf16(bytes: &[u8], from_bytes: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes
        .chunks(2)
        .filter_map(|chunk| {
            if chunk.len() == 2 {
                Some(from_bytes([chunk[0], chunk[1]]))
            } else {
                None
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

fn parse_version_line(line: &str, version_info: &mut InfVersionInfo) {
    let parts: Vec<&str> = line.splitn(2, '=').collect();
    if parts.len() != 2 {
        return;
    }

    let key = parts[0].trim().to_lowercase();
    let value = parts[1].trim().trim_matches('"').to_string();

    match key.as_str() {
        "driverver" => {
            let dv_parts: Vec<&str> = value.splitn(2, ',').collect();
            if !dv_parts.is_empty() {
                version_info.driver_date = Some(dv_parts[0].trim().to_string());
            }
            if dv_parts.len() > 1 {
                version_info.driver_version = Some(dv_parts[1].trim().to_string());
            }
        }
        "class" => version_info.class = Some(value),
        "classguid" => version_info.class_guid = Some(value),
        "provider" => version_info.provider = Some(value),
        "catalogfile" | "catalogfile.nt" | "catalogfile.ntamd64" | "catalogfile.ntx86" => {
            version_info.catalog_file = Some(value);
        }
        _ => {}
    }
}

fn parse_manufacturer_line(line: &str, manufacturers: &mut HashMap<String, String>) {
    if let Some((name, section)) = split_kv(line) {
        manufacturers.insert(name, section);
    }
}

fn parse_device_line(
    line: &str,
    section: &str,
    device_sections: &mut HashMap<String, Vec<(String, String)>>,
) {
    let (device_desc, right_side) = match split_kv(line) {
        Some(parts) => parts,
        None => return,
    };

    // Format: InstallSection, HardwareID [, CompatibleID, ...]
    let hw_parts: Vec<&str> = right_side.split(',').collect();
    if hw_parts.len() >= 2 {
        let hardware_id = hw_parts[1].trim().to_string();
        if is_hardware_id(&hardware_id) {
            device_sections
                .entry(section.to_string())
                .or_default()
                .push((device_desc, hardware_id));
        }
    }
}

fn is_hardware_id(hardware_id: &str) -> bool {
    if hardware_id.is_empty() {
        return false;
    }
    let upper = hardware_id.to_uppercase();
    const PREFIXES: [&str; 7] = [
        "PCI\\",
        "USB\\",
        "HDAUDIO\\",
        "ACPI\\",
        "HID\\",
        "SWD\\",
        "ROOT\\",
    ];
    PREFIXES.iter().any(|p| upper.starts_with(p))
        || upper.contains("VEN_")
        || upper.contains("DEV_")
}

fn parse_strings_line(line: &str, string_table: &mut HashMap<String, String>) {
    if let Some((key, value)) = split_kv(line) {
        let value = value.trim_matches('"').to_string();
        string_table.insert(key, value);
    }
}

/// Split a `key = value` line into trimmed parts. Values keep any surrounding
/// quotes; callers strip them where appropriate.
fn split_kv(line: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = line.splitn(2, '=').collect();
    if parts.len() != 2 {
        return None;
    }
    Some((parts[0].trim().to_string(), parts[1].trim().to_string()))
}

/// Resolve a `%StringKey%` reference against the INF `[Strings]` table.
fn resolve_string(s: &str, string_table: &HashMap<String, String>) -> String {
    if s.starts_with('%') && s.ends_with('%') && s.len() > 2 {
        let key = &s[1..s.len() - 1];
        string_table
            .get(key)
            .cloned()
            .unwrap_or_else(|| s.to_string())
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(name: &str, content: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("drvtest_{}_{}", std::process::id(), name));
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    const SAMPLE_INF: &str = "\
[Version]
Class=Display
ClassGuid={4d36e968-e325-11ce-bfc1-08002be10318}
Provider=%Mfg%
DriverVer = 06/17/2026,32.0.101.7088
CatalogFile=igdlh.cat

[Manufacturer]
%Mfg%=Intel,NTamd64

[Intel.NTamd64]
%DeviceDesc% = Model1, PCI\\VEN_8086&DEV_4680

[Strings]
Mfg=\"Intel Corporation\"
DeviceDesc=\"Intel(R) UHD Graphics 770\"
";

    #[test]
    fn parses_version_and_device() {
        let path = write_temp("sample.inf", SAMPLE_INF);
        let parsed = parse_inf_file(&path).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(parsed.raw_version_info.class.as_deref(), Some("Display"));
        assert_eq!(
            parsed.raw_version_info.driver_version.as_deref(),
            Some("32.0.101.7088")
        );
        assert_eq!(
            parsed.raw_version_info.driver_date.as_deref(),
            Some("06/17/2026")
        );
        assert_eq!(
            parsed.raw_version_info.catalog_file.as_deref(),
            Some("igdlh.cat")
        );

        assert_eq!(parsed.drivers.len(), 1);
        let driver = &parsed.drivers[0];
        assert_eq!(
            driver.device_name.as_deref(),
            Some("Intel(R) UHD Graphics 770")
        );
        assert_eq!(
            driver.hardware_id.as_deref(),
            Some("PCI\\VEN_8086&DEV_4680")
        );
        assert_eq!(
            driver.driver_provider_name.as_deref(),
            Some("Intel Corporation")
        );
    }

    #[test]
    fn ignores_non_hardware_lines() {
        let path = write_temp("noise.inf", "[Version]\nClass=System\n");
        let parsed = parse_inf_file(&path).unwrap();
        let _ = fs::remove_file(&path);
        assert!(parsed.drivers.is_empty());
        assert_eq!(parsed.raw_version_info.class.as_deref(), Some("System"));
    }
}
