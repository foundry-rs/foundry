use crate::{
    api::{
        AnvilApiServer, AnvilRpc, EthExtApiServer, EvmApiServer, NewFilterHook, PersonalApiServer,
        PoolRefresh, Web3ExtApiServer,
    },
    block_env::BlockEnvOverrides,
    config::NodeConfig,
    console::{ConsolePrinter, HARDHAT_CONSOLE_ADDRESS},
    debug::{AnvilDebugApi, AnvilDebugApiServer, AnvilTraceApi, AnvilTraceApiServer},
    eth_api::EthApi,
    fork::{ForkHeader, ForkInfo, ForkNetwork, ForkReplay},
    impersonation::{ImpersonatedSigner, ImpersonationState},
    launcher::AnvilNodeLauncher,
    logging::{LoggingState, log_mined_blocks},
    miner::{AnvilMiner, HookFuture},
    mining::{
        MiningController, MiningMode, PendingTxs, pool_pending_after, run_automine_task,
        run_interval_mining_task,
    },
    network::{AnvilComponents, AnvilNetwork, AnvilTypes, Prepared, ethereum::Ethereum},
    pool::SharedTransactionOrder,
    provider::AnvilProvider,
    server::{RpcServer, ServerSettings, SharedModule},
    signer::DevSigner,
    snapshot::SnapshotManager,
    state::{AnvilState, SharedAnvilState},
    time::TimeManager,
    txpool::{AnvilTxPool, AnvilTxPoolApiServer},
    types::TransactionOrder,
};
use alloy_consensus::{BlockHeader, transaction::TxHashRef};
use alloy_primitives::{Address, B256, U256};
use alloy_rpc_types_eth::FilterBlockOption;
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use eyre::{Result, WrapErr};
use foundry_common::provider::{ProviderBuilder, RetryProvider};
use foundry_evm_networks::NetworkVariant;
use jsonrpsee::{RpcModule, core::RpcResult};
use parking_lot::RwLock;
use reth_ethereum::{
    chainspec::{EthChainSpec, EthereumHardforks},
    node::{
        api::{FullNodeComponents, NodeTypes},
        builder::{LaunchNode, NodeBuilder, NodeHandle as RethNodeHandle},
        core::{
            args::{DatadirArgs, EngineArgs, PayloadBuilderArgs, RpcServerArgs, StorageArgs},
            dirs::{DataDirPath, MaybePlatformPath},
            exit::NodeExitFuture,
            node_config::NodeConfig as RethNodeConfig,
        },
    },
    pool::{
        CanonicalStateUpdate, PoolTransaction, PoolUpdateKind, TransactionOrigin, TransactionPool,
        TransactionPoolExt,
    },
    primitives::{Bytecode, NodePrimitives, Recovered, SignedTransaction},
    provider::{
        CanonStateNotifications, CanonStateSubscriptions, HeaderProvider,
        db::{
            ClientVersion, init_db,
            mdbx::{DatabaseArguments, GIGABYTE, MEGABYTE},
        },
    },
    rpc::builder::{RpcModuleSelection, constants::MAX_ETH_PROOF_WINDOW},
    storage::{BlockNumReader, BlockReader, TransactionVariant},
    tasks::{Runtime, RuntimeBuilder, RuntimeConfig, TokioConfig},
};
use reth_rpc_eth_api::{
    EthFilterApiServer,
    helpers::{
        EthTransactions,
        config::{EthConfigApiServer, EthConfigHandler},
    },
};
use std::{
    collections::BTreeMap,
    net::{SocketAddr, TcpListener},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tempfile::TempDir;
use tokio::{
    runtime::Handle,
    sync::{broadcast::error::RecvError, mpsc, oneshot},
};

/// A running node.
#[derive(Debug)]
pub struct NodeHandle {
    config: NodeConfig,
    address: SocketAddr,
    /// Resolves when the node exits.
    pub node_exit_future: NodeExit,
    /// Stops the node when the handle and every in-process API drop.
    _shutdown: Arc<oneshot::Sender<()>>,
    /// Stops the RPC servers when the handle drops, as on anvil; the in-process API keeps the
    /// node.
    _server_shutdown: oneshot::Sender<()>,
}

/// Resolves when the node exits, with its exit result.
#[derive(Debug)]
pub struct NodeExit(oneshot::Receiver<Result<()>>);

impl Future for NodeExit {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(cx).map(|result| result.unwrap_or(Ok(())))
    }
}

/// The reth node and the resources it runs on. Replaced on a relaunch.
struct RunningNode {
    node_exit_future: NodeExitFuture,
    _datadir: TempDir,
    runtime: Runtime,
}

impl RunningNode {
    /// Shuts the node's tasks down and removes its datadir.
    async fn stop(self) {
        let runtime = self.runtime;
        let _ = tokio::task::spawn_blocking(move || {
            runtime.graceful_shutdown_with_timeout(Duration::from_secs(10))
        })
        .await;
    }
}

/// A request to replace the running node with one built from a new config.
pub(crate) struct Relaunch {
    config: NodeConfig,
    reply: oneshot::Sender<Result<(), String>>,
}

/// Replaces the running node from inside the `anvil_*` namespace.
#[derive(Clone, Debug)]
pub struct Relauncher {
    requests: mpsc::UnboundedSender<Relaunch>,
    /// The config the current node was launched from, before the network prepared it.
    config: Arc<RwLock<NodeConfig>>,
    /// The config the first node was launched from.
    original: Arc<NodeConfig>,
}

impl Relauncher {
    /// Replaces the running node with one launched from the current config changed by `update`.
    /// Returns once the new node serves requests.
    pub async fn relaunch(&self, update: impl FnOnce(&mut NodeConfig)) -> Result<(), String> {
        let config = self.config.read().clone();
        self.relaunch_with(config, update).await
    }

    /// Changes the config the next relaunch starts from, without relaunching.
    pub fn update_config(&self, update: impl FnOnce(&mut NodeConfig)) {
        update(&mut self.config.write());
    }

    /// Returns the config the next relaunch starts from.
    pub fn current_config(&self) -> NodeConfig {
        self.config.read().clone()
    }

    /// Replaces the running node with one launched from the first node's config changed by
    /// `update`.
    pub async fn relaunch_from_original(
        &self,
        update: impl FnOnce(&mut NodeConfig),
    ) -> Result<(), String> {
        self.relaunch_with((*self.original).clone(), update).await
    }

    async fn relaunch_with(
        &self,
        mut config: NodeConfig,
        update: impl FnOnce(&mut NodeConfig),
    ) -> Result<(), String> {
        update(&mut config);
        let (reply, rx) = oneshot::channel();
        let stopped = || "the node supervisor has stopped".to_string();
        self.requests.send(Relaunch { config, reply }).map_err(|_| stopped())?;
        rx.await.map_err(|_| stopped())?
    }
}

impl NodeHandle {
    /// Returns the node config.
    pub const fn config(&self) -> &NodeConfig {
        &self.config
    }

    /// Returns the address the RPC server listens on.
    pub const fn socket_address(&self) -> &SocketAddr {
        &self.address
    }

    /// Returns the HTTP endpoint.
    pub fn http_endpoint(&self) -> String {
        format!("http://{}", self.address)
    }

    /// Returns the WebSocket endpoint.
    pub fn ws_endpoint(&self) -> String {
        format!("ws://{}", self.address)
    }

    /// Returns a provider for the HTTP endpoint.
    pub fn http_provider(&self) -> RetryProvider {
        ProviderBuilder::new(&self.http_endpoint()).build().expect("failed to build HTTP provider")
    }

    /// Returns the IPC endpoint path, if the node serves one.
    pub fn ipc_path(&self) -> Option<String> {
        self.config.get_ipc_path()
    }

    /// Returns a provider for the IPC endpoint, if the node serves one.
    pub fn ipc_provider(&self) -> Option<RetryProvider> {
        ProviderBuilder::new(&self.config.get_ipc_path()?).build().ok()
    }

    /// Returns a provider for the WebSocket endpoint.
    pub fn ws_provider(&self) -> RetryProvider {
        ProviderBuilder::new(&self.ws_endpoint()).build().expect("failed to build WS provider")
    }

    /// Returns the accounts the node signs for.
    pub fn dev_accounts(&self) -> impl Iterator<Item = Address> + '_ {
        self.config.signer_accounts.iter().map(|wallet| wallet.address())
    }

    /// Returns the wallets the node signs with, set to the chain id the node runs with.
    pub fn dev_wallets(&self) -> impl Iterator<Item = PrivateKeySigner> + '_ {
        let chain_id = self.config.get_chain_id();
        self.config.signer_accounts.iter().map(move |wallet| {
            let mut wallet = wallet.clone();
            wallet.set_chain_id(Some(chain_id));
            wallet
        })
    }

    /// Returns the accounts funded in genesis.
    pub fn genesis_accounts(&self) -> impl Iterator<Item = Address> + '_ {
        self.config.genesis_accounts.iter().map(|wallet| wallet.address())
    }

    /// Returns the balance of every genesis account.
    pub const fn genesis_balance(&self) -> U256 {
        self.config.genesis_balance
    }

    /// Prints the startup banner and the listening address, unless the config is silent.
    pub fn print(&self) -> Result<()> {
        self.config.print()?;
        if !self.config.silent {
            if let Some(ipc_path) = self.config.get_ipc_path() {
                foundry_common::sh_println!("IPC path: {ipc_path}")?;
            }
            foundry_common::sh_println!("Listening on {}", self.address)?;
        }
        Ok(())
    }
}

/// Launches a node and panics on failure. See [`try_spawn`].
pub async fn spawn(config: NodeConfig) -> (EthApi, NodeHandle) {
    try_spawn(config).await.expect("failed to spawn node")
}

/// Launches a reth dev node with the `anvil_*` namespace and anvil's mining, time, state, and
/// impersonation controls.
///
/// The node runs on a fresh MDBX database in a temporary directory that is removed when the
/// returned handle and every clone of the returned API drop. The node tasks run on the current
/// tokio runtime.
pub async fn try_spawn(mut config: NodeConfig) -> Result<(EthApi, NodeHandle)> {
    config.resolve_networks().await?;
    match config.networks.resolved_network().unwrap_or_default() {
        NetworkVariant::Ethereum => launch::<Ethereum>(config).await,
        #[cfg(feature = "monad")]
        NetworkVariant::Monad => launch::<crate::network::monad::Monad>(config).await,
        #[cfg(feature = "tempo")]
        NetworkVariant::Tempo => launch::<crate::network::tempo::Tempo>(config).await,
        // Other crates may enable more variants than this crate runs.
        #[allow(unreachable_patterns)]
        network => eyre::bail!("the {network:?} network is not supported yet"),
    }
}

/// Launches a node of the given network and the RPC server in front of it.
pub(crate) async fn launch<Net: AnvilNetwork>(config: NodeConfig) -> Result<(EthApi, NodeHandle)> {
    let address = SocketAddr::new(config.host[0], rpc_port(config.port)?);
    let instance_id = Arc::new(RwLock::new(B256::random()));
    let (requests, relaunches) = mpsc::unbounded_channel();
    let relauncher = Relauncher {
        requests,
        config: Arc::new(RwLock::new(config.clone())),
        original: Arc::new(config.clone()),
    };
    let mut launch_config = config.clone();
    let prepared = Net::prepare(&mut launch_config).await?;
    let (module, running) =
        launch_node::<Net>(launch_config, prepared, instance_id.clone(), relauncher.clone())
            .await?;
    // The handle reports the config the node runs with: a fork may have chosen the chain id. A
    // hardfork the endpoint chose stays out, as anvil keeps it out of the user's settings.
    let mut config = relauncher.config.read().clone();
    if config.adopted_hardfork {
        config.hardfork = None;
    }
    let module: SharedModule = Arc::new(RwLock::new(module));
    let logging = LoggingState::new(!config.silent);
    let server =
        RpcServer::start(address, ServerSettings::from_config(&config), module.clone(), logging)
            .await?;
    let address = server.address();

    let (exit_tx, exit_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel();
    tokio::spawn(supervise::<Net>(Supervisor {
        running: Some(running),
        server: Some(server),
        module: module.clone(),
        instance_id: instance_id.clone(),
        relauncher,
        relaunches,
        exit: Some(exit_tx),
        shutdown: shutdown_rx,
        server_shutdown: Some(server_shutdown_rx),
    }));

    let shutdown = Arc::new(shutdown_tx);
    Ok((
        EthApi::new(module, instance_id, shutdown.clone()),
        NodeHandle {
            config,
            address,
            node_exit_future: NodeExit(exit_rx),
            _shutdown: shutdown,
            _server_shutdown: server_shutdown_tx,
        },
    ))
}

/// Owns the running node and replaces it on request.
struct Supervisor {
    running: Option<RunningNode>,
    server: Option<RpcServer>,
    module: SharedModule,
    instance_id: Arc<RwLock<B256>>,
    relauncher: Relauncher,
    relaunches: mpsc::UnboundedReceiver<Relaunch>,
    exit: Option<oneshot::Sender<Result<()>>>,
    shutdown: oneshot::Receiver<()>,
    /// Fires when the node handle drops; the RPC servers stop, the node runs on.
    server_shutdown: Option<oneshot::Receiver<()>>,
}

async fn supervise<Net: AnvilNetwork>(mut supervisor: Supervisor) {
    while let Some(running) = supervisor.running.as_mut() {
        tokio::select! {
            request = supervisor.relaunches.recv() => {
                let Some(Relaunch { config, reply }) = request else { break };
                let result = relaunch::<Net>(&mut supervisor, config).await;
                let _ = reply.send(result.map_err(|error| error.to_string()));
            }
            result = &mut running.node_exit_future => {
                supervisor.running = None;
                if let Some(exit) = supervisor.exit.take() {
                    let _ = exit.send(result);
                }
                break;
            }
            _ = &mut supervisor.shutdown => break,
            _ = async {
                match supervisor.server_shutdown.as_mut() {
                    Some(server_shutdown) => {
                        let _ = server_shutdown.await;
                    }
                    None => std::future::pending().await,
                }
            } => {
                supervisor.server_shutdown = None;
                if let Some(server) = supervisor.server.take() {
                    server.stop();
                }
            }
        }
    }
    if let Some(server) = supervisor.server.take() {
        server.stop();
    }
    if let Some(running) = supervisor.running.take() {
        running.stop().await;
    }
}

/// Stops the running node and launches one from `config`. A config whose fork cannot be set up
/// leaves the running node untouched, as anvil's failed resets do. If the launch itself fails,
/// the previous config is launched again, so the node keeps serving.
async fn relaunch<Net: AnvilNetwork>(
    supervisor: &mut Supervisor,
    mut config: NodeConfig,
) -> Result<()> {
    // An endpoint that stalled on an earlier launch may answer now.
    config.stalled_endpoints = Default::default();
    let prepared = Net::prepare(&mut config).await?;
    if let Some(running) = supervisor.running.take() {
        running.stop().await;
    }
    let launch = |config: NodeConfig, prepared| {
        launch_node::<Net>(
            config,
            prepared,
            supervisor.instance_id.clone(),
            supervisor.relauncher.clone(),
        )
    };
    let (result, config) = match launch(config.clone(), prepared).await {
        Ok(launched) => (Ok(launched), config),
        Err(error) => {
            let mut previous = supervisor.relauncher.config.read().clone();
            let prepared = Net::prepare(&mut previous).await?;
            match launch(previous.clone(), prepared).await {
                Ok(launched) => {
                    *supervisor.module.write() = launched.0;
                    supervisor.running = Some(launched.1);
                    (Err(error), previous)
                }
                Err(fallback) => {
                    if let Some(exit) = supervisor.exit.take() {
                        let _ = exit.send(Err(fallback));
                    }
                    return Err(error);
                }
            }
        }
    };
    *supervisor.relauncher.config.write() = config;
    *supervisor.instance_id.write() = B256::random();
    match result {
        Ok((module, running)) => {
            *supervisor.module.write() = module;
            supervisor.running = Some(running);
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Launches the reth node of the given network, without RPC servers, and returns its RPC module.
async fn launch_node<Net: AnvilNetwork>(
    config: NodeConfig,
    prepared: Prepared<Net::Node>,
    instance_id: Arc<RwLock<B256>>,
    relauncher: Relauncher,
) -> Result<(RpcModule<()>, RunningNode)> {
    let runtime = RuntimeBuilder::new(
        RuntimeConfig::default().with_tokio(TokioConfig::ExistingHandle(Handle::current())),
    )
    .build()?;
    let Prepared { chain_spec, fork } = prepared;
    {
        // What a fork endpoint chose stays across relaunches, as on anvil: the network for good,
        // the chain id and hardfork until a reset to another endpoint.
        let mut launch_config = relauncher.config.write();
        launch_config.networks = config.networks;
        launch_config.adopted_fork_network = config.adopted_fork_network;
        // The dev wallets sign for the adopted chain id too.
        launch_config.set_chain_id(config.chain_id);
        launch_config.adopted_chain_id = config.adopted_chain_id;
        launch_config.hardfork = config.hardfork;
        launch_config.adopted_hardfork = config.adopted_hardfork;
    }

    let datadir = tempfile::tempdir()?;
    // The RPC servers run in front of the node, see `RpcServer`.
    let mut rpc_args = RpcServerArgs {
        http: false,
        ws: false,
        ipcdisable: true,
        disable_auth_server: true,
        ..Default::default()
    };
    if let Some(memory_limit) = config.memory_limit {
        rpc_args.rpc_evm_memory_limit = memory_limit;
    }
    // Calls and estimates without a gas limit get the block gas limit, as in anvil.
    rpc_args.rpc_gas_cap = config.get_gas_limit();
    // Proofs for any block, not only the latest.
    rpc_args.rpc_eth_proof_window = MAX_ETH_PROOF_WINDOW;
    let mut node_config = RethNodeConfig::new(chain_spec.clone())
        .with_storage(StorageArgs { v2: false })
        .with_rpc(rpc_args)
        .with_datadir_args(DatadirArgs {
            datadir: MaybePlatformPath::<DataDirPath>::from(datadir.path().to_path_buf()),
            ..Default::default()
        })
        // Pin the block gas limit, so blocks do not drift towards reth's default limit.
        .with_payload_builder(PayloadBuilderArgs {
            gas_limit: Some(config.get_gas_limit()),
            ..Default::default()
        });
    // A forkchoice update onto a canonical ancestor unwinds the chain to it, which snapshots,
    // rollbacks, reorgs, and resets rely on.
    node_config.engine = EngineArgs {
        always_process_payload_attributes_on_canonical_head: true,
        allow_unwind_canonical_header: true,
        ..Default::default()
    };
    // Reth reserves an 8 TiB map by default, which fails once a few dev nodes run side by side.
    // A dev node never approaches that size, so cap the map and grow it in smaller steps.
    let db_args = DatabaseArguments::new(ClientVersion::default())
        .with_geometry_max_size(Some(512 * GIGABYTE))
        .with_growth_step(Some(256 * MEGABYTE));
    let db = init_db(node_config.datadir().db(), db_args)?;

    let impersonation = ImpersonationState::default();
    impersonation.set_auto_impersonate(config.enable_auto_impersonate);
    // The dump names the senders of its impersonated transactions, which no signature recovers.
    if let Some(fork) = &fork {
        for (hash, sender) in fork.impersonated_transactions() {
            impersonation.remember_tx_sender(hash, sender);
        }
    }
    let (mining, miner_requests) = MiningController::new(initial_mining_mode(&config));
    let time = TimeManager::new(chain_spec.genesis().timestamp);
    let block_env = BlockEnvOverrides::default();
    if let Some(coinbase) = config.coinbase {
        block_env.set_coinbase(coinbase);
    }
    if config.disable_block_gas_limit {
        block_env.set_gas_limit(u64::MAX);
    } else if fork.is_some()
        && let Some(gas_limit) = config.gas_limit
        && gas_limit != chain_spec.genesis_header().gas_limit()
    {
        block_env.set_gas_limit(gas_limit);
    }
    block_env.set_max_transactions(Some(config.max_transactions));
    block_env.set_gas_price(config.get_gas_price());
    // Anvil gives the first block the genesis base fee, not the EIP-1559 decrease of an empty
    // parent. A fork starts from the fork block's fee.
    // A chain loaded from a dump continues the dump's fee timeline instead.
    // An explicit base fee also overrides the fee a fork block schedules, as on anvil.
    if fork.as_ref().is_some_and(|fork| !fork.is_dump())
        && !config.adopted_base_fee
        && chain_spec.genesis_header().base_fee_per_gas().is_some()
        && let Some(base_fee) = config.base_fee
    {
        block_env.set_next_base_fee(base_fee);
    } else if Net::FIRST_BLOCK_KEEPS_GENESIS_BASE_FEE
        && fork.is_none()
        && config.init_state.is_none()
        && let Some(base_fee) = chain_spec.genesis_header().base_fee_per_gas()
    {
        block_env.set_next_base_fee(base_fee);
    }
    let anvil_state = AnvilState::shared();
    // A dump at or below the fork block only overlays its accounts on the fork, as anvil does.
    if let Some(state) = &config.init_state
        && fork.as_ref().is_some_and(|fork| !fork.is_dump())
    {
        let mut writes = anvil_state.write();
        for (address, record) in &state.accounts {
            writes.set_nonce(*address, record.nonce);
            writes.set_balance(*address, record.balance);
            if !record.code.is_empty() {
                writes.set_code(*address, Bytecode::new_raw(record.code.clone()));
            }
            for (slot, value) in &record.storage {
                writes.set_storage_at(*address, *slot, (*value).into());
            }
        }
    }
    let snapshots = SnapshotManager::default();
    let logging = LoggingState::new(!config.silent);
    let rpc_module = Arc::new(Mutex::new(None));
    let launcher = AnvilNodeLauncher::new(
        runtime.clone(),
        node_config.datadir(),
        node_config.tree_config(),
        anvil_state.clone(),
        config.slots_in_an_epoch,
        fork.clone(),
    );
    let order = SharedTransactionOrder::new(config.transaction_order);
    let anvil = AnvilComponents {
        impersonation: impersonation.clone(),
        block_env: block_env.clone(),
        anvil_state: anvil_state.clone(),
        order: order.clone(),
        config: config.clone(),
        fork: fork.clone().map(|fork| fork as Arc<dyn ForkInfo>),
        console: config.print_logs.then(|| ConsolePrinter::new(logging.clone())),
        time: time.clone(),
    };

    let builder = NodeBuilder::new(node_config)
        .with_database(Arc::new(db))
        .with_types_and_provider::<Net::Node, AnvilProvider<AnvilTypes<Net::Node>>>()
        .with_components(Net::components(&anvil))
        .with_add_ons(Net::add_ons(&anvil, logging.clone()))
        .extend_rpc_modules({
            let mining = mining.clone();
            let time = time.clone();
            let block_env = block_env.clone();
            let anvil_state = anvil_state.clone();
            let snapshots = snapshots.clone();
            let chain_spec = chain_spec.clone();
            let signer_accounts = config.signer_accounts.clone();
            let tempo_fee_payer = config.tempo_fee_payer_address().and_then(|fee_payer| {
                config.signer_accounts.iter().find(|wallet| wallet.address() == fee_payer).cloned()
            });
            let rpc_module = rpc_module.clone();
            let fork = fork.clone();
            let logging = logging.clone();
            let transaction_order = config.transaction_order;
            let min_priority_fee_enforced = !config.disable_min_priority_fee;
            let identity = Net::identity(&config)?;
            let network_precompiles = network_precompiles(&config);
            move |ctx| {
                let eth_api = ctx.registry.eth_api().clone();
                // Anvil's filters report the blocks after the one they are installed on; reth's
                // first poll includes that block, so the install drains it. A log filter with a
                // `fromBlock` replays the logs from there, as on anvil.
                let new_filter = {
                    let filter = ctx.registry.eth_handlers().filter.clone();
                    NewFilterHook::new(move |log_filter| {
                        let filter = filter.clone();
                        Box::pin(async move {
                            let (id, drain) = match log_filter {
                                Some(log_filter) => {
                                    let drain = matches!(
                                        log_filter.block_option,
                                        FilterBlockOption::Range { from_block: None, .. }
                                    );
                                    (
                                        EthFilterApiServer::new_filter(&filter, log_filter).await?,
                                        drain,
                                    )
                                }
                                None => {
                                    (EthFilterApiServer::new_block_filter(&filter).await?, true)
                                }
                            };
                            if drain {
                                EthFilterApiServer::filter_changes(&filter, id.clone()).await?;
                            }
                            Ok(id)
                        })
                    })
                };
                {
                    let mut signers = eth_api.signers().write();
                    signers.push(Box::new(DevSigner::new(signer_accounts)));
                    signers.push(Box::new(ImpersonatedSigner::new(impersonation.clone())));
                }
                let fork_info = fork.map(|fork| fork as Arc<dyn ForkInfo>);
                let txpool_eth = eth_api.clone();
                // Tempo's pool keeps the reads it made at the tip; replaying the tip to it drops
                // them, so anvil state writes reach it before the next block.
                let pool_refresh = (identity.network == Some("tempo")).then(|| {
                    let pool = ctx.pool().clone();
                    let provider = ctx.provider().clone();
                    PoolRefresh::new(move || {
                        let Ok(Some(block)) = provider.best_block_number().and_then(|number| {
                            provider.recovered_block(number.into(), TransactionVariant::NoHash)
                        }) else {
                            return;
                        };
                        let info = pool.block_info();
                        pool.on_canonical_state_change(CanonicalStateUpdate {
                            new_tip: block.sealed_block(),
                            pending_block_base_fee: info.pending_basefee,
                            pending_block_blob_fee: info.pending_blob_fee,
                            changed_accounts: Vec::new(),
                            mined_transactions: Vec::new(),
                            update_kind: PoolUpdateKind::Commit,
                        });
                    })
                });
                let rpc = AnvilRpc::new(
                    identity,
                    relauncher,
                    impersonation,
                    mining,
                    time,
                    block_env,
                    anvil_state,
                    snapshots,
                    chain_spec,
                    instance_id,
                    logging,
                    transaction_order,
                    min_priority_fee_enforced,
                    fork_info.clone(),
                    ctx.pool().clone(),
                    ctx.provider().clone(),
                    eth_api,
                    new_filter,
                )
                .with_tempo_fee_payer(tempo_fee_payer)
                .with_pool_refresh(pool_refresh)
                .with_first_block_keeps_genesis_base_fee(Net::FIRST_BLOCK_KEEPS_GENESIS_BASE_FEE);
                let anvil_module = AnvilApiServer::into_rpc(rpc.clone());
                let evm_module = EvmApiServer::into_rpc(rpc.clone());
                let eth_module = EthExtApiServer::into_rpc(rpc.clone());
                let web3_module = Web3ExtApiServer::into_rpc(rpc.clone());
                let personal_module = PersonalApiServer::into_rpc(rpc);
                let txpool_module = AnvilTxPoolApiServer::into_rpc(AnvilTxPool::new(txpool_eth));
                let debug_module = AnvilDebugApiServer::into_rpc(AnvilDebugApi::new(
                    ctx.registry.debug_api(),
                    fork_info.clone(),
                ));
                let trace_module = AnvilTraceApiServer::into_rpc(AnvilTraceApi::new(
                    ctx.registry.trace_api(),
                    fork_info,
                ));

                // The in-process API calls the same handlers the servers do.
                let mut module = ctx.registry.module_for(&RpcModuleSelection::All);
                // Reth adds `eth_config` to its transport modules, which are off here. The
                // console precompile is the node's own, so the fork's list leaves it out, as
                // anvil's does.
                let mut config_module = RpcModule::new(EthConfigHandler::new(
                    ctx.provider().clone(),
                    FullNodeComponents::evm_config(ctx.node()).clone(),
                ));
                config_module.register_method("eth_config", move |_, handler, _| {
                    let mut config = EthConfigApiServer::config(handler)?;
                    // Tempo installs its precompiles per call, so the EVM lists none of them.
                    config.current.precompiles.extend(network_precompiles.clone());
                    for fork in
                        [Some(&mut config.current), config.next.as_mut(), config.last.as_mut()]
                            .into_iter()
                            .flatten()
                    {
                        fork.precompiles.retain(|_, address| *address != HARDHAT_CONSOLE_ADDRESS);
                    }
                    RpcResult::Ok(config)
                })?;
                module.merge(config_module)?;
                module.merge(anvil_module.clone())?;
                module.merge(evm_module.clone())?;
                for name in eth_module
                    .method_names()
                    .chain(web3_module.method_names())
                    .chain(debug_module.method_names())
                    .chain(trace_module.method_names())
                    .chain(txpool_module.method_names())
                {
                    module.remove_method(name);
                }
                module.merge(eth_module.clone())?;
                module.merge(web3_module.clone())?;
                module.merge(personal_module.clone())?;
                module.merge(debug_module.clone())?;
                module.merge(trace_module.clone())?;
                module.merge(txpool_module.clone())?;
                *rpc_module.lock().expect("rpc module lock") = Some(module);

                ctx.modules.merge_configured(anvil_module)?;
                ctx.modules.merge_configured(evm_module)?;
                ctx.modules.replace_configured(eth_module)?;
                ctx.modules.replace_configured(web3_module)?;
                ctx.modules.merge_configured(personal_module)?;
                ctx.modules.replace_configured(debug_module)?;
                ctx.modules.replace_configured(trace_module)?;
                ctx.modules.replace_configured(txpool_module)?;
                Ok(())
            }
        });
    let RethNodeHandle { node, node_exit_future } = launcher.launch_node(builder).await?;

    let head = node
        .provider
        .sealed_header(node.provider.best_block_number()?)?
        .ok_or_else(|| eyre::eyre!("missing head header"))?;
    let insert_provider = node.provider.clone();
    let active_forks = {
        let chain_spec = chain_spec.clone();
        move |timestamp| {
            (
                chain_spec.is_shanghai_active_at_timestamp(timestamp),
                chain_spec.is_cancun_active_at_timestamp(timestamp),
            )
        }
    };
    let (map_attributes, finish) =
        time.build_hooks(block_env.clone(), chain_spec.genesis().coinbase, active_forks);
    let genesis_hash = node.provider.genesis_header()?.hash();
    let automine = {
        let mining = mining.clone();
        move || mining.is_automine()
    };
    let pending_after = {
        let pool = node.pool.clone();
        move |head| {
            Box::pin(pool_pending_after(pool.clone(), head)) as HookFuture<Option<PendingTxs>>
        }
    };
    let miner = AnvilMiner::<<Net::Node as NodeTypes>::Payload>::new(
        node.add_ons_handle.beacon_engine_handle.clone(),
        node.payload_builder_handle.clone(),
        Net::payload_attributes(chain_spec),
        map_attributes,
        finish,
        node.provider.clone(),
        move || Ok(insert_provider.materialize_fork_reads()?),
        automine,
        pending_after,
        genesis_hash,
        head,
        miner_requests,
    );
    node.task_executor.spawn_critical_task("reth-anvil miner", miner.run());
    node.task_executor.spawn_critical_task(
        "reth-anvil automine",
        run_automine_task(node.pool.clone(), mining.clone()),
    );
    node.task_executor.spawn_critical_task(
        "reth-anvil interval mining",
        run_interval_mining_task(mining.clone()),
    );
    node.task_executor.spawn_critical_task(
        "reth-anvil state writes",
        clear_applied_state_writes(node.provider.subscribe_to_canonical_state(), anvil_state),
    );
    node.task_executor.spawn_critical_task(
        "reth-anvil logging",
        log_mined_blocks(node.provider.subscribe_to_canonical_state(), logging),
    );
    if let Some(fork) = &fork
        && let Some(replay) = fork.take_replay()
        && let Err(error) =
            replay_fork_transactions(replay, &node.pool, &mining, &time, &block_env, &order).await
    {
        // The cached remote state must not outlive a failed start, as on anvil.
        fork.remove_cache();
        return Err(error.wrap_err("failed to replay fork transaction prefix"));
    }

    let module = rpc_module
        .lock()
        .expect("rpc module lock")
        .take()
        .ok_or_else(|| eyre::eyre!("the rpc modules were not built"))?;

    Ok((module, RunningNode { node_exit_future, _datadir: datadir, runtime }))
}

/// Mines the transactions of a fork at a transaction hash into the first local block, with the
/// block environment of the remote block they came from, so the node starts right after the
/// fork transaction.
async fn replay_fork_transactions<F, Pool>(
    replay: ForkReplay<F>,
    pool: &Pool,
    mining: &MiningController<ForkHeader<F>>,
    time: &TimeManager,
    block_env: &BlockEnvOverrides,
    order: &SharedTransactionOrder,
) -> Result<()>
where
    F: ForkNetwork,
    Pool: TransactionPool<
        Transaction: PoolTransaction<Consensus = <F::Primitives as NodePrimitives>::SignedTx>,
    >,
{
    let ForkReplay { header, transactions } = replay;
    let snapshot = block_env.snapshot();
    let previous_order = order.get();
    // The block keeps the remote order, the remote block environment, and every transaction.
    order.set(TransactionOrder::Fifo);
    block_env.set_coinbase(header.beneficiary());
    block_env.set_gas_limit(header.gas_limit());
    block_env.set_max_transactions(None);
    if let Some(base_fee) = header.base_fee_per_gas() {
        block_env.set_next_base_fee(base_fee);
    }
    if let Some(prev_randao) = header.mix_hash() {
        block_env.set_next_prev_randao(prev_randao);
    }
    if let Some(root) = header.parent_beacon_block_root() {
        block_env.set_next_parent_beacon_block_root(root);
    }
    // The replayed block keeps the source timestamp without moving the clock, so the next block
    // follows it, as on anvil.
    time.pin_next_timestamp(header.timestamp());

    let expected = transactions.len();
    let mut submitted = 0;
    for tx in transactions {
        let hash = *tx.tx_hash();
        let Ok(sender) = tx.try_recover() else {
            tracing::warn!(target: "node", %hash, "skipping fork transaction: sender not recoverable");
            continue;
        };
        let Ok(pooled) =
            Pool::Transaction::try_from_consensus(Recovered::new_unchecked(tx, sender))
        else {
            tracing::warn!(target: "node", %hash, "skipping fork transaction: not a pool transaction");
            continue;
        };
        match pool.add_transaction(TransactionOrigin::Local, pooled).await {
            Ok(_) => submitted += 1,
            Err(error) => {
                order.set(previous_order);
                block_env.restore(snapshot);
                eyre::bail!("the pool rejected fork transaction {hash}: {error}");
            }
        }
    }
    let mined = mining.mine_block().await.map_err(|error| eyre::eyre!(error));
    order.set(previous_order);
    block_env.restore(snapshot);
    let mined = mined?;
    tracing::info!(
        target: "node",
        block = mined.number(),
        replayed = submitted,
        skipped = expected - submitted,
        "replayed the fork transactions"
    );
    Ok(())
}

/// Returns the configured port, or a free port when the config asks for port zero.
fn rpc_port(port: u16) -> Result<u16> {
    if port != 0 {
        return Ok(port);
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).wrap_err("failed to pick a free port")?;
    Ok(listener.local_addr()?.port())
}

const fn initial_mining_mode(config: &NodeConfig) -> MiningMode {
    match (config.no_mining, config.block_time, config.mixed_mining) {
        (true, _, _) => MiningMode::Manual,
        (false, Some(block_time), true) => MiningMode::Mixed(block_time),
        (false, Some(block_time), false) => MiningMode::Interval(block_time),
        (false, None, _) => MiningMode::Automine,
    }
}

/// Drops the read overlay for state writes once the block that applied them is canonical.
async fn clear_applied_state_writes<N: NodePrimitives>(
    mut notifications: CanonStateNotifications<N>,
    state: SharedAnvilState,
) {
    loop {
        match notifications.recv().await {
            Ok(notification) => {
                let committed = notification.committed();
                if !committed.is_empty() {
                    state.write().on_canonical_block(committed.tip().number());
                }
            }
            Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return,
        }
    }
}

/// Returns the precompiles of the network that the EVM does not list: Tempo's, which Tempo
/// installs per call.
#[cfg_attr(not(feature = "tempo"), expect(clippy::missing_const_for_fn))]
fn network_precompiles(config: &NodeConfig) -> BTreeMap<String, Address> {
    #[cfg(feature = "tempo")]
    if config.networks.is_tempo() {
        let hardfork = config.get_tempo_hardfork().ok().map(Into::into);
        return config.networks.precompiles(hardfork);
    }
    let _ = config;
    BTreeMap::new()
}
