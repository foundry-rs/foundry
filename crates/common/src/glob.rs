//! Contains `globset::Glob` wrapper functions used for filtering.

use std::{
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

/// Expand globs with a root path.
pub fn expand_globs(
    root: &Path,
    patterns: impl IntoIterator<Item = impl AsRef<str>>,
) -> eyre::Result<Vec<PathBuf>> {
    let mut expanded = Vec::new();
    for pattern in patterns {
        for paths in glob::glob(&root.join(pattern.as_ref()).display().to_string())? {
            expanded.push(paths?);
        }
    }
    Ok(expanded)
}

/// A `globset::Glob` that creates its `globset::GlobMatcher` when its created, so it doesn't need
/// to be compiled when the filter functions `TestFilter` functions are called.
#[derive(Debug, Clone)]
pub struct GlobMatcher {
    /// The parsed glob
    pub glob: globset::Glob,
    /// The compiled glob
    pub matcher: globset::GlobMatcher,
}

impl GlobMatcher {
    /// Tests whether the given path matches this pattern or not.
    ///
    /// The glob `./test/*` won't match absolute paths like `test/Contract.sol`, which is common
    /// format here, so we also handle this case here
    pub fn is_match(&self, path: &str) -> bool {
        let mut matches = self.matcher.is_match(path);
        if !matches && !path.starts_with("./") && self.as_str().starts_with("./") {
            matches = self.matcher.is_match(format!("./{path}"));
        }
        matches
    }

    /// Returns the `Glob` string used to compile this matcher.
    pub fn as_str(&self) -> &str {
        self.glob.glob()
    }
}

impl fmt::Display for GlobMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.glob.fmt(f)
    }
}

impl FromStr for GlobMatcher {
    type Err = globset::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<globset::Glob>().map(Into::into)
    }
}

impl From<globset::Glob> for GlobMatcher {
    fn from(glob: globset::Glob) -> Self {
        let matcher = glob.compile_matcher();
        Self { glob, matcher }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_match_glob_paths() {
        let matcher: GlobMatcher = "./test/*".parse().unwrap();
        assert!(matcher.is_match("test/Contract.sol"));
        assert!(matcher.is_match("./test/Contract.sol"));
    }
}
