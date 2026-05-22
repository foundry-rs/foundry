use crate::{
    opts::{Chisel, ChiselSubcommand},
    prelude::{ChiselCommand, ChiselDispatcher, SolidityHelper},
};
use clap::Parser;
use eyre::{Context, Result};
use foundry_cli::utils::{self, LoadConfig};
use foundry_common::fs;
use foundry_config::Config;
#[cfg(feature = "optimism")]
use foundry_evm::core::evm::OpEvmNetwork;
use foundry_evm::{
    core::evm::{EthEvmNetwork, FoundryEvmNetwork, TempoEvmNetwork},
    opts::EvmOpts,
};
use rustyline::{Editor, config::Configurer, error::ReadlineError};
use std::{ops::ControlFlow, path::PathBuf};
use yansi::Paint;

/// Run the `chisel` command line interface.
pub fn run() -> Result<()> {
    // Pre-parse discovery flags run before `setup()` so they cannot be blocked
    // by panic-handler / tracing init failures and avoid that init's cost.
    foundry_cli::opts::GlobalArgs::check_introspect::<Chisel>();
    foundry_cli::opts::GlobalArgs::check_markdown_help::<Chisel>();

    setup()?;

    let args = Chisel::parse();
    args.global.init()?;
    args.global.tokio_runtime().block_on(run_command(args))
}

/// Setup the global logger and other utilities.
pub fn setup() -> Result<()> {
    utils::common_setup();
    utils::subscriber();

    Ok(())
}

macro_rules! try_cf {
    ($e:expr) => {
        match $e {
            ControlFlow::Continue(()) => {}
            ControlFlow::Break(()) => return Ok(()),
        }
    };
}

/// Run the subcommand.
pub async fn run_command(args: Chisel) -> Result<()> {
    // Load configuration
    let (mut config, mut evm_opts) = args.load_config_and_evm_opts()?;

    if let Some(chain) = config.chain {
        evm_opts.networks = evm_opts.networks.with_chain_id(chain.id());
    }
    evm_opts.infer_network_from_fork().await;
    config.networks = evm_opts.networks;

    if evm_opts.networks.is_tempo() {
        return run_command_with_network::<TempoEvmNetwork>(args, config, evm_opts).await;
    }

    #[cfg(feature = "optimism")]
    if evm_opts.networks.is_optimism() {
        return run_command_with_network::<OpEvmNetwork>(args, config, evm_opts).await;
    }

    run_command_with_network::<EthEvmNetwork>(args, config, evm_opts).await
}

async fn run_command_with_network<FEN: FoundryEvmNetwork>(
    args: Chisel,
    config: Config,
    evm_opts: EvmOpts,
) -> Result<()> {
    // Create a new cli dispatcher
    let mut dispatcher = ChiselDispatcher::<FEN>::new(crate::source::SessionSourceConfig {
        // Enable traces if any level of verbosity was passed
        traces: config.verbosity > 0,
        foundry_config: config,
        no_vm: args.no_vm,
        evm_opts,
        backend: None,
        calldata: None,
        ir_minimum: args.ir_minimum,
    })?;

    // Execute prelude Solidity source files
    evaluate_prelude(&mut dispatcher, args.prelude).await?;

    if let Some(cmd) = args.cmd {
        try_cf!(handle_cli_command(&mut dispatcher, cmd).await?);
        return Ok(());
    }

    let mut rl = Editor::<SolidityHelper, _>::new()?;
    rl.set_helper(Some(dispatcher.helper.clone()));
    rl.set_auto_add_history(true);
    if let Some(path) = chisel_history_file() {
        let _ = rl.load_history(&path);
    }

    sh_println!("Welcome to Chisel! Type `{}` to show available commands.", "!help".green())?;

    // REPL loop.
    let mut interrupt = false;
    loop {
        match rl.readline(&dispatcher.get_prompt()) {
            Ok(line) => {
                debug!("dispatching next line: {line}");
                // Clear interrupt flag.
                interrupt = false;

                // Dispatch and match results.
                let r = dispatcher.dispatch(&line).await;
                dispatcher.helper.set_errored(r.is_err());
                match r {
                    Ok(ControlFlow::Continue(())) => {}
                    Ok(ControlFlow::Break(())) => break,
                    Err(e) => {
                        sh_err!("{}", foundry_common::errors::display_chain(&e))?;
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                if interrupt {
                    break;
                }
                sh_println!("(To exit, press Ctrl+C again)")?;
                interrupt = true;
            }
            Err(ReadlineError::Eof) => break,
            Err(err) => {
                sh_err!("{err}")?;
                break;
            }
        }
    }

    if let Some(path) = chisel_history_file() {
        let _ = rl.save_history(&path);
    }

    Ok(())
}

/// Evaluate multiple Solidity source files contained within a
/// Chisel prelude directory.
async fn evaluate_prelude<FEN: FoundryEvmNetwork>(
    dispatcher: &mut ChiselDispatcher<FEN>,
    maybe_prelude: Option<PathBuf>,
) -> Result<()> {
    let Some(prelude_dir) = maybe_prelude else { return Ok(()) };
    if prelude_dir.is_file() {
        sh_println!("{} {}", "Loading prelude source file:".yellow(), prelude_dir.display())?;
        try_cf!(load_prelude_file(dispatcher, prelude_dir).await?);
        sh_println!("{}\n", "Prelude source file loaded successfully!".green())?;
    } else {
        let prelude_sources = fs::files_with_ext(&prelude_dir, "sol");
        let mut print_success_msg = false;
        for source_file in prelude_sources {
            print_success_msg = true;
            sh_println!("{} {}", "Loading prelude source file:".yellow(), source_file.display())?;
            try_cf!(load_prelude_file(dispatcher, source_file).await?);
        }

        if print_success_msg {
            sh_println!("{}\n", "All prelude source files loaded successfully!".green())?;
        }
    }
    Ok(())
}

/// Loads a single Solidity file into the prelude.
async fn load_prelude_file<FEN: FoundryEvmNetwork>(
    dispatcher: &mut ChiselDispatcher<FEN>,
    file: PathBuf,
) -> Result<ControlFlow<()>> {
    let prelude = fs::read_to_string(file)
        .wrap_err("Could not load source file. Are you sure this path is correct?")?;
    dispatcher.dispatch(&prelude).await
}

async fn handle_cli_command<FEN: FoundryEvmNetwork>(
    d: &mut ChiselDispatcher<FEN>,
    cmd: ChiselSubcommand,
) -> Result<ControlFlow<()>> {
    match cmd {
        ChiselSubcommand::List => d.dispatch_command(ChiselCommand::ListSessions).await,
        ChiselSubcommand::Load { id } => d.dispatch_command(ChiselCommand::Load { id }).await,
        ChiselSubcommand::View { id } => {
            let ControlFlow::Continue(()) = d.dispatch_command(ChiselCommand::Load { id }).await?
            else {
                return Ok(ControlFlow::Break(()));
            };
            d.dispatch_command(ChiselCommand::Source).await
        }
        ChiselSubcommand::ClearCache => d.dispatch_command(ChiselCommand::ClearCache).await,
        ChiselSubcommand::Eval { command } => d.dispatch(&command).await,
    }
}

fn chisel_history_file() -> Option<PathBuf> {
    foundry_config::Config::foundry_dir().map(|p| p.join(".chisel_history"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Chisel::command().debug_assert();
    }

    /// Every `command_id` exposed by `chisel --introspect` MUST be unique.
    #[test]
    fn introspect_command_ids_are_unique() {
        use foundry_cli::introspect::{CommandRegistry, build_document, duplicate_command_ids};
        let cmd = Chisel::command();
        let doc = build_document(&cmd, &CommandRegistry::EMPTY);
        let dups = duplicate_command_ids(&doc);
        assert!(dups.is_empty(), "duplicate chisel command_ids: {dups:?}");
    }

    /// `chisel --introspect` must produce a JSON document that parses back into
    /// the canonical `IntrospectDocument` shape.
    #[test]
    fn introspect_document_is_valid_json() {
        use foundry_cli::introspect::{
            CommandRegistry, INTROSPECT_SCHEMA_ID, IntrospectDocument, render_introspect_document,
        };
        let cmd = Chisel::command();
        let json = render_introspect_document(&cmd, &CommandRegistry::EMPTY);
        let doc: IntrospectDocument = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(doc.schema_id, INTROSPECT_SCHEMA_ID);
        assert_eq!(doc.binary.name, "chisel");
    }
}
