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