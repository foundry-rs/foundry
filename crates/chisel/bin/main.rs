//! Chisel CLI
//!
//! This module contains the core readline loop for the Chisel CLI as well as the
//! executable's `main` function.

use chisel::{
    history::chisel_history_file,
    prelude::{ChiselCommand, ChiselDispatcher, DispatchResult, SolidityHelper},
};
use clap::Parser;
use eyre::Context;
use foundry_cli::{
    handler,
    opts::CoreBuildArgs,
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
use yansi::Paint;

// Loads project's figment and merges the build cli arguments into it
foundry_config::merge_impl_figment_convert!(ChiselParser, opts, evm_opts);

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
#[clap(name = "chisel", version = VERSION_MESSAGE)]
pub struct ChiselParser {
    #[command(subcommand)]
    pub sub: Option<ChiselParserSub>,

    /// Path to a directory containing Solidity files to import, or path to a single Solidity file.
    ///
    /// These files will be evaluated before the top-level of the
    /// REPL, therefore functioning as a prelude
    #[clap(long, help_heading = "REPL options")]
    pub prelude: Option<PathBuf>,

    #[clap(flatten)]
    pub opts: CoreBuildArgs,

    #[clap(flatten)]
    pub evm_opts: EvmArgs,
}

/// Chisel binary subcommands
#[derive(clap::Subcommand, Debug)]
pub enum ChiselParserSub {
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
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    handler::install()?;
    utils::subscriber();
    #[cfg(windows)]
    if !Paint::enable_windows_ascii() {
        Paint::disable()
    }

    utils::load_dotenv();

    // Parse command args
    let args = ChiselParser::parse();

    // Keeps track of whether or not an interrupt was the last input
    let mut interrupt = false;

    // Load configuration
    let (config, evm_opts) = args.load_config_and_evm_opts()?;

    // Create a new cli dispatcher
    let mut dispatcher = ChiselDispatcher::new(chisel::session_source::SessionSourceConfig {
        // Enable traces if any level of verbosity was passed
        traces: config.verbosity > 0,
        foundry_config: config,
        evm_opts,
        backend: None,
        calldata: None,
    })?;

    // Execute prelude Solidity source files
    evaluate_prelude(&mut dispatcher, args.prelude).await?;

    // Check for chisel subcommands
    match &args.sub {
        Some(ChiselParserSub::List) => {
            let sessions = dispatcher.dispatch_command(ChiselCommand::ListSessions, &[]).await;
            match sessions {
                DispatchResult::CommandSuccess(Some(session_list)) => {
                    println!("{session_list}");
                }
                DispatchResult::CommandFailed(e) => eprintln!("{e}"),
                _ => panic!("Unexpected result: Please report this bug."),
            }
            return Ok(())
        }
        Some(ChiselParserSub::Load { id }) | Some(ChiselParserSub::View { id }) => {
            // For both of these subcommands, we need to attempt to load the session from cache
            match dispatcher.dispatch_command(ChiselCommand::Load, &[id]).await {
                DispatchResult::CommandSuccess(_) => { /* Continue */ }
                DispatchResult::CommandFailed(e) => {
                    eprintln!("{e}");
                    return Ok(())
                }
                _ => panic!("Unexpected result! Please report this bug."),
            }

            // If the subcommand was `view`, print the source and exit.
            if matches!(args.sub, Some(ChiselParserSub::View { .. })) {
                match dispatcher.dispatch_command(ChiselCommand::Source, &[]).await {
                    DispatchResult::CommandSuccess(Some(source)) => {
                        println!("{source}");
                    }
                    _ => panic!("Unexpected result! Please report this bug."),
                }
                return Ok(())
            }
        }
        Some(ChiselParserSub::ClearCache) => {
            match dispatcher.dispatch_command(ChiselCommand::ClearCache, &[]).await {
                DispatchResult::CommandSuccess(Some(msg)) => println!("{}", Paint::green(msg)),
                DispatchResult::CommandFailed(e) => eprintln!("{e}"),
                _ => panic!("Unexpected result! Please report this bug."),
            }
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
    println!("Welcome to Chisel! Type `{}` to show available commands.", Paint::green("!help"));

    // Begin Rustyline loop
    loop {
        // Get the prompt from the dispatcher
        // Variable based on status of the last entry
        let prompt = dispatcher.get_prompt();
        rl.helper_mut().unwrap().set_errored(dispatcher.errored);

        // Read the next line
        let next_string = rl.readline(prompt.as_ref());

        // Try to read the string
        match next_string {
            Ok(line) => {
                // Clear interrupt flag
                interrupt = false;

                // Dispatch and match results
                dispatch_repl_line(&mut dispatcher, &line).await;
            }
            Err(ReadlineError::Interrupted) => {
                if interrupt {
                    break
                } else {
                    println!("(To exit, press Ctrl+C again)");
                    interrupt = true;
                }
            }
            Err(ReadlineError::Eof) => break,
            Err(err) => {
                println!("Error: {err:?}");
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
impl Provider for ChiselParser {
    fn metadata(&self) -> Metadata {
        Metadata::named("Script Args Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, foundry_config::figment::Error> {
        Ok(Map::from([(Config::selected_profile(), Dict::default())]))
    }
}

/// Evaluate a single Solidity line.
async fn dispatch_repl_line(dispatcher: &mut ChiselDispatcher, line: &str) {
    match dispatcher.dispatch(line).await {
        DispatchResult::Success(msg) | DispatchResult::CommandSuccess(msg) => if let Some(msg) = msg {
            println!("{}", Paint::green(msg));
        },
        DispatchResult::UnrecognizedCommand(e) => eprintln!("{e}"),
        DispatchResult::SolangParserFailed(e) => {
            eprintln!("{}", Paint::red("Compilation error"));
            eprintln!("{}", Paint::red(format!("{e:?}")));
        }
        DispatchResult::FileIoError(e) => eprintln!("{}", Paint::red(format!("⚒️ Chisel File IO Error - {e}"))),
        DispatchResult::CommandFailed(msg) | DispatchResult::Failure(Some(msg)) => eprintln!("{}", Paint::red(msg)),
        DispatchResult::Failure(None) => eprintln!("{}\nPlease Report this bug as a github issue if it persists: https://github.com/foundry-rs/foundry/issues/new/choose", Paint::red("⚒️ Unknown Chisel Error ⚒️")),
    }
}

/// Evaluate multiple Solidity source files contained within a
/// Chisel prelude directory.
async fn evaluate_prelude(
    dispatcher: &mut ChiselDispatcher,
    maybe_prelude: Option<PathBuf>,
) -> eyre::Result<()> {
    let Some(prelude_dir) = maybe_prelude else { return Ok(()) };
    if prelude_dir.is_file() {
        println!("{} {}", Paint::yellow("Loading prelude source file:"), prelude_dir.display(),);
        load_prelude_file(dispatcher, prelude_dir).await?;
        println!("{}\n", Paint::green("Prelude source file loaded successfully!"));
    } else {
        let prelude_sources = fs::files_with_ext(prelude_dir, "sol");
        let print_success_msg = !prelude_sources.is_empty();
        for source_file in prelude_sources {
            println!("{} {}", Paint::yellow("Loading prelude source file:"), source_file.display(),);
            load_prelude_file(dispatcher, source_file).await?;
        }

        if print_success_msg {
            println!("{}\n", Paint::green("All prelude source files loaded successfully!"));
        }
    }
    Ok(())
}

/// Loads a single Solidity file into the prelude.
async fn load_prelude_file(dispatcher: &mut ChiselDispatcher, file: PathBuf) -> eyre::Result<()> {
    let prelude = fs::read_to_string(file)
        .wrap_err("Could not load source file. Are you sure this path is correct?")?;
    dispatch_repl_line(dispatcher, &prelude).await;
    Ok(())
}
