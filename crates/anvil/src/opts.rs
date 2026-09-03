use crate::cmd::NodeArgs;
use clap::{Parser, Subcommand};
use foundry_cli::opts::GlobalArgs;
use foundry_common::version::{LONG_VERSION, SHORT_VERSION};

/// A fast local Ethereum development node
///
/// Examples:
/// - anvil (start a local node on 127.0.0.1:8545)
/// - anvil --fork-url $RPC_URL (fork the latest state of a live network)
/// - anvil --block-time 12 (mine a new block every 12 seconds)
/// - anvil --state state.json (load state if it exists and dump it on exit)
#[derive(Parser)]
#[command(verbatim_doc_comment, name = "anvil", version = SHORT_VERSION, long_version = LONG_VERSION, next_display_order = None)]
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
        shell: foundry_cli::clap::Shell,
    },
}
