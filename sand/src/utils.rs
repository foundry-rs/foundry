//! utilities

use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Returns a list of absolute paths to all the cairo files under the `root`, or the file itself,
/// if the path is a cairo file.
///
/// # Example
///
/// ```no_run
/// use foundry_sand::utils;
/// let cairo_files = utils::cairo_files("./contracts");
/// ```
pub fn cairo_files(root: impl AsRef<Path>) -> Vec<PathBuf> {
    WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().map(|ext| ext == "cairo").unwrap_or_default())
        .map(|e| e.path().into())
        .collect()
}
