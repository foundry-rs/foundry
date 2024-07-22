//! Helpers for constructing and using [FileFilter]s.

use core::fmt;
use foundry_compilers::FileFilter;
use serde::{Deserialize, Serialize};
use std::{
    convert::Infallible,
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
#[derive(Clone, Debug)]
pub struct GlobMatcher {
    /// The compiled glob
    pub matcher: globset::GlobMatcher,
}

impl GlobMatcher {
    /// Creates a new `GlobMatcher` from a `globset::Glob`.
    pub fn new(glob: globset::Glob) -> Self {
        Self { matcher: glob.compile_matcher() }
    }

    /// Tests whether the given path matches this pattern or not.
    ///
    /// The glob `./test/*` won't match absolute paths like `test/Contract.sol`, which is common
    /// format here, so we also handle this case here
    pub fn is_match(&self, path: &Path) -> bool {
        if self.matcher.is_match(path) {
            return true;
        }

        if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            if file_name.contains(self.as_str()) {
                return true;
            }
        }

        if !path.starts_with("./") && self.as_str().starts_with("./") {
            return self.matcher.is_match(format!("./{}", path.display()));
        }

        if path.is_relative() && Path::new(self.glob().glob()).is_absolute() {
            if let Ok(canonicalized_path) = dunce::canonicalize(path) {
                return self.matcher.is_match(&canonicalized_path);
            } else {
                return false;
            }
        }

        false
    }

    /// Matches file only if the filter does not apply.
    ///
    /// This returns the inverse of `self.is_match(file)`.
    fn is_match_exclude(&self, path: &Path) -> bool {
        !self.is_match(path)
    }

    /// Returns the `globset::Glob`.
    pub fn glob(&self) -> &globset::Glob {
        self.matcher.glob()
    }

    /// Returns the `Glob` string used to compile this matcher.
    pub fn as_str(&self) -> &str {
        self.glob().glob()
    }
}

impl fmt::Display for GlobMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.glob().fmt(f)
    }
}

impl FromStr for GlobMatcher {
    type Err = globset::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<globset::Glob>().map(Self::new)
    }
}

impl From<globset::Glob> for GlobMatcher {
    fn from(glob: globset::Glob) -> Self {
        Self::new(glob)
    }
}

impl Serialize for GlobMatcher {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.glob().glob().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for GlobMatcher {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl PartialEq for GlobMatcher {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for GlobMatcher {}

/// Bundles multiple `SkipBuildFilter` into a single `FileFilter`
#[derive(Clone, Debug)]
pub struct SkipBuildFilters {
    /// All provided filters.
    pub matchers: Vec<GlobMatcher>,
    /// Root of the project.
    pub project_root: PathBuf,
}

impl FileFilter for SkipBuildFilters {
    /// Only returns a match if _no_  exclusion filter matches
    fn is_match(&self, file: &Path) -> bool {
        self.matchers.iter().all(|matcher| {
            if !matcher.is_match_exclude(file) {
                false
            } else {
                file.strip_prefix(&self.project_root)
                    .map_or(true, |stripped| matcher.is_match_exclude(stripped))
            }
        })
    }
}

impl SkipBuildFilters {
    /// Creates a new `SkipBuildFilters` from multiple `SkipBuildFilter`.
    pub fn new<G: Into<GlobMatcher>>(
        filters: impl IntoIterator<Item = G>,
        project_root: PathBuf,
    ) -> Self {
        let matchers = filters.into_iter().map(|m| m.into()).collect();
        Self { matchers, project_root }
    }
}

/// A filter that excludes matching contracts from the build
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipBuildFilter {
    /// Exclude all `.t.sol` contracts
    Tests,
    /// Exclude all `.s.sol` contracts
    Scripts,
    /// Exclude if the file matches
    Custom(String),
}

impl SkipBuildFilter {
    fn new(s: &str) -> Self {
        match s {
            "test" | "tests" => Self::Tests,
            "script" | "scripts" => Self::Scripts,
            s => Self::Custom(s.to_string()),
        }
    }

    /// Returns the pattern to match against a file
    pub fn file_pattern(&self) -> &str {
        match self {
            Self::Tests => ".t.sol",
            Self::Scripts => ".s.sol",
            Self::Custom(s) => s.as_str(),
        }
    }
}

impl FromStr for SkipBuildFilter {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_filter() {
        let tests = GlobMatcher::from_str(SkipBuildFilter::Tests.file_pattern()).unwrap();
        let scripts = GlobMatcher::from_str(SkipBuildFilter::Scripts.file_pattern()).unwrap();
        let custom = |s| GlobMatcher::from_str(s).unwrap();

        let file = Path::new("A.t.sol");
        assert!(!tests.is_match_exclude(file));
        assert!(scripts.is_match_exclude(file));
        assert!(!custom("A.t").is_match_exclude(file));

        let file = Path::new("A.s.sol");
        assert!(tests.is_match_exclude(file));
        assert!(!scripts.is_match_exclude(file));
        assert!(!custom("A.s").is_match_exclude(file));

        let file = Path::new("/home/test/Foo.sol");
        assert!(!custom("*/test/**").is_match_exclude(file));

        let file = Path::new("/home/script/Contract.sol");
        assert!(!custom("*/script/**").is_match_exclude(file));
    }

    #[test]
    fn can_match_relative_glob_paths() {
        let matcher: GlobMatcher = "./test/*".parse().unwrap();

        // Absolute path that should match the pattern
        assert!(matcher.is_match(Path::new("test/Contract.t.sol")));

        // Relative path that should match the pattern
        assert!(matcher.is_match(Path::new("./test/Contract.t.sol")));
    }

    #[test]
    fn can_match_absolute_glob_paths() {
        let matcher: GlobMatcher = "/home/user/projects/project/test/*".parse().unwrap();

        // Absolute path that should match the pattern
        assert!(matcher.is_match(Path::new("/home/user/projects/project/test/Contract.t.sol")));

        // Absolute path that should not match the pattern
        assert!(!matcher.is_match(Path::new("/home/user/other/project/test/Contract.t.sol")));

        // Relative path that should not match an absolute pattern
        assert!(!matcher.is_match(Path::new("projects/project/test/Contract.t.sol")));
    }
}
