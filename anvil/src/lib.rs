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
    providers::{Http, Provider, Ws},
    signers::Signer,
    types::{Address, U256},
};
use foundry_evm::revm;
use futures::{FutureExt, TryFutureExt};
use parking_lot::Mutex;
use std::{
    future::Future,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
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

    let backend = Arc::new(config.setup().await);

    let fork = backend.get_fork().cloned();

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

    let host = config.host.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let mut addr = SocketAddr::new(host, port);

    // configure the rpc server and use its actual local address
    let server = server::serve(addr, api.clone(), server_config);
    addr = server.local_addr();

    // spawn the server on a new task
    let serve = tokio::task::spawn(server.map_err(NodeError::from));

    let tokio_handle = Handle::current();
    let (signal, on_shutdown) = shutdown::signal();
    let task_manager = TaskManager::new(tokio_handle, on_shutdown);

    let ipc_task = config.get_ipc_path().map(|path| spawn_ipc(api.clone(), path));

    let handle = NodeHandle {
        config,
        node_service: node_service,
        server: serve,
        ipc_task,
        address: addr,
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
    address: SocketAddr,
    /// Join handle for the Node Service
    pub node_service: JoinHandle<Result<(), NodeError>>,
    /// Join handle for the Anvil server
    pub server: JoinHandle<Result<(), NodeError>>,
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
            println!("Listening on {}", self.socket_address())
        }
    }

    /// The address of the launched server
    ///
    /// **N.B.** this may not necessarily be the same `host + port` as configured in the
    /// `NodeConfig`, if port was set to 0, then the OS auto picks an available port
    pub fn socket_address(&self) -> &SocketAddr {
        &self.address
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

    /// Returns a Provider for the http endpoint
    pub fn http_provider(&self) -> Provider<Http> {
        Provider::<Http>::try_from(self.http_endpoint())
            .unwrap()
            .interval(Duration::from_millis(500))
    }

    /// Connects to the websocket Provider of the node
    pub async fn ws_provider(&self) -> Provider<Ws> {
        Provider::new(
            Ws::connect(self.ws_endpoint()).await.expect("Failed to connect to node's websocket"),
        )
    }

    /// Connects to the ipc endpoint of the node, if spawned
    pub async fn ipc_provider(&self) -> Option<Provider<ethers::providers::Ipc>> {
        let ipc_path = self.config.get_ipc_path()?;
        tracing::trace!(target: "ipc", ?ipc_path, "connecting ipc provider");
        let provider = Provider::connect_ipc(&ipc_path).await.unwrap_or_else(|err| {
            panic!("Failed to connect to node's ipc endpoint {ipc_path}: {err:?}")
        });
        Some(provider)
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
                return Poll::Ready(res.map(|res| res.map_err(NodeError::from)));
            } else {
                pin.ipc_task = Some(ipc);
            }
        }

        // poll the node service task
        if let Poll::Ready(res) = pin.node_service.poll_unpin(cx) {
            return Poll::Ready(res);
        }

        pin.server.poll_unpin(cx)
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
