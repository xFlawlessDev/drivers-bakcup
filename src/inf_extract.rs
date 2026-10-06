//! Extraction of driver installers into a temporary working directory.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Resolve a user-supplied path into a directory to scan.
///
/// Returns the working directory and whether it was created by extraction (and
/// therefore should be cleaned up by the caller).
pub fn extract_or_use_path(path: &Path, verbose: bool) -> Result<(PathBuf, bool)> {
    if path.is_dir() {
        return Ok((path.to_path_buf(), false));
    }

    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    match extension.as_str() {
        "exe" | "zip" | "7z" | "rar" => {
            let temp_dir =
                std::env::temp_dir().join(format!("driver_inspect_{}", std::process::id()));
            fs::create_dir_all(&temp_dir)
                .with_context(|| format!("Failed to create temp dir: {}", temp_dir.display()))?;

            if verbose {
                println!("Extracting {} to {}...", path.display(), temp_dir.display());
            }

            // Try 7z first, then fall back to PowerShell for .zip archives.
            let extract_result = extract_with_7z(path, &temp_dir)
                .or_else(|_| extract_with_powershell(path, &temp_dir));

            match extract_result {
                Ok(_) => {
                    if verbose {
                        println!("Successfully extracted to {}", temp_dir.display());
                    }
                    Ok((temp_dir, true))
                }
                Err(e) => {
                    let _ = fs::remove_dir_all(&temp_dir);
                    Err(e)
                }
            }
        }
        "inf" => Ok((path.parent().unwrap_or(Path::new(".")).to_path_buf(), false)),
        _ => bail!("Unsupported file type: {}", extension),
    }
}

fn extract_with_7z(archive: &Path, dest: &Path) -> Result<()> {
    let seven_zip_paths = [
        "7z",
        "C:\\Program Files\\7-Zip\\7z.exe",
        "C:\\Program Files (x86)\\7-Zip\\7z.exe",
    ];

    for seven_zip in &seven_zip_paths {
        let output = Command::new(seven_zip)
            .arg("x")
            .arg("-y")
            .arg(format!("-o{}", dest.display()))
            .arg(archive)
            .output();

        if let Ok(result) = output {
            if result.status.success() {
                return Ok(());
            }
        }
    }

    bail!("7-Zip not found or extraction failed")
}

fn extract_with_powershell(archive: &Path, dest: &Path) -> Result<()> {
    let extension = archive
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if extension == "zip" {
        let output = Command::new("powershell")
            .arg("-Command")
            .arg(format!(
                "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                archive.display(),
                dest.display()
            ))
            .output()?;

        if output.status.success() {
            return Ok(());
        }
    }

    bail!("PowerShell extraction failed or unsupported format")
}
