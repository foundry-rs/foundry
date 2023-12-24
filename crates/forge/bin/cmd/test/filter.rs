use clap::Parser;
use forge::TestFilter;
use foundry_cli::utils::FoundryPathExt;
use foundry_common::glob::GlobMatcher;
use foundry_compilers::{FileFilter, ProjectPathsConfig};
use foundry_config::Config;
use std::{fmt, path::Path};

/// The filter to use during testing.
///
/// See also `FileFilter`.
#[derive(Clone, Parser, Default)]
#[clap(next_help_heading = "Test filtering")]
pub struct FilterArgs {
    /// Only run test functions matching the specified regex pattern.
    #[clap(long = "match-test", visible_alias = "mt", value_name = "REGEX")]
    pub test_pattern: Option<regex::Regex>,

    /// Only run test functions that do not match the specified regex pattern.
    #[clap(long = "no-match-test", visible_alias = "nmt", value_name = "REGEX")]
    pub test_pattern_inverse: Option<regex::Regex>,

    /// Only run tests in contracts matching the specified regex pattern.
    #[clap(long = "match-contract", visible_alias = "mc", value_name = "REGEX")]
    pub contract_pattern: Option<regex::Regex>,

    /// Only run tests in contracts that do not match the specified regex pattern.
    #[clap(long = "no-match-contract", visible_alias = "nmc", value_name = "REGEX")]
    pub contract_pattern_inverse: Option<regex::Regex>,

    /// Only run tests in source files matching the specified glob pattern.
    #[clap(long = "match-path", visible_alias = "mp", value_name = "GLOB")]
    pub path_pattern: Option<GlobMatcher>,

    /// Only run tests in source files that do not match the specified glob pattern.
    #[clap(
        name = "no-match-path",
        long = "no-match-path",
        visible_alias = "nmp",
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

    /// Merges the set filter globs with mutate test config values
    pub fn merge_with_mutate_config(&self, config: &Config) -> ProjectPathsAwareFilter {
        let mut filter = self.clone();

        if filter.test_pattern.is_none() {
            filter.test_pattern = config.mutate.test_pattern.clone().map(|p| p.into());
        }

        if filter.test_pattern_inverse.is_none() {
            filter.test_pattern_inverse = config.mutate.test_contract_pattern_inverse.clone().map(|p| p.into());
        }

        if filter.contract_pattern.is_none() {
            filter.contract_pattern = config.mutate.test_contract_pattern.clone().map(|p| p.into());
        }

        if filter.contract_pattern_inverse.is_none() {
            filter.contract_pattern = config.mutate.test_contract_pattern_inverse.clone().map(|p| p.into());
        }

        if filter.path_pattern.is_none() {
            filter.path_pattern = config.mutate.test_path_pattern.clone().map(|p| p.into());
        }

        if filter.path_pattern_inverse.is_none() {
            filter.path_pattern = config.mutate.test_path_pattern_inverse.clone().map(|p| p.into());
        }
        
        ProjectPathsAwareFilter { args_filter: self.clone(), paths: config.project_paths() }
    }
}

impl fmt::Debug for FilterArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilterArgs")
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
    fn matches_test(&self, test_name: &str) -> bool {
        let mut ok = true;
        if let Some(re) = &self.test_pattern {
            ok = ok && re.is_match(test_name);
        }
        if let Some(re) = &self.test_pattern_inverse {
            ok = ok && !re.is_match(test_name);
        }
        ok
    }

    fn matches_contract(&self, contract_name: &str) -> bool {
        let mut ok = true;
        if let Some(re) = &self.contract_pattern {
            ok = ok && re.is_match(contract_name);
        }
        if let Some(re) = &self.contract_pattern_inverse {
            ok = ok && !re.is_match(contract_name);
        }
        ok
    }

    fn matches_path(&self, path: &Path) -> bool {
        let Some(path) = path.to_str() else {
            return false;
        };

        let mut ok = true;
        if let Some(re) = &self.path_pattern {
            ok = ok && re.is_match(path);
        }
        if let Some(re) = &self.path_pattern_inverse {
            ok = ok && !re.is_match(path);
        }
        ok
    }
}

impl fmt::Display for FilterArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut patterns = Vec::new();
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
    fn matches_test(&self, test_name: &str) -> bool {
        self.args_filter.matches_test(test_name)
    }

    fn matches_contract(&self, contract_name: &str) -> bool {
        self.args_filter.matches_contract(contract_name)
    }

    fn matches_path(&self, path: &Path) -> bool {
        // we don't want to test files that belong to a library
        self.args_filter.matches_path(path) && !self.paths.has_library_ancestor(path)
    }
}

impl fmt::Display for ProjectPathsAwareFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.args_filter.fmt(f)
    }
}
