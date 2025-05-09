use clap::{Parser, Subcommand};
use foundry_cli::opts::{BuildOpts, GlobalArgs};
use foundry_common::{
    evm::EvmArgs,
    version::{LONG_VERSION, SHORT_VERSION},
};
use std::path::PathBuf;

// Loads project's figment and merges the build cli arguments into it
foundry_config::merge_impl_figment_convert!(Chisel, build, evm);

/// Fast, utilitarian, and verbose Solidity REPL.
#[derive(Debug, Parser)]
#[command(name = "chisel", version = SHORT_VERSION, long_version = LONG_VERSION)]
pub struct Chisel {
    /// Include the global arguments.
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub cmd: Option<ChiselSubcommand>,

    /// Path to a directory containing Solidity files to import, or path to a single Solidity file.
    ///
    /// These files will be evaluated before the top-level of the
    /// REPL, therefore functioning as a prelude
    #[arg(long, help_heading = "REPL options")]
    pub prelude: Option<PathBuf>,

    /// Disable the default `Vm` import.
    #[arg(long, help_heading = "REPL options", long_help = format!(
        "Disable the default `Vm` import.\n\n\
         The import is disabled by default if the Solc version is less than {}.",
        crate::session_source::MIN_VM_VERSION
    ))]
    pub no_vm: bool,

    #[command(flatten)]
    pub build: BuildOpts,

    #[command(flatten)]
    pub evm: EvmArgs,
}

/// Chisel binary subcommands
#[derive(Debug, Subcommand)]
pub enum ChiselSubcommand {
    /// List all cached sessions
    List,

    /// Load a cached session
    Load {
        /// The ID of the session to load.
        id: String,
    },

    /// View the source of a cached session
    View {
        /// The ID of the session to load.
        id: String,
    },

    /// Clear all cached chisel sessions from the cache directory
    ClearCache,

    /// Simple evaluation of a command without entering the REPL
    Eval {
        /// The command to be evaluated.
        command: String,
    },
}
