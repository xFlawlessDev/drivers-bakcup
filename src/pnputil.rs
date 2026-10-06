//! Thin wrapper around `pnputil /export-driver`.

use std::path::Path;
use std::process::Command;

/// Outcome of an export attempt.
pub enum ExportOutcome {
    Success,
    Failed(String),
}

/// Export a published driver package into `dest`.
pub fn export_driver(published_name: &str, dest: &Path) -> ExportOutcome {
    match Command::new("pnputil")
        .arg("/export-driver")
        .arg(published_name)
        .arg(dest)
        .output()
    {
        Ok(output) if output.status.success() => ExportOutcome::Success,
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);

            let mut message = format!("exit code {}", exit_code);
            if !stdout.trim().is_empty() {
                message.push_str(&format!("; stdout: {}", stdout.trim()));
            }
            if !stderr.trim().is_empty() {
                message.push_str(&format!("; stderr: {}", stderr.trim()));
            }

            // Surface the most common, actionable causes.
            let stderr_lower = stderr.to_lowercase();
            let stdout_lower = stdout.to_lowercase();
            let hint = if stderr_lower.contains("access") || stderr_lower.contains("denied") {
                Some("permissions issue - try running as Administrator")
            } else if stderr_lower.contains("not found") || stderr_lower.contains("cannot find") {
                Some("driver package may be corrupted or already removed")
            } else if stdout_lower.contains("missing or invalid target directory")
                || exit_code == 87
            {
                Some("path too long or invalid")
            } else if stdout_lower.contains("the data is invalid") || exit_code == 13 {
                Some("driver may be protected or corrupted")
            } else {
                None
            };

            if let Some(hint) = hint {
                message.push_str(&format!(" ({})", hint));
            }
            ExportOutcome::Failed(message)
        }
        Err(e) => ExportOutcome::Failed(format!(
            "could not run pnputil ({}) - ensure it is on PATH and run as Administrator",
            e
        )),
    }
}
