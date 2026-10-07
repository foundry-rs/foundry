use crate::{
    api::{AnvilApiServer, AnvilRpc, EthExtApiServer, EvmApiServer, PersonalApiServer},
    block_env::BlockEnvOverrides,
    config::NodeConfig,
    eth_api::EthApi,
    fork::ForkInfo,
    impersonation::{ImpersonatedSigner, ImpersonationState},
    launcher::AnvilNodeLauncher,
    logging::{LoggingState, log_mined_blocks},
    miner::AnvilMiner,
    mining::{MiningController, MiningMode, run_automine_task, run_interval_mining_task},
    network::{AnvilComponents, AnvilNetwork, AnvilTypes, Prepared, ethereum::Ethereum},
    provider::AnvilProvider,
    server::{RpcServer, ServerSettings, SharedModule},
    signer::DevSigner,
    snapshot::SnapshotManager,
    state::{AnvilState, SharedAnvilState},
    time::TimeManager,
};
use alloy_consensus::BlockHeader;
use alloy_primitives::{Address, B256, U256};
use alloy_signer_local::PrivateKeySigner;
use eyre::{Result, WrapErr};
use foundry_common::provider::{ProviderBuilder, RetryProvider};
use foundry_evm_networks::NetworkVariant;
use jsonrpsee::RpcModule;
use parking_lot::RwLock;
use reth_ethereum::{
    chainspec::EthChainSpec,
    node::{
        api::{FullNodeComponents, NodeTypes},
        builder::{LaunchNode, NodeBuilder, NodeHandle as RethNodeHandle},
        core::{
            args::{DatadirArgs, PayloadBuilderArgs, RpcServerArgs, StorageArgs},
            dirs::{DataDirPath, MaybePlatformPath},
            exit::NodeExitFuture,
            node_config::NodeConfig as RethNodeConfig,
        },
    },
    primitives::NodePrimitives,
    provider::{
        CanonStateNotifications, CanonStateSubscriptions, HeaderProvider,
        db::{
            ClientVersion, init_db,
            mdbx::{DatabaseArguments, GIGABYTE, MEGABYTE},
        },
    },
    rpc::builder::RpcModuleSelection,
    storage::BlockNumReader,
    tasks::{Runtime, RuntimeBuilder, RuntimeConfig, TokioConfig},
};
use reth_rpc_eth_api::helpers::{
    EthTransactions,
    config::{EthConfigApiServer, EthConfigHandler},
};
use std::{
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
    /// Stops the node when the handle drops.
    _shutdown: oneshot::Sender<()>,
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
}

impl Relauncher {
    /// Replaces the running node with one launched from the current config changed by `update`.
    /// Returns once the new node serves requests.
    pub async fn relaunch(&self, update: impl FnOnce(&mut NodeConfig)) -> Result<(), String> {
        let mut config = self.config.read().clone();
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

    /// Returns a provider for the WebSocket endpoint.
    pub fn ws_provider(&self) -> RetryProvider {
        ProviderBuilder::new(&self.ws_endpoint()).build().expect("failed to build WS provider")
    }

    /// Returns the accounts the node signs for.
    pub fn dev_accounts(&self) -> impl Iterator<Item = Address> + '_ {
        self.config.signer_accounts.iter().map(|wallet| wallet.address())
    }

    /// Returns the wallets the node signs with.
    pub fn dev_wallets(&self) -> impl Iterator<Item = PrivateKeySigner> + '_ {
        self.config.signer_accounts.iter().cloned()
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
/// returned handle drops. The node tasks run on the current tokio runtime.
pub async fn try_spawn(config: NodeConfig) -> Result<(EthApi, NodeHandle)> {
    match config.networks.resolved_network().unwrap_or_default() {
        NetworkVariant::Ethereum => launch::<Ethereum>(config).await,
        #[cfg(feature = "monad")]
        NetworkVariant::Monad => launch::<crate::network::monad::Monad>(config).await,
        network => eyre::bail!("the {network:?} network is not supported yet"),
    }
}

/// Launches a node of the given network and the RPC server in front of it.
pub(crate) async fn launch<Net: AnvilNetwork>(config: NodeConfig) -> Result<(EthApi, NodeHandle)> {
    let address = SocketAddr::new(config.host[0], rpc_port(config.port)?);
    let instance_id = Arc::new(RwLock::new(B256::random()));
    let (requests, relaunches) = mpsc::unbounded_channel();
    let relauncher = Relauncher { requests, config: Arc::new(RwLock::new(config.clone())) };
    let (module, running) =
        launch_node::<Net>(config.clone(), instance_id.clone(), relauncher.clone()).await?;
    let module: SharedModule = Arc::new(RwLock::new(module));
    let logging = LoggingState::new(!config.silent);
    let server =
        RpcServer::start(address, ServerSettings::from_config(&config), module.clone(), logging)
            .await?;
    let address = server.address();

    let (exit_tx, exit_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    tokio::spawn(supervise::<Net>(Supervisor {
        running: Some(running),
        server: Some(server),
        module: module.clone(),
        instance_id: instance_id.clone(),
        relauncher,
        relaunches,
        exit: Some(exit_tx),
        shutdown: shutdown_rx,
    }));

    Ok((
        EthApi::new(module, instance_id),
        NodeHandle { config, address, node_exit_future: NodeExit(exit_rx), _shutdown: shutdown_tx },
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
        }
    }
    if let Some(server) = supervisor.server.take() {
        server.stop();
    }
    if let Some(running) = supervisor.running.take() {
        running.stop().await;
    }
}

/// Stops the running node and launches one from `config`. If that fails, the previous config is
/// launched again, so the node keeps serving.
async fn relaunch<Net: AnvilNetwork>(
    supervisor: &mut Supervisor,
    config: NodeConfig,
) -> Result<()> {
    if let Some(running) = supervisor.running.take() {
        running.stop().await;
    }
    let launch = |config: NodeConfig| {
        launch_node::<Net>(config, supervisor.instance_id.clone(), supervisor.relauncher.clone())
    };
    let (result, config) = match launch(config.clone()).await {
        Ok(launched) => (Ok(launched), config),
        Err(error) => {
            let previous = supervisor.relauncher.config.read().clone();
            match launch(previous.clone()).await {
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
    mut config: NodeConfig,
    instance_id: Arc<RwLock<B256>>,
    relauncher: Relauncher,
) -> Result<(RpcModule<()>, RunningNode)> {
    let runtime = RuntimeBuilder::new(
        RuntimeConfig::default().with_tokio(TokioConfig::ExistingHandle(Handle::current())),
    )
    .build()?;
    let Prepared { chain_spec, fork } = Net::prepare(&mut config).await?;

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
    let node_config = RethNodeConfig::new(chain_spec.clone())
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
    // Reth reserves an 8 TiB map by default, which fails once a few dev nodes run side by side.
    // A dev node never approaches that size, so cap the map and grow it in smaller steps.
    let db_args = DatabaseArguments::new(ClientVersion::default())
        .with_geometry_max_size(Some(512 * GIGABYTE))
        .with_growth_step(Some(256 * MEGABYTE));
    let db = init_db(node_config.datadir().db(), db_args)?;

    let impersonation = ImpersonationState::default();
    impersonation.set_auto_impersonate(config.enable_auto_impersonate);
    let (mining, miner_requests) = MiningController::new(initial_mining_mode(&config));
    let time = TimeManager::new(chain_spec.genesis().timestamp);
    let block_env = BlockEnvOverrides::default();
    if config.disable_block_gas_limit {
        block_env.set_gas_limit(u64::MAX);
    } else if fork.is_some()
        && let Some(gas_limit) = config.gas_limit
        && gas_limit != chain_spec.genesis_header().gas_limit()
    {
        block_env.set_gas_limit(gas_limit);
    }
    let anvil_state = AnvilState::shared();
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
    let anvil = AnvilComponents {
        impersonation: impersonation.clone(),
        block_env: block_env.clone(),
        anvil_state: anvil_state.clone(),
        config: config.clone(),
    };

    let builder = NodeBuilder::new(node_config)
        .with_database(Arc::new(db))
        .with_types_and_provider::<Net::Node, AnvilProvider<AnvilTypes<Net::Node>>>()
        .with_components(Net::components(&anvil))
        .with_add_ons(Net::add_ons(logging.clone()))
        .extend_rpc_modules({
            let mining = mining.clone();
            let time = time.clone();
            let block_env = block_env.clone();
            let anvil_state = anvil_state.clone();
            let snapshots = snapshots.clone();
            let chain_spec = chain_spec.clone();
            let signer_accounts = config.signer_accounts.clone();
            let rpc_module = rpc_module.clone();
            let fork = fork.clone();
            let logging = logging.clone();
            let transaction_order = config.transaction_order;
            let identity = Net::identity(&config)?;
            move |ctx| {
                let eth_api = ctx.registry.eth_api().clone();
                {
                    let mut signers = eth_api.signers().write();
                    signers.push(Box::new(DevSigner::new(signer_accounts)));
                    signers.push(Box::new(ImpersonatedSigner::new(impersonation.clone())));
                }
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
                    fork.map(|fork| fork as Arc<dyn ForkInfo>),
                    ctx.pool().clone(),
                    ctx.provider().clone(),
                    eth_api,
                );
                let anvil_module = AnvilApiServer::into_rpc(rpc.clone());
                let evm_module = EvmApiServer::into_rpc(rpc.clone());
                let eth_module = EthExtApiServer::into_rpc(rpc.clone());
                let personal_module = PersonalApiServer::into_rpc(rpc);

                // The in-process API calls the same handlers the servers do.
                let mut module = ctx.registry.module_for(&RpcModuleSelection::All);
                // Reth adds `eth_config` to its transport modules, which are off here.
                module.merge(EthConfigApiServer::into_rpc(EthConfigHandler::new(
                    ctx.provider().clone(),
                    ctx.node().evm_config().clone(),
                )))?;
                module.merge(anvil_module.clone())?;
                module.merge(evm_module.clone())?;
                for name in eth_module.method_names() {
                    module.remove_method(name);
                }
                module.merge(eth_module.clone())?;
                module.merge(personal_module.clone())?;
                *rpc_module.lock().expect("rpc module lock") = Some(module);

                ctx.modules.merge_configured(anvil_module)?;
                ctx.modules.merge_configured(evm_module)?;
                ctx.modules.replace_configured(eth_module)?;
                ctx.modules.merge_configured(personal_module)?;
                Ok(())
            }
        });
    let RethNodeHandle { node, node_exit_future } = launcher.launch_node(builder).await?;

    let head = node
        .provider
        .sealed_header(node.provider.best_block_number()?)?
        .ok_or_else(|| eyre::eyre!("missing head header"))?;
    let rewind_provider = node.provider.clone();
    let insert_provider = node.provider.clone();
    let miner = AnvilMiner::<<Net::Node as NodeTypes>::Payload>::new(
        node.add_ons_handle.beacon_engine_handle.clone(),
        node.payload_builder_handle.clone(),
        Net::payload_attributes(chain_spec),
        time.payload_attributes_hook(block_env),
        move |header| Ok(rewind_provider.rewind_to(header)?),
        move || Ok(insert_provider.materialize_fork_reads()?),
        head,
        miner_requests,
    );
    node.task_executor.spawn_critical_task("reth-anvil miner", miner.run());
    node.task_executor.spawn_critical_task(
        "reth-anvil automine",
        run_automine_task(node.pool.clone(), mining.clone()),
    );
    node.task_executor
        .spawn_critical_task("reth-anvil interval mining", run_interval_mining_task(mining));
    node.task_executor.spawn_critical_task(
        "reth-anvil state writes",
        clear_applied_state_writes(node.provider.subscribe_to_canonical_state(), anvil_state),
    );
    node.task_executor.spawn_critical_task(
        "reth-anvil logging",
        log_mined_blocks(node.provider.subscribe_to_canonical_state(), logging),
    );

    let module = rpc_module
        .lock()
        .expect("rpc module lock")
        .take()
        .ok_or_else(|| eyre::eyre!("the rpc modules were not built"))?;

    Ok((module, RunningNode { node_exit_future, _datadir: datadir, runtime }))
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
