use std::path::{Path, PathBuf};

/// Expand globs with a root path.
pub fn expand_globs(
    root: &Path,
    mut patterns: impl Iterator<Item = impl AsRef<str>>,
) -> eyre::Result<Vec<PathBuf>> {
    patterns.try_fold(Vec::default(), |mut expanded, pattern| {
        let paths = glob::glob(&root.join(pattern.as_ref()).display().to_string())?;
        expanded.extend(paths.into_iter().collect::<Result<Vec<_>, _>>()?);
        Ok(expanded)
    })
}
