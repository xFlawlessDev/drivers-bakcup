//! Zip archive helpers.
//!
//! Creation prefers PowerShell's `Compress-Archive`, falling back to the .NET
//! `System.IO.Compression.ZipFile` API (which needs no PowerShell module).
//! Keeping this in PowerShell avoids adding a compression crate dependency; the
//! tool already shells out for `pnputil` and extraction.

use anyhow::{bail, Result};
use std::path::Path;
use std::process::Command;

/// Create a zip archive of `source_dir`.
///
/// `archive_path` is the file to create; the directory's *contents* are stored
/// at the archive root (extracting yields the files directly, without an extra
/// nesting level).
pub fn zip_directory(source_dir: &Path, archive_path: &Path, verbose: bool) -> Result<()> {
    if !source_dir.is_dir() {
        bail!("Not a directory: {}", source_dir.display());
    }

    // Remove an existing archive so the writer starts clean.
    if archive_path.exists() {
        let _ = std::fs::remove_file(archive_path);
    }

    let src = ps_quote(&source_dir.display().to_string());
    let dst = ps_quote(&archive_path.display().to_string());

    // Try Compress-Archive first, then the .NET ZipFile API.
    let script = format!(
        "$ErrorActionPreference='Stop';\
         try {{\
           Import-Module Microsoft.PowerShell.Archive -ErrorAction Stop;\
           Compress-Archive -Path '{src}\\*' -DestinationPath '{dst}' -Force -ErrorAction Stop\
         }} catch {{\
           Add-Type -AssemblyName System.IO.Compression.FileSystem;\
           if (Test-Path '{dst}') {{ Remove-Item -LiteralPath '{dst}' -Force }}\
           [System.IO.Compression.ZipFile]::CreateFromDirectory('{src}', '{dst}', \
             [System.IO.Compression.CompressionLevel]::Optimal, $false)\
         }}",
        src = src,
        dst = dst,
    );

    let output = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(&script)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to zip {}: {}", source_dir.display(), stderr.trim());
    }

    if !archive_path.is_file() {
        bail!(
            "Zip command reported success but no archive exists: {}",
            archive_path.display()
        );
    }

    if verbose {
        println!(
            "  Zipped {} -> {}",
            source_dir.display(),
            archive_path.display()
        );
    }

    Ok(())
}

/// Escape a value for single-quoted PowerShell strings.
fn ps_quote(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn zip_creates_nonempty_archive() {
        let base = std::env::temp_dir().join(format!("ziptest_{}", std::process::id()));
        let src = base.join("pkg");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), "hello world").unwrap();

        let archive = base.join("pkg.zip");
        zip_directory(&src, &archive, false).unwrap();

        assert!(archive.is_file(), "archive should be created");
        assert!(
            fs::metadata(&archive).unwrap().len() > 0,
            "archive should not be empty"
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn zip_rejects_non_directory() {
        let missing = std::env::temp_dir().join("ziptest_missing_dir_xyz");
        let archive = std::env::temp_dir().join("ziptest_missing.zip");
        assert!(zip_directory(&missing, &archive, false).is_err());
    }
}
