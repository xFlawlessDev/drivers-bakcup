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

    // Pass 1: collect the `[Strings]` table. Manufacturer and device sections can
    // reference it (`%Wintun.Name%`), and `[Strings]` usually appears last.
    let mut current_section = String::new();
    for raw in content.lines() {
        let line = strip_trailing_comment(raw.trim());
        if line.is_empty() {
            continue;
        }
        if let Some(section) = section_name(line) {
            current_section = section;
            continue;
        }
        if current_section == "strings" {
            parse_strings_line(line, &mut string_table);
        }
    }

    // Pass 2: parse version, manufacturer and device sections with the string
    // table available for reference resolution.
    current_section.clear();

    for line in content.lines() {
        let line = line.trim();

        if line.is_empty() || line.starts_with(';') {
            continue;
        }

        // Section headers may carry a trailing comment, e.g.
        // `[Intel.NTamd64.10.0...17763] ; Win10 RS5, 19H1`.
        if let Some(section) = section_name(line) {
            current_section = section;
            continue;
        }

        // Strip a trailing `;` comment from data lines (but not inside quotes).
        let line = strip_trailing_comment(line);

        match current_section.as_str() {
            "version" => parse_version_line(line, &mut version_info),
            "manufacturer" => parse_manufacturer_line(line, &mut manufacturers),
            "strings" => {} // already collected in pass 1
            section if is_device_section(section, &manufacturers, &string_table) => {
                parse_device_line(line, &current_section, &mut device_sections);
            }
            _ => {}
        }
    }

    let mut drivers = build_driver_info(
        &device_sections,
        &manufacturers,
        &string_table,
        &version_info,
        &file_name,
    );

    // Resolve `%StringKey%` references in the `[Version]` section (notably
    // `Provider=%ManufacturerName%`) against the `[Strings]` table.
    if let Some(provider) = &version_info.provider {
        version_info.provider = Some(resolve_string(provider, &string_table));
    }
    if let Some(class) = &version_info.class {
        version_info.class = Some(resolve_string(class, &string_table));
    }
    if let Some(catalog) = &version_info.catalog_file {
        version_info.catalog_file = Some(resolve_string(catalog, &string_table));
    }

    // Keep per-device provider consistent with the resolved version provider.
    if let Some(provider) = &version_info.provider {
        for driver in &mut drivers {
            if driver
                .driver_provider_name
                .as_deref()
                .map(|p| p.starts_with('%') && p.ends_with('%'))
                .unwrap_or(true)
            {
                driver.driver_provider_name = Some(provider.clone());
            }
        }
    }

    Ok(ParsedInfFile {
        file_path: inf_path.to_path_buf(),
        file_name,
        drivers,
        raw_version_info: version_info,
    })
}

/// Return the lowercased section name when the line is a `[Section]` header.
///
/// A section header may carry a trailing comment, e.g.
/// `[Intel.NTamd64.10.0...17763] ; Win10 RS5`.
fn section_name(line: &str) -> Option<String> {
    if !line.starts_with('[') {
        return None;
    }
    let end = line.find(']')?;
    Some(line[1..end].trim().to_lowercase())
}

/// Remove a trailing `;` comment from an INF line, ignoring `;` inside quotes.
fn strip_trailing_comment(line: &str) -> &str {
    let mut in_quotes = false;
    for (idx, ch) in line.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ';' if !in_quotes => return line[..idx].trim_end(),
            _ => {}
        }
    }
    line
}

/// True when a section is a Models (device) section.
///
/// `[Manufacturer]` maps a name to a base section, e.g. `%Mfg%=Wintun, NTamd64`
/// declares the Models sections `[Wintun]` and `[Wintun.NTamd64]`. Manufacturer
/// entries can reference strings, so references are resolved before comparing.
///
/// Only the declared base (and its OS-decorated variants) is treated as a device
/// section, so unrelated sections such as `[Wintun.Install]` are not matched.
fn is_device_section(
    section: &str,
    manufacturers: &HashMap<String, String>,
    string_table: &HashMap<String, String>,
) -> bool {
    let sec = section.trim().to_lowercase();

    for value in manufacturers.values() {
        // The first comma-separated field is the base section name.
        let base = value.split(',').next().unwrap_or(value);
        let base = resolve_string(base.trim(), string_table);
        let base = base.trim().trim_matches('"').to_lowercase();
        if base.is_empty() {
            continue;
        }

        // `[Base]` or `[Base.NTamd64]`, but not `[Base.Install]`.
        if sec == base {
            return true;
        }
        if let Some(suffix) = sec.strip_prefix(&format!("{}.", base)) {
            if is_os_target(suffix) {
                return true;
            }
        }
    }

    false
}

/// True for an INF target-OS decoration such as `ntamd64`, `ntx86`, `ntarm64`,
/// `ntamd64.10.0...17763`, or a bare version like `6.0`.
fn is_os_target(suffix: &str) -> bool {
    let s = suffix.trim();
    s.starts_with("nt")
        || s.starts_with("ntamd64")
        || s.starts_with("ntx86")
        || s.starts_with("ntarm")
        || s.chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
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
                    let base = resolve_string(base.trim(), string_table);
                    let base = base.trim().trim_matches('"').to_lowercase();
                    !base.is_empty()
                        && (section_name.to_lowercase() == base
                            || section_name
                                .to_lowercase()
                                .starts_with(&format!("{}.", base)))
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
                hardware_id: if hardware_id.trim().is_empty() {
                    None
                } else {
                    Some(hardware_id.clone())
                },
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
    let value = unquote_inf_value(parts[1]);

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

    // Format: InstallSection, HardwareID [, CompatibleID, ...].
    // The description is often quoted (`"Send to OneNote" = ...`), so unquote it.
    let device_desc = unquote_inf_value(&device_desc);
    let hw_parts: Vec<&str> = right_side.split(',').collect();
    if hw_parts.len() < 2 {
        return;
    }

    // The hardware ID is the second field. Older v3 print INFs leave it empty and
    // put a compatible ID / driver name in the third, e.g.
    // `"Adobe PDF Converter" = WTPRNT.PPD, ,Adobe PDF`.
    let hardware_id = unquote_inf_value(hw_parts[1]);
    if is_hardware_id(&hardware_id) {
        device_sections
            .entry(section.to_string())
            .or_default()
            .push((device_desc, hardware_id));
        return;
    }

    // Fall back to the first field that looks like a hardware ID. This also
    // covers INFs that list only a compatible ID.
    for part in hw_parts.iter().skip(2) {
        let candidate = unquote_inf_value(part);
        if is_hardware_id(&candidate) {
            device_sections
                .entry(section.to_string())
                .or_default()
                .push((device_desc, candidate));
            return;
        }
    }

    // No hardware ID at all: still record the device with an empty ID so its
    // name and count are captured (e.g. v3 print drivers keyed by DataFile).
    if !device_desc.is_empty() {
        device_sections
            .entry(section.to_string())
            .or_default()
            .push((device_desc, String::new()));
    }
}

/// Device enumerator prefixes used by Windows hardware IDs.
///
/// `SWC\` identifies Software Components; its IDs are INF-supplied strings that
/// the system prefixes with `SWC\` (see *INF AddComponent Directive*).
/// `INTELAUDIO\` is the Intel Smart Sound (SST) audio bus enumerator.
const HWID_PREFIXES: [&str; 12] = [
    "PCI\\",
    "USB\\",
    "HDAUDIO\\",
    "INTELAUDIO\\",
    "ACPI\\",
    "HID\\",
    "SWD\\",
    "ROOT\\",
    "SWC\\",
    "SCSI\\",
    "MONITOR\\",
    "DISPLAY\\",
];

/// True when a Models-section hardware-ID slot holds a real device ID.
///
/// INF `Models` entries have the form `desc = install-section, hwid[, compat-id...]`.
/// The hardware ID can be:
/// * a bus-prefixed ID (`PCI\...`, `USB\...`, `SWC\...`, `INTELAUDIO\...`);
/// * a root-enumerated ID (`*VMnetAdapter0`, `Wintun`, `vmware_bridge`);
/// * a bare GUID (`{3EE39114-...}`), as used by some print INFs.
///
/// Values that clearly are not IDs (a bare number, a bare file name, an install
/// section name) are rejected so unrelated lines are not mistaken for devices.
fn is_hardware_id(hardware_id: &str) -> bool {
    let id = hardware_id.trim();
    if id.is_empty() {
        return false;
    }

    let upper = id.to_uppercase();
    if HWID_PREFIXES.iter().any(|p| upper.starts_with(p)) {
        return true;
    }
    if upper.starts_with('*') {
        // Root-enumerated compatible ID, e.g. `*VMnetAdapter0`.
        return id.len() > 1;
    }
    if upper.contains("VEN_") || upper.contains("DEV_") {
        return true;
    }
    if is_guid(id) {
        return true;
    }

    is_vendor_defined_id(id)
}

/// Heuristic for root-enumerated, vendor-defined hardware IDs such as `Wintun`
/// or `vmware_bridge`.
///
/// These contain no spaces, no path separators, and at least one letter; they are
/// not numbers, file names, or section names.
fn is_vendor_defined_id(id: &str) -> bool {
    if id.len() < 3 || id.len() > 200 {
        return false;
    }
    if id.contains([' ', '\\', '/', ',', '"']) {
        return false;
    }
    if id.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    // Reject file names and section-looking values.
    let lower = id.to_lowercase();
    if lower.ends_with(".inf") || lower.ends_with(".sys") || lower.ends_with(".dll") {
        return false;
    }
    if lower.ends_with("install") || lower.contains("install.") {
        return false;
    }
    // Must contain an ASCII letter.
    id.chars().any(|c| c.is_ascii_alphabetic())
}

/// True when the value is a single GUID, optionally brace-wrapped.
fn is_guid(value: &str) -> bool {
    let inner = value
        .trim()
        .strip_prefix('{')
        .and_then(|v| v.strip_suffix('}'))
        .unwrap_or(value.trim());

    if inner.len() != 36 {
        return false;
    }

    // 8-4-4-4-12 hexadecimal groups.
    inner.split('-').map(|g| g.len()).eq([8, 4, 4, 4, 12])
        && inner.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

fn parse_strings_line(line: &str, string_table: &mut HashMap<String, String>) {
    if let Some((key, value)) = split_kv(line) {
        string_table.insert(key, unquote_inf_value(&value));
    }
}

/// Unquote an INF value, joining adjacent quoted segments.
///
/// INF files may concatenate quoted strings, e.g. `"Intel(R) UHD Graphics" "710"`,
/// which Windows renders as `Intel(R) UHD Graphics710`. The whitespace between a
/// closing and an opening quote is dropped; whitespace inside quotes is kept.
fn unquote_inf_value(value: &str) -> String {
    let value = value.trim();

    // Fast path: a single unquoted token.
    if !value.contains('"') {
        return value.to_string();
    }

    let mut result = String::new();
    let mut in_quotes = false;
    for ch in value.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            _ => {
                // Drop whitespace that sits between two quoted segments.
                let between_segments = !in_quotes && ch.is_whitespace() && value.contains('"');
                if !between_segments {
                    result.push(ch);
                }
            }
        }
    }

    // Collapse any surrounding whitespace from the last unquoted run.
    result.trim().to_string()
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

    #[test]
    fn joins_adjacent_quoted_strings() {
        // Intel INFs write `"Intel(R) UHD Graphics" "710"`.
        let inf = "\
[Version]
Class=Display
Provider=\"Intel\"

[Manufacturer]
%Mfg%=Intel

[Intel]
%DeviceDesc% = Model1, PCI\\VEN_8086&DEV_1234

[Strings]
Mfg=\"Intel\"
DeviceDesc=\"Intel(R) UHD Graphics\" \"710\"
";
        let path = write_temp("adjacent.inf", inf);
        let parsed = parse_inf_file(&path).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(
            parsed.drivers[0].device_name.as_deref(),
            Some("Intel(R) UHD Graphics710")
        );
    }

    #[test]
    fn unquote_handles_plain_and_quoted() {
        assert_eq!(unquote_inf_value("Intel"), "Intel");
        assert_eq!(unquote_inf_value("\"Intel\""), "Intel");
        assert_eq!(unquote_inf_value("\"A\" \"B\""), "AB");
    }

    #[test]
    fn accepts_known_hardware_id_prefixes() {
        assert!(is_hardware_id("PCI\\VEN_8086&DEV_4680"));
        assert!(is_hardware_id("USB\\VID_8087&PID_0025"));
        assert!(is_hardware_id("HDAUDIO\\FUNC_01&VEN_8086"));
        assert!(is_hardware_id("ACPI\\INTC10B0"));
        assert!(is_hardware_id("HID\\VID_046D"));
        assert!(is_hardware_id("SWD\\COMPUTER"));
        assert!(is_hardware_id("ROOT\\bertreader"));
    }

    #[test]
    fn accepts_software_component_hardware_ids() {
        // Software Components use the `SWC\` prefix (INF AddComponent).
        assert!(is_hardware_id("SWC\\3C4852D6-D47B-4F46-B05E-B5EDC1AA440E"));
        assert!(is_hardware_id("SWC\\VID0001&PID0001"));
    }

    #[test]
    fn accepts_bare_guid_hardware_ids() {
        assert!(is_hardware_id("{3EE39114-30B4-45a4-A109-19D4A40FCC22}"));
        assert!(is_hardware_id("3EE39114-30B4-45a4-A109-19D4A40FCC22"));
    }

    #[test]
    fn rejects_values_that_are_not_hardware_ids() {
        assert!(!is_hardware_id(""));
        assert!(!is_hardware_id("2"));
        assert!(!is_hardware_id("wintun.sys"));
        assert!(!is_hardware_id("VMnetAdapter0.Install"));
        assert!(!is_hardware_id("some file name"));
    }

    #[test]
    fn accepts_root_enumerated_and_vendor_ids() {
        // Microsoft root-enumerated compatible IDs and vendor-defined IDs.
        assert!(is_hardware_id("*VMnetAdapter0"));
        assert!(is_hardware_id("Wintun"));
        assert!(is_hardware_id("vmware_bridge"));
    }

    #[test]
    fn accepts_intel_audio_bus_hardware_ids() {
        assert!(is_hardware_id("INTELAUDIO\\SDW_ROOT"));
        assert!(is_hardware_id("INTELAUDIO\\FUNC_01&VEN_8086&DEV_281B"));
    }

    #[test]
    fn strips_trailing_comments_only_outside_quotes() {
        assert_eq!(strip_trailing_comment("A = B ; note"), "A = B");
        assert_eq!(strip_trailing_comment("A = \"x;y\""), "A = \"x;y\"");
        assert_eq!(strip_trailing_comment("A = B"), "A = B");
    }

    #[test]
    fn parses_section_header_with_trailing_comment() {
        let inf = "\
[Version]
Class=SoftwareComponent
Provider=\"Intel\"

[Manufacturer]
%Mfg%=Intel, NTamd64

[Intel.NTamd64.10.0...17763] ; Win10 RS5, 19H1
%Location% = InstallSection, SWC\\PROVIDER_INTEL_COMPONENT_ICLSCLIENT

[Strings]
Mfg=\"Intel\"
Location=\"Intel(R) iCLS Client\"
";
        let path = write_temp("swc_comment.inf", inf);
        let parsed = parse_inf_file(&path).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(
            parsed.drivers.len(),
            1,
            "device line after a commented header"
        );
        assert_eq!(
            parsed.drivers[0].hardware_id.as_deref(),
            Some("SWC\\PROVIDER_INTEL_COMPONENT_ICLSCLIENT")
        );
        assert_eq!(
            parsed.drivers[0].device_name.as_deref(),
            Some("Intel(R) iCLS Client")
        );
    }

    #[test]
    fn parses_manufacturer_referenced_by_string() {
        // `%Company% = %Name%, NTamd64` where the section is `[Wintun.NTamd64]`.
        let inf = "\
[Version]
Class=Net
Provider=\"WireGuard LLC\"

[Manufacturer]
%Company% = %Name%, NTamd64

[Wintun.NTamd64]
%DeviceDesc% = Wintun.Install, Wintun

[Strings]
Company=\"WireGuard LLC\"
Name=\"Wintun\"
DeviceDesc=\"Wintun Userspace Tunnel\"
";
        let path = write_temp("wintun.inf", inf);
        let parsed = parse_inf_file(&path).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(
            parsed.drivers.len(),
            1,
            "device line under a referenced manufacturer"
        );
        assert_eq!(
            parsed.drivers[0].device_name.as_deref(),
            Some("Wintun Userspace Tunnel")
        );
        assert_eq!(parsed.drivers[0].hardware_id.as_deref(), Some("Wintun"));
    }

    #[test]
    fn parses_printer_style_line_without_hardware_id() {
        // v3 print INF: `desc = section, ,compat` (hardware ID slot empty).
        let inf = "\
[Version]
Class=Printer
Provider=\"Adobe\"

[Manufacturer]
%Adobe%=Adobe,NTamd64

[Adobe.NTamd64]
\"Adobe PDF Converter\" = WTPRNT.PPD, ,Adobe PDF

[Strings]
Adobe=\"Adobe\"
";
        let path = write_temp("adobe.inf", inf);
        let parsed = parse_inf_file(&path).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(parsed.drivers.len(), 1);
        assert_eq!(
            parsed.drivers[0].device_name.as_deref(),
            Some("Adobe PDF Converter")
        );
        // No hardware ID, but the device is still captured.
        assert_eq!(parsed.drivers[0].hardware_id, None);
    }

    #[test]
    fn parses_quoted_device_description() {
        let inf = "\
[Version]
Class=Printer
Provider=\"Microsoft\"

[Manufacturer]
%Mfg%=Microsoft, NTamd64

[Microsoft.NTamd64]
\"Send to Microsoft OneNote 16 Driver\" = Install,{3EE39114-30B4-45a4-A109-19D4A40FCC22}

[Strings]
Mfg=\"Microsoft\"
";
        let path = write_temp("quoted_desc.inf", inf);
        let parsed = parse_inf_file(&path).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(parsed.drivers.len(), 1);
        assert_eq!(
            parsed.drivers[0].device_name.as_deref(),
            Some("Send to Microsoft OneNote 16 Driver")
        );
        assert_eq!(
            parsed.drivers[0].hardware_id.as_deref(),
            Some("{3EE39114-30B4-45a4-A109-19D4A40FCC22}")
        );
    }
}
