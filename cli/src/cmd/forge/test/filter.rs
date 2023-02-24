use crate::utils::FoundryPathExt;
use clap::Parser;
use ethers::solc::{FileFilter, ProjectPathsConfig};
use forge::TestFilter;
use foundry_config::Config;
use std::{fmt, path::Path, str::FromStr};

/// The filter to use during testing.
///
/// See also `FileFilter`.
#[derive(Clone, Parser)]
#[clap(next_help_heading = "Test filtering")]
pub struct FilterArgs {
    /// Only run test functions matching the specified regex pattern.
    ///
    /// Deprecated: See --match-test
    #[clap(long = "match", short = 'm')]
    pub pattern: Option<regex::Regex>,

    /// Only run test functions matching the specified regex pattern.
    #[clap(
        long = "match-test",
        visible_alias = "mt",
        conflicts_with = "pattern",
        value_name = "REGEX"
    )]
    pub test_pattern: Option<regex::Regex>,

    /// Only run test functions that do not match the specified regex pattern.
    #[clap(
        long = "no-match-test",
        visible_alias = "nmt",
        conflicts_with = "pattern",
        value_name = "REGEX"
    )]
    pub test_pattern_inverse: Option<regex::Regex>,

    /// Only run tests in contracts matching the specified regex pattern.
    #[clap(
        long = "match-contract",
        visible_alias = "mc",
        conflicts_with = "pattern",
        value_name = "REGEX"
    )]
    pub contract_pattern: Option<regex::Regex>,

    /// Only run tests in contracts that do not match the specified regex pattern.
    #[clap(
        long = "no-match-contract",
        visible_alias = "nmc",
        conflicts_with = "pattern",
        value_name = "REGEX"
    )]
    pub contract_pattern_inverse: Option<regex::Regex>,

    /// Only run tests in source files matching the specified glob pattern.
    #[clap(
        long = "match-path",
        visible_alias = "mp",
        conflicts_with = "pattern",
        value_name = "GLOB"
    )]
    pub path_pattern: Option<GlobMatcher>,

    /// Only run tests in source files that do not match the specified glob pattern.
    #[clap(
        name = "no-match-path",
        long = "no-match-path",
        visible_alias = "nmp",
        conflicts_with = "pattern",
        value_name = "GLOB"
    )]
    pub path_pattern_inverse: Option<GlobMatcher>,
}

impl FilterArgs {
    /// Merges the set filter globs with the config's values
    pub fn merge_with_config(&self, config: &Config) -> ProjectPathsAwareFilter {
        let mut filter = self.clone();
        if filter.test_pattern.is_none() {
            filter.test_pattern = config.test_pattern.clone().map(|p| p.into());
        }
        if filter.test_pattern_inverse.is_none() {
            filter.test_pattern_inverse = config.test_pattern_inverse.clone().map(|p| p.into());
        }
        if filter.contract_pattern.is_none() {
            filter.contract_pattern = config.contract_pattern.clone().map(|p| p.into());
        }
        if filter.contract_pattern_inverse.is_none() {
            filter.contract_pattern_inverse =
                config.contract_pattern_inverse.clone().map(|p| p.into());
        }
        if filter.path_pattern.is_none() {
            filter.path_pattern = config.path_pattern.clone().map(Into::into);
        }
        if filter.path_pattern_inverse.is_none() {
            filter.path_pattern_inverse = config.path_pattern_inverse.clone().map(Into::into);
        }
        ProjectPathsAwareFilter { args_filter: filter, paths: config.project_paths() }
    }
}

impl fmt::Debug for FilterArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilterArgs")
            .field("match", &self.pattern.as_ref().map(|r| r.as_str()))
            .field("match-test", &self.test_pattern.as_ref().map(|r| r.as_str()))
            .field("no-match-test", &self.test_pattern_inverse.as_ref().map(|r| r.as_str()))
            .field("match-contract", &self.contract_pattern.as_ref().map(|r| r.as_str()))
            .field("no-match-contract", &self.contract_pattern_inverse.as_ref().map(|r| r.as_str()))
            .field("match-path", &self.path_pattern.as_ref().map(|g| g.as_str()))
            .field("no-match-path", &self.path_pattern_inverse.as_ref().map(|g| g.as_str()))
            .finish_non_exhaustive()
    }
}

impl FileFilter for FilterArgs {
    /// Returns true if the file regex pattern match the `file`
    ///
    /// If no file regex is set this returns true if the file ends with `.t.sol`, see
    /// [FoundryPathExr::is_sol_test()]
    fn is_match(&self, file: &Path) -> bool {
        if let Some(file) = file.as_os_str().to_str() {
            if let Some(ref glob) = self.path_pattern {
                return glob.is_match(file)
            }
            if let Some(ref glob) = self.path_pattern_inverse {
                return !glob.is_match(file)
            }
        }
        file.is_sol_test()
    }
}

impl TestFilter for FilterArgs {
    fn matches_test(&self, test_name: impl AsRef<str>) -> bool {
        let mut ok = true;
        let test_name = test_name.as_ref();
        // Handle the deprecated option match
        if let Some(re) = &self.pattern {
            ok &= re.is_match(test_name);
        }
        if let Some(re) = &self.test_pattern {
            ok &= re.is_match(test_name);
        }
        if let Some(re) = &self.test_pattern_inverse {
            ok &= !re.is_match(test_name);
        }
        ok
    }

    fn matches_contract(&self, contract_name: impl AsRef<str>) -> bool {
        let mut ok = true;
        let contract_name = contract_name.as_ref();
        if let Some(re) = &self.contract_pattern {
            ok &= re.is_match(contract_name);
        }
        if let Some(re) = &self.contract_pattern_inverse {
            ok &= !re.is_match(contract_name);
        }
        ok
    }

    fn matches_path(&self, path: impl AsRef<str>) -> bool {
        let mut ok = true;
        let path = path.as_ref();
        if let Some(ref glob) = self.path_pattern {
            ok &= glob.is_match(path);
        }
        if let Some(ref glob) = self.path_pattern_inverse {
            ok &= !glob.is_match(path);
        }
        ok
    }
}

impl fmt::Display for FilterArgs {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut patterns = Vec::new();
        if let Some(ref p) = self.pattern {
            patterns.push(format!("\tmatch: `{}`", p.as_str()));
        }
        if let Some(ref p) = self.test_pattern {
            patterns.push(format!("\tmatch-test: `{}`", p.as_str()));
        }
        if let Some(ref p) = self.test_pattern_inverse {
            patterns.push(format!("\tno-match-test: `{}`", p.as_str()));
        }
        if let Some(ref p) = self.contract_pattern {
            patterns.push(format!("\tmatch-contract: `{}`", p.as_str()));
        }
        if let Some(ref p) = self.contract_pattern_inverse {
            patterns.push(format!("\tno-match-contract: `{}`", p.as_str()));
        }
        if let Some(ref p) = self.path_pattern {
            patterns.push(format!("\tmatch-path: `{}`", p.as_str()));
        }
        if let Some(ref p) = self.path_pattern_inverse {
            patterns.push(format!("\tno-match-path: `{}`", p.as_str()));
        }
        write!(f, "{}", patterns.join("\n"))
    }
}

/// A filter that combines all command line arguments and the paths of the current projects
#[derive(Debug, Clone)]
pub struct ProjectPathsAwareFilter {
    args_filter: FilterArgs,
    paths: ProjectPathsConfig,
}

// === impl ProjectPathsAwareFilter ===

impl ProjectPathsAwareFilter {
    /// Returns the CLI arguments
    pub fn args(&self) -> &FilterArgs {
        &self.args_filter
    }

    /// Returns the CLI arguments mutably
    pub fn args_mut(&mut self) -> &mut FilterArgs {
        &mut self.args_filter
    }
}

impl FileFilter for ProjectPathsAwareFilter {
    /// Returns true if the file regex pattern match the `file`
    ///
    /// If no file regex is set this returns true if the file ends with `.t.sol`, see
    /// [FoundryPathExr::is_sol_test()]
    fn is_match(&self, file: &Path) -> bool {
        self.args_filter.is_match(file)
    }
}

impl TestFilter for ProjectPathsAwareFilter {
    fn matches_test(&self, test_name: impl AsRef<str>) -> bool {
        self.args_filter.matches_test(test_name)
    }

    fn matches_contract(&self, contract_name: impl AsRef<str>) -> bool {
        self.args_filter.matches_contract(contract_name)
    }

    fn matches_path(&self, path: impl AsRef<str>) -> bool {
        let path = path.as_ref();
        // we don't want to test files that belong to a library
        self.args_filter.matches_path(path) && !self.paths.has_library_ancestor(Path::new(path))
    }
}

impl fmt::Display for ProjectPathsAwareFilter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.args_filter.fmt(f)
    }
}

/// A `globset::Glob` that creates its `globset::GlobMatcher` when its created, so it doesn't need
/// to be compiled when the filter functions `TestFilter` functions are called.
#[derive(Debug, Clone)]
pub struct GlobMatcher {
    /// The parsed glob
    pub glob: globset::Glob,
    /// The compiled `glob`
    pub matcher: globset::GlobMatcher,
}

// === impl GlobMatcher ===

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
