//! WMI access for present PnP devices.

use crate::models::PnPSignedDriver;
use anyhow::{Context, Result};
use wmi::{COMLibrary, WMIConnection};

/// A thin wrapper around a WMI connection.
pub struct WmiQuery {
    connection: WMIConnection,
}

impl WmiQuery {
    /// Initialize COM and connect to WMI.
    pub fn connect() -> Result<Self> {
        let com_con = COMLibrary::new().context("Failed to initialize COM library")?;
        let connection = WMIConnection::new(com_con).context("Failed to create WMI connection")?;
        Ok(Self { connection })
    }

    /// Query every signed driver for a present device.
    pub fn signed_drivers(&self) -> Result<Vec<PnPSignedDriver>> {
        let drivers: Vec<PnPSignedDriver> = self
            .connection
            .query()
            .context("Failed to query WMI for PnP signed drivers")?;
        Ok(drivers)
    }
}
