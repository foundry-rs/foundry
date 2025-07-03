use crate::{
    history::chisel_history_file,
    prelude::{ChiselCommand, ChiselDispatcher, DispatchResult, SolidityHelper},
};
use clap::Parser;
use eyre::{Context, Result};
use foundry_cli::{
    handler,
    utils::{self, LoadConfig},
};
use foundry_common::fs;
use foundry_config::{
    Config,
    figment::{
        Metadata, Profile, Provider,
        value::{Dict, Map},
    },
};
use rustyline::{Editor, config::Configurer, error::ReadlineError};
use std::path::PathBuf;
use tracing::debug;
use yansi::Paint;

use crate::opts::{Chisel, ChiselSubcommand};

/// Run the `chisel` command line interface.
pub fn run() -> Result<()> {
    setup()?;

    let args = Chisel::parse();
    args.global.init()?;

    run_command(args)
}

/// Setup the global logger and other utilities.
pub fn setup() -> Result<()> {
    utils::install_crypto_provider();
    handler::install();
    utils::subscriber();
    utils::load_dotenv();

    Ok(())
}

/// Run the subcommand.
#[tokio::main]
pub async fn run_command(args: Chisel) -> Result<()> {
    // Keeps track of whether or not an interrupt was the last input
    let mut interrupt = false;

    // Load configuration
    let (config, evm_opts) = args.load_config_and_evm_opts()?;

    // Create a new cli dispatcher
    let mut dispatcher = ChiselDispatcher::new(crate::session_source::SessionSourceConfig {
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
            return Ok(());
        }
        Some(ChiselSubcommand::Load { id }) | Some(ChiselSubcommand::View { id }) => {
            // For both of these subcommands, we need to attempt to load the session from cache
            match dispatcher.dispatch_command(ChiselCommand::Load, &[id]).await {
                DispatchResult::CommandSuccess(_) => { /* Continue */ }
                DispatchResult::CommandFailed(e) => {
                    sh_err!("{e}")?;
                    return Ok(());
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
                return Ok(());
            }
        }
        Some(ChiselSubcommand::ClearCache) => {
            match dispatcher.dispatch_command(ChiselCommand::ClearCache, &[]).await {
                DispatchResult::CommandSuccess(Some(msg)) => sh_println!("{}", msg.green())?,
                DispatchResult::CommandFailed(e) => sh_err!("{e}")?,
                _ => panic!("Unexpected result! Please report this bug."),
            }
            return Ok(());
        }
        Some(ChiselSubcommand::Eval { command }) => {
            dispatch_repl_line(&mut dispatcher, command).await?;
            return Ok(());
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
                    break;
                } else {
                    sh_println!("(To exit, press Ctrl+C again)")?;
                    interrupt = true;
                }
            }
            Err(ReadlineError::Eof) => break,
            Err(err) => {
                sh_err!("{err:?}")?;
                break;
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
async fn dispatch_repl_line(dispatcher: &mut ChiselDispatcher, line: &str) -> Result<bool> {
    let r = dispatcher.dispatch(line).await;
    match &r {
        DispatchResult::Success(msg) | DispatchResult::CommandSuccess(msg) => {
            debug!(%line, ?msg, "dispatch success");
            if let Some(msg) = msg {
                sh_println!("{}", msg.green())?;
            }
        }
        DispatchResult::UnrecognizedCommand(e) => sh_err!("{e}")?,
        DispatchResult::SolangParserFailed(e) => {
            sh_err!("{}", "Compilation error".red())?;
            sh_eprintln!("{}", format!("{e:?}").red())?;
        }
        DispatchResult::FileIoError(e) => sh_err!("{}", format!("File IO - {e}").red())?,
        DispatchResult::CommandFailed(msg) | DispatchResult::Failure(Some(msg)) => {
            sh_err!("{}", msg.red())?
        }
        DispatchResult::Failure(None) => sh_err!(
            "Please report this bug as a github issue if it persists: https://github.com/foundry-rs/foundry/issues/new/choose"
        )?,
    }
    Ok(r.is_error())
}

/// Evaluate multiple Solidity source files contained within a
/// Chisel prelude directory.
async fn evaluate_prelude(
    dispatcher: &mut ChiselDispatcher,
    maybe_prelude: Option<PathBuf>,
) -> Result<()> {
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
async fn load_prelude_file(dispatcher: &mut ChiselDispatcher, file: PathBuf) -> Result<()> {
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
