//! Chisel CLI
//!
//! This module contains the core readline loop for the Chisel CLI as well as the
//! executable's `main` function.

use chisel::{
    history::chisel_history_file,
    prelude::{ChiselCommand, ChiselDispatcher, DispatchResult, SolidityHelper},
};
use clap::{Parser, Subcommand};
use eyre::Context;
use foundry_cli::{
    handler,
    opts::{CoreBuildArgs, GlobalOpts},
    utils::{self, LoadConfig},
};
use foundry_common::{evm::EvmArgs, fs};
use foundry_config::{
    figment::{
        value::{Dict, Map},
        Metadata, Profile, Provider,
    },
    Config,
};
use rustyline::{config::Configurer, error::ReadlineError, Editor};
use std::path::PathBuf;
use tracing::debug;
use yansi::Paint;

#[macro_use]
extern crate foundry_common;

#[cfg(all(feature = "jemalloc", unix))]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

// Loads project's figment and merges the build cli arguments into it
foundry_config::merge_impl_figment_convert!(Chisel, opts, evm_args);

const VERSION_MESSAGE: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("VERGEN_GIT_SHA"),
    " ",
    env!("VERGEN_BUILD_TIMESTAMP"),
    ")"
);

/// Fast, utilitarian, and verbose Solidity REPL.
#[derive(Debug, Parser)]
#[command(name = "chisel", version = VERSION_MESSAGE)]
pub struct Chisel {
    /// Include the global options.
    #[command(flatten)]
    pub global: GlobalOpts,

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
        chisel::session_source::MIN_VM_VERSION
    ))]
    pub no_vm: bool,

    #[command(flatten)]
    pub opts: CoreBuildArgs,

    #[command(flatten)]
    pub evm_args: EvmArgs,
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

fn main() {
    if let Err(err) = run() {
        let _ = foundry_common::sh_err!("{err:?}");
        std::process::exit(1);
    }
}

fn run() -> eyre::Result<()> {
    handler::install();
    utils::subscriber();
    utils::load_dotenv();

    let args = Chisel::parse();
    args.global.init()?;
    main_args(args)
}

#[allow(clippy::needless_return)]
#[tokio::main]
async fn main_args(args: Chisel) -> eyre::Result<()> {
    // Keeps track of whether or not an interrupt was the last input
    let mut interrupt = false;

    // Load configuration
    let (config, evm_opts) = args.load_config_and_evm_opts()?;

    // Create a new cli dispatcher
    let mut dispatcher = ChiselDispatcher::new(chisel::session_source::SessionSourceConfig {
        // Enable traces if any level of verbosity was passed
        traces: config.verbosity > 0,
        foundry_config: config,
        no_vm: args.no_vm,
        evm_opts,
        backend: None,
        calldata: None,
    })?;

    // Execute prelude Solidity source files
    evaluate_prelude(&mut dispatcher, args.prelude).await?;

    // Check for chisel subcommands
    match &args.cmd {
        Some(ChiselSubcommand::List) => {
            let sessions = dispatcher.dispatch_command(ChiselCommand::ListSessions, &[]).await;
            match sessions {
                DispatchResult::CommandSuccess(Some(session_list)) => {
                    sh_println!("{session_list}")?;
                }
                DispatchResult::CommandFailed(e) => sh_err!("{e}")?,
                _ => panic!("Unexpected result: Please report this bug."),
            }
            return Ok(())
        }
        Some(ChiselSubcommand::Load { id }) | Some(ChiselSubcommand::View { id }) => {
            // For both of these subcommands, we need to attempt to load the session from cache
            match dispatcher.dispatch_command(ChiselCommand::Load, &[id]).await {
                DispatchResult::CommandSuccess(_) => { /* Continue */ }
                DispatchResult::CommandFailed(e) => {
                    sh_err!("{e}")?;
                    return Ok(())
                }
                _ => panic!("Unexpected result! Please report this bug."),
            }

            // If the subcommand was `view`, print the source and exit.
            if matches!(args.cmd, Some(ChiselSubcommand::View { .. })) {
                match dispatcher.dispatch_command(ChiselCommand::Source, &[]).await {
                    DispatchResult::CommandSuccess(Some(source)) => {
                        sh_println!("{source}")?;
                    }
                    _ => panic!("Unexpected result! Please report this bug."),
                }
                return Ok(())
            }
        }
        Some(ChiselSubcommand::ClearCache) => {
            match dispatcher.dispatch_command(ChiselCommand::ClearCache, &[]).await {
                DispatchResult::CommandSuccess(Some(msg)) => sh_println!("{}", msg.green())?,
                DispatchResult::CommandFailed(e) => sh_err!("{e}")?,
                _ => panic!("Unexpected result! Please report this bug."),
            }
            return Ok(())
        }
        Some(ChiselSubcommand::Eval { command }) => {
            dispatch_repl_line(&mut dispatcher, command).await?;
            return Ok(())
        }
        None => { /* No chisel subcommand present; Continue */ }
    }

    // Create a new rustyline Editor
    let mut rl = Editor::<SolidityHelper, _>::new()?;
    rl.set_helper(Some(SolidityHelper::default()));

    // automatically add lines to history
    rl.set_auto_add_history(true);

    // load history
    if let Some(chisel_history) = chisel_history_file() {
        let _ = rl.load_history(&chisel_history);
    }

    // Print welcome header
    sh_println!("Welcome to Chisel! Type `{}` to show available commands.", "!help".green())?;

    // Begin Rustyline loop
    loop {
        // Get the prompt from the dispatcher
        // Variable based on status of the last entry
        let prompt = dispatcher.get_prompt();

        // Read the next line
        let next_string = rl.readline(prompt.as_ref());

        // Try to read the string
        match next_string {
            Ok(line) => {
                debug!("dispatching next line: {line}");
                // Clear interrupt flag
                interrupt = false;

                // Dispatch and match results
                let errored = dispatch_repl_line(&mut dispatcher, &line).await?;
                rl.helper_mut().unwrap().set_errored(errored);
            }
            Err(ReadlineError::Interrupted) => {
                if interrupt {
                    break
                } else {
                    sh_println!("(To exit, press Ctrl+C again)")?;
                    interrupt = true;
                }
            }
            Err(ReadlineError::Eof) => break,
            Err(err) => {
                sh_err!("{err:?}")?;
                break
            }
        }
    }

    if let Some(chisel_history) = chisel_history_file() {
        let _ = rl.save_history(&chisel_history);
    }

    Ok(())
}

/// [Provider] impl
impl Provider for Chisel {
    fn metadata(&self) -> Metadata {
        Metadata::named("Script Args Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, foundry_config::figment::Error> {
        Ok(Map::from([(Config::selected_profile(), Dict::default())]))
    }
}

/// Evaluate a single Solidity line.
async fn dispatch_repl_line(dispatcher: &mut ChiselDispatcher, line: &str) -> eyre::Result<bool> {
    let r = dispatcher.dispatch(line).await;
    match &r {
        DispatchResult::Success(msg) | DispatchResult::CommandSuccess(msg) => {
            debug!(%line, ?msg, "dispatch success");
            if let Some(msg) = msg {
                sh_println!("{}", msg.green())?;
            }
        },
        DispatchResult::UnrecognizedCommand(e) => sh_err!("{e}")?,
        DispatchResult::SolangParserFailed(e) => {
            sh_err!("{}", "Compilation error".red())?;
            sh_eprintln!("{}", format!("{e:?}").red())?;
        }
        DispatchResult::FileIoError(e) => sh_err!("{}", format!("File IO - {e}").red())?,
        DispatchResult::CommandFailed(msg) | DispatchResult::Failure(Some(msg)) => sh_err!("{}", msg.red())?,
        DispatchResult::Failure(None) => sh_err!("Please report this bug as a github issue if it persists: https://github.com/foundry-rs/foundry/issues/new/choose")?,
    }
    Ok(r.is_error())
}

/// Evaluate multiple Solidity source files contained within a
/// Chisel prelude directory.
async fn evaluate_prelude(
    dispatcher: &mut ChiselDispatcher,
    maybe_prelude: Option<PathBuf>,
) -> eyre::Result<()> {
    let Some(prelude_dir) = maybe_prelude else { return Ok(()) };
    if prelude_dir.is_file() {
        sh_println!("{} {}", "Loading prelude source file:".yellow(), prelude_dir.display())?;
        load_prelude_file(dispatcher, prelude_dir).await?;
        sh_println!("{}\n", "Prelude source file loaded successfully!".green())?;
    } else {
        let prelude_sources = fs::files_with_ext(&prelude_dir, "sol");
        let mut print_success_msg = false;
        for source_file in prelude_sources {
            print_success_msg = true;
            sh_println!("{} {}", "Loading prelude source file:".yellow(), source_file.display())?;
            load_prelude_file(dispatcher, source_file).await?;
        }

        if print_success_msg {
            sh_println!("{}\n", "All prelude source files loaded successfully!".green())?;
        }
    }
    Ok(())
}

/// Loads a single Solidity file into the prelude.
async fn load_prelude_file(dispatcher: &mut ChiselDispatcher, file: PathBuf) -> eyre::Result<()> {
    let prelude = fs::read_to_string(file)
        .wrap_err("Could not load source file. Are you sure this path is correct?")?;
    dispatch_repl_line(dispatcher, &prelude).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Chisel::command().debug_assert();
    }
}
