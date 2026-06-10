use eyre::{Result, WrapErr};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs,
    io::{ErrorKind, Write},
    path::Path,
};

/// Shared TOML registry helpers for Tempo local state.
///
/// We keep the read/parse and atomic write logic here so `keys.toml`,
/// `sessions.toml`, and any future Tempo registry files all use the same
/// persistence semantics instead of duplicating the same boilerplate.
///
/// Strict readers return `Ok(None)` only when the file is missing.
/// Corruption and I/O failures bubble up so mutating paths can fail closed.
pub(crate) fn read_toml_file<T: DeserializeOwned>(path: &Path, label: &str) -> Result<Option<T>> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            tracing::trace!(?path, "{label} file not found");
            return Ok(None);
        }
        Err(e) => {
            return Err(e)
                .wrap_err_with(|| format!("failed to read {label} file {}", path.display()));
        }
    };

    let value = toml::from_str(&contents)
        .wrap_err_with(|| format!("failed to parse {label} file {}", path.display()))?;
    Ok(Some(value))
}

/// Write a Tempo registry file atomically via temp file + rename.
///
/// This keeps every registry on the same durability path and avoids repeating
/// the same create-dir / serialize / flush / persist sequence in each caller.
/// The temp file and parent directory are synced so rename is much closer to a
/// crash-safe durable write.
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
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| eyre::eyre!("failed to persist {}: {e}", path.display()))?;
    sync_parent_dir(dir)?;

    Ok(())
}

#[cfg(unix)]
fn sync_parent_dir(dir: &Path) -> Result<()> {
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_dir(_dir: &Path) -> Result<()> {
    Ok(())
}
