use crate::{
    opts::{Chisel, ChiselSubcommand},
    prelude::{ChiselCommand, ChiselDispatcher, SolidityHelper},
};
use clap::Parser;
use eyre::{Context, Result};
use foundry_cli::utils::{self, LoadConfig};
use foundry_common::fs;
use foundry_config::Config;
#[cfg(feature = "monad")]
use foundry_evm::core::evm::MonadEvmNetwork;
#[cfg(feature = "optimism")]
use foundry_evm::core::evm::OpEvmNetwork;
use foundry_evm::{
    core::evm::{EthEvmNetwork, FoundryEvmNetwork, TempoEvmNetwork},
    executors::ExecutorBuilder,
    opts::EvmOpts,
};
use foundry_evm_networks::NetworkConfigs;
use rustyline::{Editor, config::Configurer, error::ReadlineError};
use std::{ops::ControlFlow, path::PathBuf};
use yansi::Paint;

/// Run the `chisel` command line interface.
pub fn run() -> Result<()> {
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

    evm_opts.networks =
        infer_network_from_chain_id(evm_opts.networks, config.chain.map(|chain| chain.id()))?;
    evm_opts.infer_network_from_fork().await?;
    evm_opts.pin_fork_block().await?;
    config.networks = evm_opts.networks;
    let local_networks = evm_opts.networks;
    let local_chain_id = evm_opts.env.chain_id.or(config.chain.map(|chain| chain.id()));

    if evm_opts.networks.is_tempo() {
        return Box::pin(run_command_with_network::<TempoEvmNetwork>(
            args,
            config,
            evm_opts,
            ExecutorBuilder::<TempoEvmNetwork>::new(),
            local_networks,
            local_chain_id,
        ))
        .await;
    }

    #[cfg(feature = "base")]
    if evm_opts.networks.is_base() {
        return Box::pin(run_command_with_network::<foundry_evm::core::evm::BaseEvmNetwork>(
            args,
            config,
            evm_opts,
            ExecutorBuilder::<foundry_evm::core::evm::BaseEvmNetwork>::new(),
            local_networks,
            local_chain_id,
        ))
        .await;
    }

    #[cfg(feature = "monad")]
    if evm_opts.networks.is_monad() {
        return Box::pin(run_command_with_network::<MonadEvmNetwork>(
            args,
            config,
            evm_opts,
            ExecutorBuilder::<MonadEvmNetwork>::new(),
            local_networks,
            local_chain_id,
        ))
        .await;
    }

    #[cfg(feature = "optimism")]
    if evm_opts.networks.is_optimism() {
        return Box::pin(run_command_with_network::<OpEvmNetwork>(
            args,
            config,
            evm_opts,
            ExecutorBuilder::<OpEvmNetwork>::new(),
            local_networks,
            local_chain_id,
        ))
        .await;
    }

    Box::pin(run_command_with_network::<EthEvmNetwork>(
        args,
        config,
        evm_opts,
        ExecutorBuilder::<EthEvmNetwork>::new(),
        local_networks,
        local_chain_id,
    ))
    .await
}

fn infer_network_from_chain_id(
    networks: NetworkConfigs,
    chain_id: Option<u64>,
) -> Result<NetworkConfigs> {
    if let Some(chain_id) = chain_id {
        networks.try_with_chain_id(chain_id).map_err(eyre::Report::msg)
    } else {
        Ok(networks)
    }
}

async fn run_command_with_network<FEN: FoundryEvmNetwork>(
    args: Chisel,
    config: Config,
    evm_opts: EvmOpts,
    executor_builder: ExecutorBuilder<FEN>,
    local_networks: NetworkConfigs,
    local_chain_id: Option<u64>,
) -> Result<()> {
    let fork_network_is_inferred = evm_opts.fork_network_is_inferred;
    let fork_chain_id_is_inferred = evm_opts.fork_chain_id_is_inferred;
    // Create a new cli dispatcher
    let mut dispatcher = ChiselDispatcher::<FEN>::new(crate::source::SessionSourceConfig {
        // Enable traces if any level of verbosity was passed
        traces: config.verbosity > 0,
        foundry_config: config,
        no_vm: args.no_vm,
        evm_opts,
        executor_builder,
        local_networks: Some(local_networks),
        local_chain_id,
        fork_network_is_inferred,
        fork_chain_id_is_inferred,
        resolved_hardfork: None,
        source_chain_id: None,
        cached_backend: None,
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
        let prompt = dispatcher.get_prompt();
        match rl.readline(prompt.as_ref()) {
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
    dispatcher.dispatch_solidity(&prelude).await
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

    /// Base chain IDs resolved to Optimism before Base support existed, so a build without the
    /// `base` feature — which is what release binaries ship — must keep resolving them that way.
    #[test]
    #[cfg(all(not(feature = "base"), feature = "optimism"))]
    fn chain_id_without_base_still_resolves_to_optimism() {
        for chain_id in [8453, 84532] {
            let networks = infer_network_from_chain_id(NetworkConfigs::default(), Some(chain_id))
                .unwrap_or_else(|error| panic!("chain ID {chain_id} must still resolve: {error}"));
            assert!(networks.is_optimism(), "chain ID {chain_id} must resolve to Optimism");
        }
    }

    #[test]
    #[cfg(not(feature = "monad"))]
    fn chain_id_rejects_disabled_monad_network() {
        let error = infer_network_from_chain_id(NetworkConfigs::default(), Some(143)).unwrap_err();

        assert_eq!(
            error.to_string(),
            "cannot infer execution network from chain ID 143: network family `monad` is not \
             enabled in this build"
        );
    }

    #[test]
    fn explicit_ethereum_overrides_chain_id_inference() {
        let ethereum = NetworkConfigs::with_ethereum();
        for chain_id in [8453, 143] {
            assert_eq!(infer_network_from_chain_id(ethereum, Some(chain_id)).unwrap(), ethereum);
        }
    }

    #[tokio::test]
    async fn prelude_does_not_dispatch_chisel_commands() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("prelude.sol");
        std::fs::write(&file, "!calldata 0x00").unwrap();
        let config = crate::source::SessionSourceConfig::<EthEvmNetwork> {
            foundry_config: Config {
                solc: Some(foundry_config::SolcReq::Version(semver::Version::new(0, 8, 29))),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut dispatcher = ChiselDispatcher::new(config).unwrap();

        assert!(load_prelude_file(&mut dispatcher, file).await.is_err());
    }
}
