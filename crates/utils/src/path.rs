//! General Foundry path utils

use std::path::PathBuf;

/// Canonicalize a path, returning an error if the path does not exist.
/// Mainly useful to apply canonicalization to paths obtained from project files
/// but still error properly instead of flattening the errors.
pub fn canonicalize_path(path: &PathBuf) -> eyre::Result<PathBuf> {
    Ok(dunce::canonicalize(path)?)
}
