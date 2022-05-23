mod config;
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
};
pub use config::{AccountGenerator, NodeConfig, CHAIN_ID, VERSION_MESSAGE};
use eth::backend::fork::ClientFork;
use ethers::{
    core::k256::ecdsa::SigningKey,
    prelude::Wallet,
    providers::{Http, Provider, Ws},
    signers::Signer,
    types::{Address, U256},
};
use foundry_evm::revm;
use futures::FutureExt;
use parking_lot::Mutex;
use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::task::JoinError;

/// contains the background service that drives the node
mod service;

/// ethereum related implementations
pub mod eth;
/// support for polling filters
pub mod filter;
/// commandline output
pub mod logging;
/// types for subscriptions
pub mod pubsub;
/// axum RPC server implementations
pub mod server;

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
        Arc::new(vec![dev_signer]),
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
    let socket = SocketAddr::new(host, port);

    // launch the rpc server
    let serve = tokio::task::spawn(server::serve(socket, api.clone(), server_config));

    // select over both tasks
    let inner = futures::future::select(node_service, serve);

    let handle = NodeHandle {
        config,
        inner: Box::pin(async move {
            // wait for the first task to finish
            inner.await.into_inner().0
        }),
        address: socket,
    };

    handle.print(fork.as_ref());

    (api, handle)
}

type NodeFuture = Pin<Box<dyn Future<Output = Result<hyper::Result<()>, JoinError>>>>;

/// A handle to the spawned node and server
pub struct NodeHandle {
    config: NodeConfig,
    address: SocketAddr,
    /// the future that drives the rpc service and the node service
    inner: NodeFuture,
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
        self.config.gas_price
    }
}

impl Future for NodeHandle {
    type Output = Result<hyper::Result<()>, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();
        pin.inner.poll_unpin(cx)
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
