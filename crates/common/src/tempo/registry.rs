use eyre::Result;
use serde::{Serialize, de::DeserializeOwned};
use std::{fs, io::Write, path::Path};

/// Shared TOML registry helpers for Tempo local state.
///
/// We keep the read/parse and atomic write logic here so `keys.toml`,
/// `sessions.toml`, and any future Tempo registry files all use the same
/// persistence semantics instead of duplicating the same boilerplate.
pub(crate) fn read_toml_file<T: DeserializeOwned>(path: &Path, label: &str) -> Option<T> {
    if !path.exists() {
        tracing::trace!(?path, "{label} file not found");
        return None;
    }

    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) => {
            tracing::warn!(?path, %e, "failed to read {label} file");
            return None;
        }
    };

    match toml::from_str(&contents) {
        Ok(value) => Some(value),
        Err(e) => {
            tracing::warn!(?path, %e, "failed to parse {label} file");
            None
        }
    }
}

/// Write a Tempo registry file atomically via temp file + rename.
///
/// This keeps every registry on the same durability path and avoids repeating
/// the same create-dir / serialize / flush / persist sequence in each caller.
pub(crate) fn write_toml_file_atomic<T: Serialize>(
    path: &Path,
    value: &T,
    header: &str,
) -> Result<()> {
    let dir =
        path.parent().ok_or_else(|| eyre::eyre!("invalid registry path: {}", path.display()))?;
    fs::create_dir_all(dir)?;

    let body = toml::to_string_pretty(value)?;
    let contents = if header.trim().is_empty() { body } else { format!("{header}\n\n{body}") };

    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(contents.as_bytes())?;
    tmp.flush()?;
    tmp.persist(path).map_err(|e| eyre::eyre!("failed to persist {}: {e}", path.display()))?;

    Ok(())
}
