#[macro_use]
extern crate tracing;

use crate::{
    eth::{
        backend::{info::StorageInfo, mem},
        fees::{FeeHistoryService, FeeManager},
        miner::{Miner, MiningMode},
        pool::Pool,
        sign::{DevSigner, Signer as EthSigner},
        EthApi,
    },
    filter::Filters,
    logging::{LoggingManager, NodeLogLayer},
    service::NodeService,
    shutdown::Signal,
    tasks::TaskManager,
};
use eth::backend::fork::ClientFork;
use ethers::{
    core::k256::ecdsa::SigningKey,
    prelude::Wallet,
    signers::Signer,
    types::{Address, U256},
};
use foundry_common::{ProviderBuilder, RetryProvider};
use foundry_evm::revm;
use futures::{FutureExt, TryFutureExt};
use parking_lot::Mutex;
use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    runtime::Handle,
    task::{JoinError, JoinHandle},
};

/// contains the background service that drives the node
mod service;

mod config;
pub use config::{AccountGenerator, NodeConfig, CHAIN_ID, VERSION_MESSAGE};
mod hardfork;
use crate::server::{
    error::{NodeError, NodeResult},
    spawn_ipc,
};
pub use hardfork::Hardfork;

/// ethereum related implementations
pub mod eth;
/// support for polling filters
pub mod filter;
/// support for handling `genesis.json` files
pub mod genesis;
/// commandline output
pub mod logging;
/// types for subscriptions
pub mod pubsub;
/// axum RPC server implementations
pub mod server;
/// Futures for shutdown signal
mod shutdown;
/// additional task management
mod tasks;

/// contains cli command
#[cfg(feature = "cmd")]
pub mod cmd;

/// Creates the node and runs the server
///
/// Returns the [EthApi] that can be used to interact with the node and the [JoinHandle] of the
/// task.
///
/// # Example
///
/// ```rust
/// # use anvil::NodeConfig;
/// # async fn spawn() {
/// let config = NodeConfig::default();
/// let (api, handle) = anvil::spawn(config).await;
///
/// // use api
///
/// // wait forever
/// handle.await.unwrap();
/// # }
/// ```
pub async fn spawn(mut config: NodeConfig) -> (EthApi, NodeHandle) {
    let logger = if config.enable_tracing { init_tracing() } else { Default::default() };
    logger.set_enabled(!config.silent);

    let backend = Arc::new(config.setup().await);

    if config.enable_auto_impersonate {
        backend.auto_impersonate_account(true).await;
    }

    let fork = backend.get_fork();

    let NodeConfig {
        signer_accounts,
        block_time,
        port,
        max_transactions,
        server_config,
        no_mining,
        transaction_order,
        genesis,
        ..
    } = config.clone();

    let pool = Arc::new(Pool::default());

    let mode = if let Some(block_time) = block_time {
        MiningMode::interval(block_time)
    } else if no_mining {
        MiningMode::None
    } else {
        // get a listener for ready transactions
        let listener = pool.add_ready_listener();
        MiningMode::instant(max_transactions, listener)
    };
    let miner = Miner::new(mode);

    let dev_signer: Box<dyn EthSigner> = Box::new(DevSigner::new(signer_accounts));
    let mut signers = vec![dev_signer];
    if let Some(genesis) = genesis {
        // include all signers from genesis.json if any
        let genesis_signers = genesis.private_keys();
        if !genesis_signers.is_empty() {
            let genesis_signers: Box<dyn EthSigner> = Box::new(DevSigner::new(genesis_signers));
            signers.push(genesis_signers);
        }
    }

    let fees = backend.fees().clone();
    let fee_history_cache = Arc::new(Mutex::new(Default::default()));
    let fee_history_service = FeeHistoryService::new(
        backend.new_block_notifications(),
        Arc::clone(&fee_history_cache),
        fees,
        StorageInfo::new(Arc::clone(&backend)),
    );

    let filters = Filters::default();

    // create the cloneable api wrapper
    let api = EthApi::new(
        Arc::clone(&pool),
        Arc::clone(&backend),
        Arc::new(signers),
        fee_history_cache,
        fee_history_service.fee_history_limit(),
        miner.clone(),
        logger,
        filters.clone(),
        transaction_order,
    );

    // spawn the node service
    let node_service =
        tokio::task::spawn(NodeService::new(pool, backend, miner, fee_history_service, filters));

    let mut servers = Vec::new();
    let mut addresses = Vec::new();

    for addr in config.host.iter() {
        let sock_addr = SocketAddr::new(addr.to_owned(), port);
        let srv = server::serve(sock_addr, api.clone(), server_config.clone());

        addresses.push(srv.local_addr());

        // spawn the server on a new task
        let srv = tokio::task::spawn(srv.map_err(NodeError::from));
        servers.push(srv);
    }

    let tokio_handle = Handle::current();
    let (signal, on_shutdown) = shutdown::signal();
    let task_manager = TaskManager::new(tokio_handle, on_shutdown);

    let ipc_task = config.get_ipc_path().map(|path| spawn_ipc(api.clone(), path));

    let handle = NodeHandle {
        config,
        node_service,
        servers,
        ipc_task,
        addresses,
        _signal: Some(signal),
        task_manager,
    };

    handle.print(fork.as_ref());

    (api, handle)
}

type IpcTask = JoinHandle<io::Result<()>>;

/// A handle to the spawned node and server tasks
///
/// This future will resolve if either the node or server task resolve/fail.
pub struct NodeHandle {
    config: NodeConfig,
    /// The address of the running rpc server
    addresses: Vec<SocketAddr>,
    /// Join handle for the Node Service
    pub node_service: JoinHandle<Result<(), NodeError>>,
    /// Join handles (one per socket) for the Anvil server.
    pub servers: Vec<JoinHandle<Result<(), NodeError>>>,
    // The future that joins the ipc server, if any
    ipc_task: Option<IpcTask>,
    /// A signal that fires the shutdown, fired on drop.
    _signal: Option<Signal>,
    /// A task manager that can be used to spawn additional tasks
    task_manager: TaskManager,
}

impl NodeHandle {
    /// The [NodeConfig] the node was launched with
    pub fn config(&self) -> &NodeConfig {
        &self.config
    }

    /// Prints the launch info
    pub(crate) fn print(&self, fork: Option<&ClientFork>) {
        self.config.print(fork);
        if !self.config.silent {
            println!(
                "Listening on {}",
                self.addresses
                    .iter()
                    .map(|addr| { addr.to_string() })
                    .collect::<Vec<String>>()
                    .join(", ")
            )
        }
    }

    /// The address of the launched server
    ///
    /// **N.B.** this may not necessarily be the same `host + port` as configured in the
    /// `NodeConfig`, if port was set to 0, then the OS auto picks an available port
    pub fn socket_address(&self) -> &SocketAddr {
        &self.addresses[0]
    }

    /// Returns the http endpoint
    pub fn http_endpoint(&self) -> String {
        format!("http://{}", self.socket_address())
    }

    /// Returns the websocket endpoint
    pub fn ws_endpoint(&self) -> String {
        format!("ws://{}", self.socket_address())
    }

    /// Returns the path of the launched ipc server, if any
    pub fn ipc_path(&self) -> Option<String> {
        self.config.get_ipc_path()
    }

    /// Constructs a [`RetryProvider`] for this handle's HTTP endpoint.
    pub fn http_provider(&self) -> RetryProvider {
        ProviderBuilder::new(&self.http_endpoint()).build().expect("failed to build HTTP provider")
        // .interval(Duration::from_millis(500))
    }

    /// Constructs a [`RetryProvider`] for this handle's WS endpoint.
    pub fn ws_provider(&self) -> RetryProvider {
        ProviderBuilder::new(&self.ws_endpoint()).build().expect("failed to build WS provider")
    }

    /// Constructs a [`RetryProvider`] for this handle's IPC endpoint, if any.
    pub fn ipc_provider(&self) -> Option<RetryProvider> {
        ProviderBuilder::new(&self.config.get_ipc_path()?).build().ok()
    }

    /// Signer accounts that can sign messages/transactions from the EVM node
    pub fn dev_accounts(&self) -> impl Iterator<Item = Address> + '_ {
        self.config.signer_accounts.iter().map(|wallet| wallet.address())
    }

    /// Signer accounts that can sign messages/transactions from the EVM node
    pub fn dev_wallets(&self) -> impl Iterator<Item = Wallet<SigningKey>> + '_ {
        self.config.signer_accounts.iter().cloned()
    }

    /// Accounts that will be initialised with `genesis_balance` in the genesis block
    pub fn genesis_accounts(&self) -> impl Iterator<Item = Address> + '_ {
        self.config.genesis_accounts.iter().map(|w| w.address())
    }

    /// Native token balance of every genesis account in the genesis block
    pub fn genesis_balance(&self) -> U256 {
        self.config.genesis_balance
    }

    /// Default gas price for all txs
    pub fn gas_price(&self) -> U256 {
        self.config.get_gas_price()
    }

    /// Returns the shutdown signal
    pub fn shutdown_signal(&self) -> &Option<Signal> {
        &self._signal
    }

    /// Returns mutable access to the shutdown signal
    ///
    /// This can be used to extract the Signal
    pub fn shutdown_signal_mut(&mut self) -> &mut Option<Signal> {
        &mut self._signal
    }

    /// Returns the task manager that can be used to spawn new tasks
    ///
    /// ```
    /// use anvil::NodeHandle;
    /// # fn t(handle: NodeHandle) {
    /// let task_manager = handle.task_manager();
    /// let on_shutdown = task_manager.on_shutdown();
    ///
    /// task_manager.spawn(async move {
    ///     on_shutdown.await;
    ///     // do something
    /// });
    ///
    /// # }
    /// ```
    pub fn task_manager(&self) -> &TaskManager {
        &self.task_manager
    }
}

impl Future for NodeHandle {
    type Output = Result<NodeResult<()>, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();

        // poll the ipc task
        if let Some(mut ipc) = pin.ipc_task.take() {
            if let Poll::Ready(res) = ipc.poll_unpin(cx) {
                return Poll::Ready(res.map(|res| res.map_err(NodeError::from)))
            } else {
                pin.ipc_task = Some(ipc);
            }
        }

        // poll the node service task
        if let Poll::Ready(res) = pin.node_service.poll_unpin(cx) {
            return Poll::Ready(res)
        }

        // poll the axum server handles
        for server in pin.servers.iter_mut() {
            if let Poll::Ready(res) = server.poll_unpin(cx) {
                return Poll::Ready(res)
            }
        }

        Poll::Pending
    }
}

#[allow(unused)]
#[doc(hidden)]
pub fn init_tracing() -> LoggingManager {
    use tracing_subscriber::prelude::*;

    let manager = LoggingManager::default();
    // check whether `RUST_LOG` is explicitly set
    if std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::Registry::default()
            .with(tracing_subscriber::EnvFilter::from_default_env())
            .with(tracing_subscriber::fmt::layer())
            .init();
    } else {
        tracing_subscriber::Registry::default()
            .with(NodeLogLayer::new(manager.clone()))
            .with(
                tracing_subscriber::fmt::layer()
                    .without_time()
                    .with_target(false)
                    .with_level(false),
            )
            .init();
    }

    manager
}
