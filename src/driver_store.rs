//! Enumeration of the Windows driver store via `pnputil`.

use crate::models::DriverStorePackage;
use crate::util::is_microsoft_provider;
use std::collections::HashMap;
use std::process::Command;

/// Enumerate every driver package installed in the Windows driver store.
///
/// This is a superset of `Win32_PnPSignedDriver`: it also lists packages whose
/// device is not currently present (for example display-audio drivers that only
/// enumerate while an HDMI/display link is active), which would otherwise be
/// silently skipped by the backup.
pub fn get_driver_store_packages() -> Vec<DriverStorePackage> {
    let output = match Command::new("pnputil").arg("/enum-drivers").output() {
        Ok(output) => output,
        Err(e) => {
            eprintln!("Warning: could not run 'pnputil /enum-drivers': {}", e);
            return Vec::new();
        }
    };

    parse_enum_drivers(&String::from_utf8_lossy(&output.stdout))
}

/// Parse the output of `pnputil /enum-drivers` into packages.
pub fn parse_enum_drivers(stdout: &str) -> Vec<DriverStorePackage> {
    let mut packages: Vec<DriverStorePackage> = Vec::new();
    let mut current: Option<DriverStorePackage> = None;

    // Field values are prefixed with a label followed by ':'. Split only on the
    // first ':' so values that contain ':' (e.g. class GUIDs) stay intact.
    for line in stdout.lines() {
        let trimmed = line.trim();
        let (key, value) = match trimmed.split_once(':') {
            Some((k, v)) => (k.trim().to_lowercase(), v.trim().to_string()),
            None => continue,
        };

        match key.as_str() {
            "published name" => {
                if let Some(pkg) = current.take() {
                    packages.push(pkg);
                }
                current = Some(DriverStorePackage {
                    published_name: value.to_lowercase(),
                    original_name: None,
                    provider_name: None,
                    class_name: None,
                    class_guid: None,
                    driver_version: None,
                    driver_date: None,
                });
            }
            "original name" => {
                if let Some(ref mut pkg) = current {
                    pkg.original_name = Some(value);
                }
            }
            "provider name" => {
                if let Some(ref mut pkg) = current {
                    pkg.provider_name = Some(value);
                }
            }
            "class name" => {
                if let Some(ref mut pkg) = current {
                    pkg.class_name = Some(value);
                }
            }
            "class guid" => {
                if let Some(ref mut pkg) = current {
                    pkg.class_guid = Some(value);
                }
            }
            // `pnputil` prints "Driver Version:    06/16/2025 11.2.0.15"
            "driver version" => {
                if let Some(ref mut pkg) = current {
                    let mut parts = value.split_whitespace();
                    pkg.driver_date = parts.next().map(|s| s.to_string());
                    pkg.driver_version = parts.next().map(|s| s.to_string());
                }
            }
            _ => {}
        }
    }

    if let Some(pkg) = current.take() {
        packages.push(pkg);
    }

    packages
}

/// Select the non-Microsoft packages that are not bound to a present device.
pub fn select_unbound_non_microsoft<'a>(
    packages: &'a [DriverStorePackage],
    bound_oem_infs: &std::collections::HashSet<String>,
) -> Vec<&'a DriverStorePackage> {
    packages
        .iter()
        .filter(|pkg| !bound_oem_infs.contains(&pkg.published_name))
        .filter(|pkg| !is_microsoft_provider(pkg.provider_name.as_deref()))
        .collect()
}

/// Build a lookup table mapping published (OEM) INF names to original INF names.
pub fn build_inf_lookup() -> HashMap<String, String> {
    let output = Command::new("pnputil").arg("/enum-drivers").output();

    let mut lookup = HashMap::new();
    if let Ok(result) = output {
        for pkg in parse_enum_drivers(&String::from_utf8_lossy(&result.stdout)) {
            if let Some(original) = pkg.original_name {
                lookup.insert(pkg.published_name, original);
            }
        }
    }

    lookup
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const SAMPLE: &str = "\
Microsoft PnP Utility

Published Name:     oem0.inf
Original Name:      intcdaud.inf
Provider Name:      Intel(R) Corporation
Class Name:         MEDIA
Class GUID:         {4d36e96c-e325-11ce-bfc1-08002be10318}
Driver Version:     06/16/2025 11.2.0.15
Signer Name:        Microsoft Windows Hardware Compatibility Publisher

Published Name:     oem42.inf
Original Name:      iigd_dch.inf
Provider Name:      Intel Corporation
Class Name:         Display
Class GUID:         {4d36e968-e325-11ce-bfc1-08002be10318}
Driver Version:     06/17/2026 32.0.101.7088
";

    #[test]
    fn parses_two_packages() {
        let pkgs = parse_enum_drivers(SAMPLE);
        assert_eq!(pkgs.len(), 2);

        assert_eq!(pkgs[0].published_name, "oem0.inf");
        assert_eq!(pkgs[0].original_name.as_deref(), Some("intcdaud.inf"));
        assert_eq!(
            pkgs[0].provider_name.as_deref(),
            Some("Intel(R) Corporation")
        );
        assert_eq!(pkgs[0].class_name.as_deref(), Some("MEDIA"));
        assert_eq!(
            pkgs[0].class_guid.as_deref(),
            Some("{4d36e96c-e325-11ce-bfc1-08002be10318}")
        );
        assert_eq!(pkgs[0].driver_version.as_deref(), Some("11.2.0.15"));
        assert_eq!(pkgs[0].driver_date.as_deref(), Some("06/16/2025"));

        assert_eq!(pkgs[1].published_name, "oem42.inf");
        assert_eq!(pkgs[1].class_name.as_deref(), Some("Display"));
    }

    #[test]
    fn class_guid_keeps_colon_free_value_intact() {
        // The GUID contains a ':' in the original output? No - but the parser
        // must only split on the first ':' so values are not truncated.
        let pkgs = parse_enum_drivers(SAMPLE);
        assert!(pkgs[0]
            .class_guid
            .as_ref()
            .unwrap()
            .starts_with("{4d36e96c"));
    }

    #[test]
    fn selects_unbound_non_microsoft() {
        let pkgs = parse_enum_drivers(SAMPLE);
        let mut bound = HashSet::new();
        bound.insert("oem42.inf".to_string());

        let unbound = select_unbound_non_microsoft(&pkgs, &bound);
        assert_eq!(unbound.len(), 1);
        assert_eq!(unbound[0].published_name, "oem0.inf");
    }

    #[test]
    fn empty_output_yields_no_packages() {
        assert!(parse_enum_drivers("").is_empty());
    }
}
