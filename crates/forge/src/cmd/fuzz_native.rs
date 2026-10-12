//! Native fuzz command dispatch.

use super::test::TestArgs;
use crate::result::TestOutcome;
use clap::{Parser, Subcommand};
use eyre::Result;

pub use super::fuzz_run::FuzzRunArgs;

/// Run Forge fuzz tests.
#[derive(Clone, Debug, Parser)]
pub struct FuzzArgs {
    #[command(subcommand)]
    pub command: FuzzSubcommands,
}

impl FuzzArgs {
    pub async fn run(self) -> Result<TestOutcome> {
        let FuzzSubcommands::Run(args) = self.command;
        let mut test = TestArgs::from_fuzz_run(args);
        test.enable_fuzz_only_with_auto_fuzz_corpus();
        test.run().await
    }

    pub const fn is_junit(&self) -> bool {
        let FuzzSubcommands::Run(args) = &self.command;
        args.junit
    }
}

#[derive(Clone, Debug, Subcommand)]
pub enum FuzzSubcommands {
    /// Run only fuzz and invariant tests.
    Run(FuzzRunArgs),
}
