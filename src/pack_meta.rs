//! Best-effort detection of driver pack metadata (ODM, model, revision).
//!
//! Vendor packs vary widely. Detection looks, in order, at:
//! 1. Explicit CLI overrides (handled by the caller).
//! 2. `autoins.ini` at the pack root, which commonly carries the revision.
//! 3. Folder names in the pack path, e.g. `D:\Driver MAP\765\V2xxRNP_1.08.06_0911`.

use crate::models::PackMetadata;
use std::fs;
use std::path::Path;

/// Detect metadata for the pack rooted at `source`.
///
/// `source` may be the `Drivers` sub-folder or the pack root; both are checked
/// for `autoins.ini`.
pub fn detect_metadata(source: &Path) -> PackMetadata {
    let mut meta = PackMetadata::default();

    if let Some(ini) = find_autoins(source) {
        apply_autoins(&ini, &mut meta);
    }

    // Start from the pack root (the folder above `Drivers`, when present) so
    // the model folder is a direct parent rather than the `Drivers` folder.
    let root = pack_root(source).unwrap_or_else(|| source.to_path_buf());
    infer_from_path(&root, &mut meta);

    // If the revision ended up as a bare version (e.g. `1.08.06` from
    // autoins.ini) but a richer revision folder exists, prefer the folder name.
    if let Some(rev) = &meta.revision {
        if rev.chars().all(|c| c.is_ascii_digit() || c == '.') {
            if let Some(folder_rev) = revision_from_path(&root) {
                meta.revision = Some(folder_rev);
            }
        }
    }

    meta
}

/// The pack root: the parent of a `Drivers` component, if `source` is inside one.
fn pack_root(source: &Path) -> Option<std::path::PathBuf> {
    let components: Vec<_> = source.components().collect();
    let idx = components.iter().rposition(|c| {
        c.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("Drivers")
    })?;
    if idx == 0 {
        return None;
    }
    let mut root = std::path::PathBuf::new();
    for c in &components[..idx] {
        root.push(c.as_os_str());
    }
    Some(root)
}

/// Find the first path component that looks like a revision token.
fn revision_from_path(path: &Path) -> Option<String> {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .rev()
        .find(|name| looks_like_revision(name))
}

/// Find `autoins.ini`, checking `source` then its parents.
fn find_autoins(source: &Path) -> Option<std::path::PathBuf> {
    let mut current = Some(source);
    while let Some(dir) = current {
        let candidate = dir.join("autoins.ini");
        if candidate.is_file() {
            return Some(candidate);
        }
        current = dir.parent();
    }
    None
}

/// Read known keys from `autoins.ini` into the metadata.
fn apply_autoins(path: &Path, meta: &mut PackMetadata) {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        let key = key.trim().to_lowercase();
        let value = value.trim().trim_matches('"').trim().to_string();
        if value.is_empty() {
            continue;
        }

        match key.as_str() {
            // `CDVersion` is the pack revision in the observed Clevo layout.
            "cdversion" | "version" | "revision" if meta.revision.is_none() => {
                meta.revision = Some(value);
            }
            "odm" | "oem" if meta.odm.is_none() => meta.odm = Some(value),
            "model" if meta.model.is_none() => meta.model = Some(value),
            _ => {}
        }
    }
}

/// Infer ODM/model/revision from parent folder names when still unknown.
fn infer_from_path(source: &Path, meta: &mut PackMetadata) {
    let components: Vec<String> = source
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();

    // Find the deepest folder that looks like a revision (`V2xxRNP_1.08.06_0911`).
    let revision_idx = components.iter().rposition(|c| looks_like_revision(c));

    // A revision looks like `V2xxRNP_1.08.06_0911`: contains an underscore and a
    // digit, and is not a generic folder name.
    if meta.revision.is_none() {
        if let Some(idx) = revision_idx {
            meta.revision = Some(components[idx].clone());
        }
    }

    // The model is the folder immediately above the revision folder (the layout
    // is typically `<...>/<model>/<revision>/...`).
    if meta.model.is_none() {
        if let Some(idx) = revision_idx {
            if idx > 0 {
                let candidate = &components[idx - 1];
                if !candidate.is_empty() && !looks_like_revision(candidate) {
                    meta.model = Some(candidate.clone());
                }
            }
        }
    }

    // Fallback for packs named `<ODM><Model>_<Model>_Drivers` (e.g.
    // `GemOne238TJ_270XJ_Drivers`), which must be parsed before the generic
    // folder-above-`Drivers` fallback.
    if let Some(name) = model_from_pack_folder(&components) {
        if meta.model.is_none() {
            meta.model = Some(name);
        }
        if meta.odm.is_none() {
            meta.odm = odm_from_pack_folder(&components);
        }
    }

    // Fallback: the folder named just above a `Drivers` component, skipping the
    // revision folder and pack-name folders.
    if meta.model.is_none() {
        if let Some(idx) = components
            .iter()
            .rposition(|c| c.eq_ignore_ascii_case("Drivers"))
        {
            let mut candidate_idx = idx.checked_sub(1);
            while let Some(i) = candidate_idx {
                let candidate = &components[i];
                if !candidate.is_empty()
                    && !looks_like_revision(candidate)
                    && !candidate.to_lowercase().ends_with("_drivers")
                {
                    meta.model = Some(candidate.clone());
                    break;
                }
                candidate_idx = i.checked_sub(1);
            }
        }
    }
}

/// Extract an ODM prefix from a folder like `GemOne238TJ_270XJ_Drivers`.
fn odm_from_pack_folder(components: &[String]) -> Option<String> {
    for name in components.iter().rev() {
        let lower = name.to_lowercase();
        if lower.ends_with("_drivers") && lower != "drivers" {
            let stem = &name[..name.len() - "_Drivers".len()];
            if let Some((odm, _model)) = stem.split_once('_') {
                if !odm.is_empty() {
                    return Some(odm.to_string());
                }
            }
        }
    }
    None
}

/// Extract a model identifier from a folder like `GemOne238TJ_270XJ_Drivers`.
fn model_from_pack_folder(components: &[String]) -> Option<String> {
    for name in components.iter().rev() {
        let lower = name.to_lowercase();
        if lower.ends_with("_drivers") && lower != "drivers" {
            let stem = &name[..name.len() - "_Drivers".len()];
            if let Some((odm, model)) = stem.split_once('_') {
                if !odm.is_empty() && !model.is_empty() {
                    return Some(model.to_string());
                }
            }
            if !stem.is_empty() {
                return Some(stem.to_string());
            }
        }
    }
    None
}

/// Heuristic: a revision token contains an underscore and a digit, and is not a
/// generic folder name. Tokens ending in `_Drivers`/`_Driver` are path noise, not
/// revisions.
fn looks_like_revision(name: &str) -> bool {
    let lower = name.to_lowercase();
    if lower.ends_with("_drivers") || lower == "drivers" {
        return false;
    }
    name.contains('_') && name.chars().any(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn detects_revision_and_model_from_path() {
        let src = Path::new("D:\\Driver MAP\\765\\V2xxRNP_1.08.06_0911\\Drivers");
        let mut meta = PackMetadata::default();
        let root = pack_root(src).unwrap();
        infer_from_path(&root, &mut meta);
        assert_eq!(meta.revision.as_deref(), Some("V2xxRNP_1.08.06_0911"));
        assert_eq!(meta.model.as_deref(), Some("765"));
    }

    #[test]
    fn pack_root_strips_drivers_component() {
        let src = Path::new("D:\\Driver MAP\\765\\V2xxRNP_1.08.06_0911\\Drivers");
        let root = pack_root(src).unwrap();
        assert!(root.ends_with("V2xxRNP_1.08.06_0911"));
        assert!(!root.to_string_lossy().to_lowercase().ends_with("drivers"));
    }

    #[test]
    fn detects_odm_and_model_from_flat_pack_name() {
        let src = Path::new("D:\\Driver MAP\\GemOne238TJ_270XJ_Drivers\\Drivers");
        let mut meta = PackMetadata::default();
        infer_from_path(src, &mut meta);
        assert_eq!(meta.model.as_deref(), Some("270XJ"));
        assert_eq!(meta.odm.as_deref(), Some("GemOne238TJ"));
    }

    #[test]
    fn reads_autoins_cdversion() {
        let dir = std::env::temp_dir().join(format!("packmeta_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let ini = dir.join("autoins.ini");
        let mut f = fs::File::create(&ini).unwrap();
        f.write_all(b"[Platform]\nCDVersion=\"1.08.06\"\n").unwrap();

        let mut meta = PackMetadata::default();
        apply_autoins(&ini, &mut meta);
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(meta.revision.as_deref(), Some("1.08.06"));
    }

    #[test]
    fn revision_heuristic() {
        assert!(looks_like_revision("V2xxRNP_1.08.06_0911"));
        assert!(!looks_like_revision("765"));
        assert!(!looks_like_revision("Drivers"));
    }
}
