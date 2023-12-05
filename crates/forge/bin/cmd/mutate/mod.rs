use clap::Parser;
use foundry_common::traits::{TestFilter, FunctionFilter, TestFunctionExt};
use foundry_cli::utils::FoundryPathExt;
use foundry_common::glob::GlobMatcher;
use foundry_config::Config;
use foundry_compilers::{FileFilter, ProjectPathsConfig};
use std::{fmt, path::Path};


mod filter;
pub use filter::*;

/// CLI arguments for `forge mutate`.
#[derive(Debug, Clone, Parser)]
#[clap(next_help_heading = "Mutation Test options")]
pub struct MutateTestArgs {
    /// Output Rutant test results in JSON format.
    #[clap(long, short, help_heading = "Display options")]
    json: bool,

    #[clap(flatten)]
    filter: MutationTestFilterArgs,

    /// Exit with code 0 even if a test fails.
    #[clap(long, env = "FORGE_ALLOW_FAILURE")]
    allow_failure: bool,

    /// Stop running mutation tests after the first failure
    #[clap(long)]
    pub fail_fast: bool,

    /// List mutation tests instead of running them
    #[clap(long, short, help_heading = "Display options")]
    list: bool,

    #[clap(flatten)]
    evm_opts: EvmArgs,

    #[clap(flatten)]
    opts: CoreBuildArgs,

    /// Export generated mutants to a directory
    #[clap(long, default_value_t = false)]
    pub export: bool,

    /// Print mutation test summary table
    #[clap(long, help_heading = "Display options", default_value_t = true)]
    pub summary: bool,

    /// Print detailed mutation test summary table
    #[clap(long, help_heading = "Display options")]
    pub detailed: bool,
}
