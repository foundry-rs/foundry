use crate::cmd::NodeArgs;
use clap::{Parser, Subcommand};
use foundry_cli::opts::GlobalArgs;
use foundry_common::version::{LONG_VERSION, SHORT_VERSION};

/// A fast local Ethereum development node.
#[derive(Parser)]
#[command(name = "anvil", version = SHORT_VERSION, long_version = LONG_VERSION, next_display_order = None)]
pub struct Anvil {
    /// Include the global arguments.
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(flatten)]
    pub node: NodeArgs,

    #[command(subcommand)]
    pub cmd: Option<AnvilSubcommand>,
}

#[derive(Subcommand)]
pub enum AnvilSubcommand {
    /// Generate shell completions script.
    #[command(visible_alias = "com")]
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Generate Fig autocompletion spec.
    #[command(visible_alias = "fig")]
    GenerateFigSpec,
}
