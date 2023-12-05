use clap::Parser;
use foundry_common::traits::{TestFilter, FunctionFilter, TestFunctionExt};
use foundry_cli::utils::FoundryPathExt;
use foundry_common::glob::GlobMatcher;
use foundry_config::Config;
use foundry_compilers::{FileFilter, ProjectPathsConfig};
use std::{fmt, path::Path};


/// The filter to use during mutation testing.
///
/// See also `FileFilter`.
#[derive(Clone, Parser)]
#[clap(next_help_heading = "Mutation Test filtering")]
pub struct MutationTestFilterArgs {
    /// Only run test functions matching the specified regex pattern.
    #[clap(long = "match-test", visible_alias = "mt", value_name = "REGEX")]
    pub test_pattern: Option<regex::Regex>,

    /// Only run test functions that do not match the specified regex pattern.
    #[clap(long = "no-match-test", visible_alias = "nmt", value_name = "REGEX")]
    pub test_pattern_inverse: Option<regex::Regex>,

    /// Only run mutations on functions matching the specified regex pattern.
    #[clap(long = "match-function", visible_alias = "mf", value_name = "REGEX")]
    pub function_pattern: Option<regex::Regex>,

    /// Only run mutations on functions that do not match the specified regex pattern.
    #[clap(
        long = "no-match-function",
        visible_alias = "nmf",
        value_name = "REGEX"
    )]
    pub function_pattern_inverse: Option<regex::Regex>,

    /// Only run mutations on functions in contracts matching the specified regex pattern.
    #[clap(long = "match-contract", visible_alias = "mc", value_name = "REGEX")]
    pub contract_pattern: Option<regex::Regex>,

    /// Only run mutations in contracts that do not match the specified regex pattern.
    #[clap(
        long = "no-match-contract",
        visible_alias = "nmc",
        value_name = "REGEX"
    )]
    pub contract_pattern_inverse: Option<regex::Regex>,

    /// Only run mutations on source files matching the specified glob pattern.
    #[clap(long = "match-path", visible_alias = "mp", value_name = "GLOB")]
    pub path_pattern: Option<GlobMatcher>,

    /// Only run mutations on source files that do not match the specified glob pattern.
    #[clap(
        name = "no-match-path",
        long = "no-match-path",
        visible_alias = "nmp",
        value_name = "GLOB"
    )]
    pub path_pattern_inverse: Option<GlobMatcher>,

    /// Only test mutants using this approach
    /// This is a generalized version of test_pattern and test_pattern_inverse
    #[clap(value_enum, default_value = "file")]
    pub test_mode: TestMode,
}

impl MutationTestFilterArgs {
    /// Merges the set filter globs with the config's values
    pub fn merge_with_config(
        &self,
        config: &Config,
    ) -> MutationProjectPathsAwareFilter {
        let mut filter = self.clone();
        if filter.test_pattern.is_none() {
            filter.test_pattern = config.mutation.test_pattern.clone().map(|p| p.into());
        }
        if filter.test_pattern_inverse.is_none() {
            filter.test_pattern_inverse = config.mutation.test_pattern_inverse.clone().map(|p| p.into());
        }
        if filter.function_pattern.is_none() {
            filter.function_pattern = config
                .mutation
                .function_pattern
                .clone()
                .map(|p| p.into());
        }
        if filter.function_pattern_inverse.is_none() {
            filter.function_pattern_inverse = config
                .mutation
                .function_pattern_inverse
                .clone()
                .map(|p| p.into());
        }
        if filter.contract_pattern.is_none() {
            filter.contract_pattern = config
                .mutation
                .contract_pattern
                .clone()
                .map(|p| p.into());
        }
        if filter.contract_pattern_inverse.is_none() {
            filter.contract_pattern_inverse = config
                .mutation
                .contract_pattern_inverse
                .clone()
                .map(|p| p.into());
        }
        if filter.path_pattern.is_none() {
            filter.path_pattern = config.mutation.path_pattern.clone().map(Into::into);
        }
        if filter.path_pattern_inverse.is_none() {
            filter.path_pattern_inverse = config
                .mutation
                .path_pattern_inverse
                .clone()
                .map(Into::into);
        }
        MutationProjectPathsAwareFilter {
            args_filter: filter,
            paths: config.project_paths(),
        }
    }
}

impl fmt::Debug for MutationTestFilterArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MutationTestFilterArgs")
            .field(
                "match-test",
                &self.test_pattern.as_ref().map(|r| r.as_str()),
            )
            .field(
                "no-match-test",
                &self.test_pattern_inverse.as_ref().map(|r| r.as_str()),
            )
            .field(
                "match-function",
                &self.function_pattern.as_ref().map(|r| r.as_str()),
            )
            .field(
                "no-match-function",
                &self.function_pattern_inverse.as_ref().map(|r| r.as_str()),
            )
            .field(
                "match-contract",
                &self.contract_pattern.as_ref().map(|r| r.as_str()),
            )
            .field(
                "no-match-contract",
                &self.contract_pattern_inverse.as_ref().map(|r| r.as_str()),
            )
            .field(
                "match-path",
                &self.path_pattern.as_ref().map(|g| g.as_str()),
            )
            .field(
                "no-match-path",
                &self.path_pattern_inverse.as_ref().map(|g| g.as_str()),
            )
            .finish_non_exhaustive()
    }
}

impl FileFilter for MutationTestFilterArgs {
    /// Returns true if the file regex pattern match the `file`
    ///
    /// If no file regex is set this returns true if the file ends with `.t.sol`, see
    /// [FoundryPathExr::is_sol_test()]
    fn is_match(&self, file: &Path) -> bool {
        if let Some(file) = file.as_os_str().to_str() {
            if let Some(ref glob) = self.path_pattern {
                return glob.is_match(file);
            }
            if let Some(ref glob) = self.path_pattern_inverse {
                return !glob.is_match(file);
            }
        }
        file.is_sol_test()
    }
}

impl TestFilter for MutationTestFilterArgs {
    fn matches_test(&self, test_name: impl AsRef<str>) -> bool {
        let mut ok = true;
        let test_name = test_name.as_ref();
        if let Some(re) = &self.function_pattern {
            ok &= re.is_match(test_name);
        }
        if let Some(re) = &self.function_pattern_inverse {
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

impl FunctionFilter for MutationTestFilterArgs {
    fn matches_function(&self, function_name: impl AsRef<str>) -> bool {
        let mut ok = true;
        let function_name = function_name.as_ref();

        if let Some(re) = &self.function_pattern {
            ok &= re.is_match(function_name);
        }

        if let Some(re) = &self.function_pattern_inverse {
            ok &= !re.is_match(function_name);
        }

        ok &= !function_name.is_test();

        ok
    }
}

impl fmt::Display for MutationTestFilterArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut patterns = Vec::new();
        if let Some(ref p) = self.function_pattern {
            patterns.push(format!("\tmatch-function: `{}`", p.as_str()));
        }
        if let Some(ref p) = self.function_pattern_inverse {
            patterns.push(format!("\tno-match-function: `{}`", p.as_str()));
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
pub struct MutationProjectPathsAwareFilter {
    args_filter: MutationTestFilterArgs,
    paths: ProjectPathsConfig,
}

impl MutationProjectPathsAwareFilter {
    /// Returns the CLI arguments
    pub fn args(&self) -> &MutationTestFilterArgs {
        &self.args_filter
    }

    /// Returns the CLI arguments mutably
    pub fn args_mut(&mut self) -> &mut MutationTestFilterArgs {
        &mut self.args_filter
    }
}

impl FileFilter for MutationProjectPathsAwareFilter {
    /// Returns true if the file regex pattern match the `file`
    ///
    /// If no file regex is set this returns true if the file ends with `.t.sol`, see
    /// [FoundryPathExr::is_sol_test()]
    fn is_match(&self, file: &Path) -> bool {
        self.args_filter.is_match(file)
    }
}

impl TestFilter for MutationProjectPathsAwareFilter {
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

impl FunctionFilter for MutationProjectPathsAwareFilter {
    fn matches_function(&self, function_name:impl AsRef<str>) -> bool {
        self.args_filter.matches_function(function_name)
    }
}

impl fmt::Display for MutationProjectPathsAwareFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.args_filter.fmt(f)
    }
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum TestMode {
    /// Only run tests matching the contract file name
    /// e.g. for Counter.sol mutations run tests in Counter.t.sol 
    File,
    
    /// Only run tests matching similar function names
    /// e.g. for Counter.sol:addNumber mutation run tests in test suite
    /// matching a regex test_addNumber, testAddNumber, test_AddNumber, 
    /// testFuzz_[a|A]ddNumber, testFail_addNumber
    Function,
    
    /// Run the entire test suite. This should be used in a CI 
    /// environment as it would take quite sometime 
    Full
}