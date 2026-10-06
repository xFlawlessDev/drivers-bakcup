//! Data structures shared across the tool.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Parsed driver information from an INF file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfDriverInfo {
    pub device_name: Option<String>,
    pub description: Option<String>,
    pub device_class: Option<String>,
    pub class_guid: Option<String>,
    pub driver_version: Option<String>,
    pub driver_date: Option<String>,
    pub driver_provider_name: Option<String>,
    pub hardware_id: Option<String>,
    pub inf_name: Option<String>,
    pub catalog_file: Option<String>,
    pub manufacturer: Option<String>,
}

/// A parsed INF file and the devices it supports.
#[derive(Debug, Clone)]
pub struct ParsedInfFile {
    pub file_path: PathBuf,
    pub file_name: String,
    pub drivers: Vec<InfDriverInfo>,
    pub raw_version_info: InfVersionInfo,
}

/// The `[Version]` section of an INF file.
#[derive(Debug, Clone, Default)]
pub struct InfVersionInfo {
    pub driver_version: Option<String>,
    pub driver_date: Option<String>,
    pub class: Option<String>,
    pub class_guid: Option<String>,
    pub provider: Option<String>,
    pub catalog_file: Option<String>,
}

/// A device as reported by `Win32_PnPSignedDriver`.
#[derive(Deserialize, Debug, Clone)]
#[serde(rename = "Win32_PnPSignedDriver")]
pub struct PnPSignedDriver {
    #[serde(rename = "ClassGuid")]
    pub class_guid: Option<String>,

    #[serde(rename = "Description")]
    pub description: Option<String>,

    #[serde(rename = "DeviceClass")]
    pub device_class: Option<String>,

    #[serde(rename = "DeviceName")]
    pub device_name: Option<String>,

    #[serde(rename = "DriverDate")]
    pub driver_date: Option<String>,

    #[serde(rename = "DriverProviderName")]
    pub driver_provider_name: Option<String>,

    #[serde(rename = "DriverVersion")]
    pub driver_version: Option<String>,

    #[serde(rename = "InfName")]
    pub inf_name: Option<String>,

    #[serde(rename = "HardwareID")]
    pub hardware_id: Option<String>,

    #[serde(rename = "DeviceID")]
    pub device_id: Option<String>,
}

/// A driver package installed in the Windows driver store, as reported by
/// `pnputil /enum-drivers`. Unlike [`PnPSignedDriver`], this includes packages
/// that are not currently bound to a present device (for example display-audio
/// drivers that only enumerate while an HDMI/display link is active).
#[derive(Debug, Clone)]
pub struct DriverStorePackage {
    /// Published (OEM) INF name, e.g. `oem0.inf`.
    pub published_name: String,
    /// Original INF file name inside the package, e.g. `intcdaud.inf`.
    pub original_name: Option<String>,
    pub provider_name: Option<String>,
    pub class_name: Option<String>,
    pub class_guid: Option<String>,
    pub driver_version: Option<String>,
    pub driver_date: Option<String>,
}

/// One row of the device class legend:
/// `(class name, class GUID, present device count, installed package count)`.
pub type DeviceClassLegendEntry = (String, String, usize, usize);
