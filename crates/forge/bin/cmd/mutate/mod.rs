use super::install;
use clap::Parser;
use eyre::{eyre, Result, Error};
use forge::{
    inspectors::CheatsConfig,
    result::SuiteResult,
    MultiContractRunnerBuilder, TestOptions, TestOptionsBuilder,
};
use foundry_cli::{
    init_progress,
    update_progress,
    opts::CoreBuildArgs,
    utils::{self, LoadConfig},
};
use foundry_common::{
    compile::{self, ProjectCompiler},
    evm::EvmArgs,
    shell::{self}
};
use foundry_compilers::{project_util::{copy_dir, TempProject}, report};
use foundry_config::{
    figment,
    figment::{
        value::{Dict, Map},
        Metadata, Profile, Provider,
    },
    get_available_profiles, Config,
};
use foundry_evm::opts::EvmOpts;
use futures::future::try_join_all;
use itertools::Itertools;
use std::{collections::BTreeMap, fs, sync::mpsc::channel, time::{Duration, Instant}, path::{PathBuf, Path}};
use yansi::Paint;
use foundry_evm_mutator::{Mutant, Mutator, MutatorConfigBuilder};


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
