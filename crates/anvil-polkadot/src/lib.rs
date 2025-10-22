//! Anvil is a fast local Ethereum development node.

#![cfg_attr(docsrs, feature(doc_cfg))]

use crate::{
    api_server::ApiHandle,
    config::AnvilNodeConfig,
    logging::{LoggingManager, NodeLogLayer},
    substrate_node::{genesis::GenesisConfig, service::Service, snapshot::SnapshotManager},
};
use clap::{CommandFactory, Parser};
use eyre::Result;
use foundry_cli::utils;
use opts::{Anvil, AnvilSubcommand};
use polkadot_sdk::{
    sc_cli::{self, SubstrateCli, build_runtime},
    sc_service::{self, TaskManager},
};
use server::try_spawn_ipc;
use std::net::SocketAddr;

pub mod substrate_node;

pub mod config;

/// commandline output
pub mod logging;
/// types for subscriptions
pub mod pubsub;
/// axum RPC server implementations
pub mod server;
//node_info
mod macros;

pub mod api_server;

/// contains cli command
pub mod cmd;

pub mod opts;

#[macro_use]
extern crate tracing;

/// Run the `anvil` command line interface.
pub fn run() -> Result<()> {
    setup()?;

    let args = Anvil::parse();
    args.global.init()?;

    run_command(args)
}

/// Setup the panic handler and other utilities.
pub fn setup() -> Result<()> {
    utils::install_crypto_provider();
    foundry_cli::handler::install();
    utils::load_dotenv();
    utils::enable_paint();

    Ok(())
}

/// Run the subcommand.
pub fn run_command(args: Anvil) -> Result<()> {
    if let Some(cmd) = &args.cmd {
        match cmd {
            AnvilSubcommand::Completions { shell } => {
                clap_complete::generate(
                    *shell,
                    &mut Anvil::command(),
                    "anvil-polkadot",
                    &mut std::io::stdout(),
                );
            }
            AnvilSubcommand::GenerateFigSpec => clap_complete::generate(
                clap_complete_fig::Fig,
                &mut Anvil::command(),
                "anvil-polkadot",
                &mut std::io::stdout(),
            ),
        }
        return Ok(());
    }

    let (anvil_config, substrate_config) = args.node.into_node_config()?;

    let substrate_client =
        opts::SubstrateCli { genesis_config: GenesisConfig::from(&anvil_config) };

    let tokio_runtime = build_runtime()?;

    let signals = tokio_runtime.block_on(async { sc_cli::Signals::capture() })?;
    let config =
        substrate_client.create_configuration(&substrate_config, tokio_runtime.handle().clone())?;
    let logging_manager = if anvil_config.enable_tracing {
        init_tracing(anvil_config.silent)
    } else {
        LoggingManager::default()
    };

    let runner: sc_cli::Runner<opts::SubstrateCli> =
        sc_cli::Runner::new(config, tokio_runtime, signals)?;

    Ok(runner.run_node_until_exit(|config| async move {
        let (_service, task_manager, ..) = spawn(anvil_config, config, logging_manager).await?;
        Ok::<TaskManager, sc_cli::Error>(task_manager)
    })?)
}

pub async fn spawn(
    anvil_config: AnvilNodeConfig,
    substrate_config: sc_service::Configuration,
    logging_manager: LoggingManager,
) -> Result<(Service, TaskManager, ApiHandle), sc_cli::Error> {
    // Spawn the substrate node.
    let (substrate_service, task_manager) =
        substrate_node::service::new(&anvil_config, substrate_config)
            .map_err(sc_cli::Error::Service)?;
    let snapshot_manager =
        SnapshotManager::new(substrate_service.client.clone(), substrate_service.backend.clone());

    // Spawn the other tasks.
    let api_handle =
        spawn_anvil_tasks(anvil_config, &substrate_service, logging_manager, snapshot_manager)
            .await
            .map_err(|err| sc_cli::Error::Application(err.into()))?;

    Ok((substrate_service, task_manager, api_handle))
}

pub async fn spawn_anvil_tasks(
    anvil_config: AnvilNodeConfig,
    service: &Service,
    logging_manager: LoggingManager,
    snapshot_manager: SnapshotManager,
) -> Result<ApiHandle> {
    // Spawn the api server.
    let api_handle = api_server::spawn(&anvil_config, service, logging_manager, snapshot_manager);

    // Spawn the network servers.
    for addr in &anvil_config.host {
        let sock_addr = SocketAddr::new(*addr, anvil_config.port);

        // Create a TCP listener.
        let tcp_listener = tokio::net::TcpListener::bind(sock_addr).await?;

        // Spawn the server future on a new task.
        let srv =
            server::serve_on(tcp_listener, anvil_config.server_config.clone(), api_handle.clone());
        service
            .spawn_handle
            .spawn("anvil", "anvil-tcp", async move { srv.await.expect("TCP server failure") });
    }

    // If configured, spawn the IPC server.
    anvil_config
        .get_ipc_path()
        .map(|path| try_spawn_ipc(&service.spawn_handle, path, api_handle.clone()))
        .transpose()?;

    anvil_config.print()?;

    Ok(api_handle)
}

pub fn init_tracing(silent: bool) -> LoggingManager {
    use tracing_subscriber::prelude::*;

    let manager = LoggingManager::default();
    manager.set_enabled(!silent);

    let env_filter = if !silent && std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::EnvFilter::from_default_env()
    } else {
        tracing_subscriber::EnvFilter::new("warn,node=debug")
    };

    let _ = if std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::Registry::default()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer())
            .try_init()
    } else {
        // Default filter: show substrate warnings/errors and our node targets
        tracing_subscriber::Registry::default()
            .with(env_filter)
            .with(NodeLogLayer::new(manager.clone()))
            .with(
                tracing_subscriber::fmt::layer()
                    .without_time()
                    .with_target(false)
                    .with_level(false),
            )
            .try_init()
    };

    manager
}
