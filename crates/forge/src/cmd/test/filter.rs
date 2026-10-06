use alloy_json_abi::Function;
use clap::Parser;
use foundry_common::{TestFilter, TestFunctionKind};
use foundry_compilers::{FileFilter, ProjectPathsConfig};
use foundry_config::{Config, filter::GlobMatcher};
use serde::{Deserialize, Serialize};
use std::{fmt, path::Path};

/// A failed test persisted for `forge test --rerun`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RerunFailure {
    /// The test suite identifier (`path:contract_name`).
    pub contract: String,
    /// The test signature or invariant predicate name.
    pub test: String,
}

/// Persisted `forge test --rerun` failures.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RerunFailures {
    pub version: u8,
    pub failures: Vec<RerunFailure>,
}

/// The filter to use during testing.
///
/// See also `FileFilter`.
#[derive(Clone, Default, Parser)]
#[command(next_help_heading = "Test filtering")]
pub struct FilterArgs {
    /// Only run test functions matching the specified regex pattern.
    #[arg(long = "match-test", visible_alias = "mt", value_name = "REGEX")]
    pub test_pattern: Option<regex::Regex>,

    /// Only run test functions that do not match the specified regex pattern.
    #[arg(long = "no-match-test", visible_alias = "nmt", value_name = "REGEX")]
    pub test_pattern_inverse: Option<regex::Regex>,

    /// Only run tests in contracts matching the specified regex pattern.
    #[arg(long = "match-contract", visible_alias = "mc", value_name = "REGEX")]
    pub contract_pattern: Option<regex::Regex>,

    /// Only run tests in contracts that do not match the specified regex pattern.
    #[arg(long = "no-match-contract", visible_alias = "nmc", value_name = "REGEX")]
    pub contract_pattern_inverse: Option<regex::Regex>,

    /// Only run tests in source files matching the specified glob pattern.
    #[arg(long = "match-path", visible_alias = "mp", value_name = "GLOB")]
    pub path_pattern: Option<GlobMatcher>,

    /// Only run tests in source files that do not match the specified glob pattern.
    #[arg(
        id = "no-match-path",
        long = "no-match-path",
        visible_alias = "nmp",
        value_name = "GLOB"
    )]
    pub path_pattern_inverse: Option<GlobMatcher>,

    /// Only show coverage for files that do not match the specified regex pattern.
    #[arg(long = "no-match-coverage", visible_alias = "nmco", value_name = "REGEX")]
    pub coverage_pattern_inverse: Option<regex::Regex>,
}

impl FilterArgs {
    /// Returns true if the filter is empty.
    pub const fn is_empty(&self) -> bool {
        self.test_pattern.is_none()
            && self.test_pattern_inverse.is_none()
            && self.contract_pattern.is_none()
            && self.contract_pattern_inverse.is_none()
            && self.path_pattern.is_none()
            && self.path_pattern_inverse.is_none()
    }

    /// Merges the set filter globs with the config's values
    pub fn merge_with_config(mut self, config: &Config) -> ProjectPathsAwareFilter {
        self.test_pattern =
            self.test_pattern.or_else(|| config.test_pattern.clone().map(Into::into));
        self.test_pattern_inverse = self
            .test_pattern_inverse
            .or_else(|| config.test_pattern_inverse.clone().map(Into::into));
        self.contract_pattern =
            self.contract_pattern.or_else(|| config.contract_pattern.clone().map(Into::into));
        self.contract_pattern_inverse = self
            .contract_pattern_inverse
            .or_else(|| config.contract_pattern_inverse.clone().map(Into::into));
        self.path_pattern =
            self.path_pattern.or_else(|| config.path_pattern.clone().map(Into::into));
        self.path_pattern_inverse = self
            .path_pattern_inverse
            .or_else(|| config.path_pattern_inverse.clone().map(Into::into));
        self.coverage_pattern_inverse = self
            .coverage_pattern_inverse
            .or_else(|| config.coverage_pattern_inverse.clone().map(Into::into));
        ProjectPathsAwareFilter {
            args_filter: self,
            paths: config.project_paths(),
            rerun_failures: None,
        }
    }

    /// Returns all patterns as `(flag, pattern)` pairs.
    fn patterns(&self) -> [(&'static str, Option<&str>); 7] {
        [
            ("match-test", self.test_pattern.as_ref().map(|r| r.as_str())),
            ("no-match-test", self.test_pattern_inverse.as_ref().map(|r| r.as_str())),
            ("match-contract", self.contract_pattern.as_ref().map(|r| r.as_str())),
            ("no-match-contract", self.contract_pattern_inverse.as_ref().map(|r| r.as_str())),
            ("match-path", self.path_pattern.as_ref().map(|g| g.as_str())),
            ("no-match-path", self.path_pattern_inverse.as_ref().map(|g| g.as_str())),
            ("no-match-coverage", self.coverage_pattern_inverse.as_ref().map(|r| r.as_str())),
        ]
    }
}

impl fmt::Debug for FilterArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("FilterArgs");
        for (name, pattern) in self.patterns() {
            s.field(name, &pattern);
        }
        s.finish_non_exhaustive()
    }
}

impl FileFilter for FilterArgs {
    /// Returns true if the file regex pattern match the `file`
    ///
    /// If no file regex is set this returns true by default
    fn is_match(&self, file: &Path) -> bool {
        self.matches_path(file)
    }
}

impl TestFilter for FilterArgs {
    fn matches_test(&self, test_signature: &str) -> bool {
        self.test_pattern.as_ref().is_none_or(|re| re.is_match(test_signature))
            && self.test_pattern_inverse.as_ref().is_none_or(|re| !re.is_match(test_signature))
    }

    fn matches_contract(&self, contract_name: &str) -> bool {
        self.contract_pattern.as_ref().is_none_or(|re| re.is_match(contract_name))
            && self.contract_pattern_inverse.as_ref().is_none_or(|re| !re.is_match(contract_name))
    }

    fn matches_path(&self, path: &Path) -> bool {
        self.path_pattern.as_ref().is_none_or(|g| g.is_match(path))
            && self.path_pattern_inverse.as_ref().is_none_or(|g| !g.is_match(path))
    }
}

impl fmt::Display for FilterArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (name, pattern) in self.patterns() {
            if let Some(pattern) = pattern {
                writeln!(f, "\t{name}: `{pattern}`")?;
            }
        }
        Ok(())
    }
}

/// A filter that combines all command line arguments and the paths of the current projects
#[derive(Clone, Debug)]
pub struct ProjectPathsAwareFilter {
    args_filter: FilterArgs,
    paths: ProjectPathsConfig,
    rerun_failures: Option<Vec<RerunFailure>>,
}

impl ProjectPathsAwareFilter {
    /// Returns true if the filter is empty.
    pub const fn is_empty(&self) -> bool {
        self.args_filter.is_empty()
    }

    /// Returns the CLI arguments.
    pub const fn args(&self) -> &FilterArgs {
        &self.args_filter
    }

    /// Returns the CLI arguments mutably.
    pub const fn args_mut(&mut self) -> &mut FilterArgs {
        &mut self.args_filter
    }

    /// Returns the project paths.
    pub const fn paths(&self) -> &ProjectPathsConfig {
        &self.paths
    }

    /// Sets exact contract/test pairs persisted by `forge test --rerun`.
    pub fn set_rerun_failures(&mut self, failures: Vec<RerunFailure>) {
        self.rerun_failures = Some(failures);
    }

    /// Returns exact contract/test pairs persisted by `forge test --rerun`.
    pub fn rerun_failures(&self) -> Option<&[RerunFailure]> {
        self.rerun_failures.as_deref()
    }

    fn matches_rerun_contract(&self, failure_contract: &str, contract_id: &str) -> bool {
        if failure_contract == contract_id {
            return true;
        }
        let (Some((failure_path, failure_name)), Some((contract_path, contract_name))) =
            (failure_contract.rsplit_once(':'), contract_id.rsplit_once(':'))
        else {
            return false;
        };
        if failure_name != contract_name {
            return false;
        }

        let normalize = |path: &str| {
            let path = Path::new(path);
            if let Ok(path) = path.strip_prefix(&self.paths.root) {
                return path.to_path_buf();
            }
            if path.is_absolute()
                && let Ok(root) = dunce::canonicalize(&self.paths.root)
                && let Ok(path) = dunce::canonicalize(path)
                && let Ok(path) = path.strip_prefix(root)
            {
                return path.to_path_buf();
            }
            path.to_path_buf()
        };
        normalize(failure_path) == normalize(contract_path)
    }
}

impl FileFilter for ProjectPathsAwareFilter {
    /// Returns true if the file regex pattern match the `file`
    ///
    /// If no file regex is set this returns true by default
    fn is_match(&self, mut file: &Path) -> bool {
        file = file.strip_prefix(&self.paths.root).unwrap_or(file);
        self.args_filter.is_match(file)
    }
}

impl TestFilter for ProjectPathsAwareFilter {
    fn matches_test(&self, test_signature: &str) -> bool {
        self.args_filter.matches_test(test_signature)
    }

    fn matches_contract(&self, contract_name: &str) -> bool {
        self.args_filter.matches_contract(contract_name)
    }

    fn matches_path(&self, mut path: &Path) -> bool {
        // we don't want to test files that belong to a library
        path = path.strip_prefix(&self.paths.root).unwrap_or(path);
        self.args_filter.matches_path(path) && !self.paths.has_library_ancestor(path)
    }

    fn matches_test_function_kind_in_contract(
        &self,
        contract_id: &str,
        func: &Function,
        kind: TestFunctionKind,
    ) -> bool {
        let signature = func.signature();
        if !kind.is_any_test() || !self.args_filter.matches_test(&signature) {
            return false;
        }
        let Some(failures) = &self.rerun_failures else { return true };
        let name = signature.split('(').next().unwrap_or(&signature);
        failures.iter().any(|failure| {
            self.matches_rerun_contract(&failure.contract, contract_id)
                && (failure.test == signature || failure.test == name)
        })
    }
}

impl fmt::Display for ProjectPathsAwareFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.args_filter.fmt(f)
    }
}
