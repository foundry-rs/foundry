//! Filter for excluding contracts in `forge build`

use ethers::solc::FileFilter;
use std::{convert::Infallible, path::Path, str::FromStr};

/// Bundles multiple `SkipBuildFilter` into a single `FileFilter`
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SkipBuildFilters(pub Vec<SkipBuildFilter>);

impl FileFilter for SkipBuildFilters {
    /// Only returns a match if no filter a
    fn is_match(&self, file: &Path) -> bool {
        self.0.iter().all(|filter| filter.is_match(file))
    }
}

/// A filter that excludes matching contracts from the build
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SkipBuildFilter {
    /// Exclude all `.t.sol` contracts
    Tests,
    /// Exclude all `.s.sol` contracts
    Scripts,
    /// Exclude if the file matches
    Custom(String),
}

impl SkipBuildFilter {
    /// Returns the pattern to match against a file
    fn file_pattern(&self) -> &str {
        match self {
            SkipBuildFilter::Tests => ".t.sol",
            SkipBuildFilter::Scripts => ".s.sol",
            SkipBuildFilter::Custom(s) => s.as_str(),
        }
    }
}

impl<T: AsRef<str>> From<T> for SkipBuildFilter {
    fn from(s: T) -> Self {
        match s.as_ref() {
            "tests" => SkipBuildFilter::Tests,
            "scripts" => SkipBuildFilter::Scripts,
            s => SkipBuildFilter::Custom(s.to_string()),
        }
    }
}

impl FromStr for SkipBuildFilter {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(s.into())
    }
}

impl FileFilter for SkipBuildFilter {
    /// Matches file only if the filter does not apply
    ///
    /// This is returns the inverse of `file.name.contains(pattern)`
    fn is_match(&self, file: &Path) -> bool {
        fn exclude(file: &Path, pattern: &str) -> Option<bool> {
            let file_name = file.file_name()?.to_str()?;
            Some(file_name.contains(pattern))
        }

        !exclude(file, self.file_pattern()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_filter() {
        let file = Path::new("A.t.sol");
        assert!(!SkipBuildFilter::Tests.is_match(file));
        assert!(SkipBuildFilter::Scripts.is_match(file));
        assert!(!SkipBuildFilter::Custom("A.t".to_string()).is_match(file));

        let file = Path::new("A.s.sol");
        assert!(SkipBuildFilter::Tests.is_match(file));
        assert!(!SkipBuildFilter::Scripts.is_match(file));
        assert!(!SkipBuildFilter::Custom("A.s".to_string()).is_match(file));
    }
}
