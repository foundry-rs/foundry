use crate::{
    api::{AnvilApiServer, AnvilRpc, EthExtApiServer, EvmApiServer, PersonalApiServer},
    block_env::BlockEnvOverrides,
    config::NodeConfig,
    eth_api::EthApi,
    evm::AnvilExecutorBuilder,
    fork::ForkBackend,
    impersonation::{ImpersonatedSigner, ImpersonationState},
    launcher::AnvilNodeLauncher,
    logging::{LoggingState, NodeInfoLayer, log_mined_blocks},
    miner::AnvilMiner,
    mining::{MiningController, MiningMode, run_automine_task, run_interval_mining_task},
    pool::AnvilPoolBuilder,
    provider::AnvilProvider,
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
use reth_ethereum::{
    engine::local::LocalPayloadAttributesBuilder,
    node::{
        EthereumNode,
        api::NodeTypesWithDBAdapter,
        builder::{
            LaunchNode, NodeBuilder, NodeHandle as RethNodeHandle,
            components::{NoopConsensusBuilder, NoopNetworkBuilder},
            rpc::RethRpcServerHandles,
        },
        core::{
            args::{DatadirArgs, RpcServerArgs, StorageArgs},
            dirs::{DataDirPath, MaybePlatformPath},
            exit::NodeExitFuture,
            node_config::NodeConfig as RethNodeConfig,
        },
        node::EthereumAddOns,
    },
    provider::{
        CanonStateNotifications, CanonStateSubscriptions, HeaderProvider,
        db::{
            ClientVersion, DatabaseEnv, init_db,
            mdbx::{DatabaseArguments, GIGABYTE, MEGABYTE},
        },
    },
    rpc::builder::RpcModuleSelection,
    storage::BlockNumReader,
    tasks::{Runtime, RuntimeBuilder, RuntimeConfig, TokioConfig},
};
use std::{
    net::{SocketAddr, TcpListener},
    sync::{Arc, Mutex},
};
use tempfile::TempDir;
use tokio::{runtime::Handle, sync::broadcast::error::RecvError};

/// The reth node types used by reth-anvil.
type AnvilNodeTypes = NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>;

/// A running node.
#[derive(Debug)]
pub struct NodeHandle {
    config: NodeConfig,
    address: SocketAddr,
    /// The RPC server handles.
    pub rpc_server_handles: RethRpcServerHandles,
    /// Resolves when the node exits.
    pub node_exit_future: NodeExitFuture,
    _datadir: TempDir,
    _runtime: Runtime,
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
pub async fn try_spawn(mut config: NodeConfig) -> Result<(EthApi, NodeHandle)> {
    let runtime = RuntimeBuilder::new(
        RuntimeConfig::default().with_tokio(TokioConfig::ExistingHandle(Handle::current())),
    )
    .build()?;
    let (fork, chain_spec) = if config.is_fork() {
        let (fork, accounts) = ForkBackend::setup(&config).await?;
        config.apply_fork(fork.chain_id(), fork.header(), fork.gas_price());
        let chain_spec = config.fork_chain_spec(fork.header(), &accounts)?;
        (Some(fork), chain_spec)
    } else {
        (None, config.chain_spec()?)
    };
    let address = SocketAddr::new(config.host[0], rpc_port(config.port)?);

    let datadir = tempfile::tempdir()?;
    let node_config = RethNodeConfig::new(chain_spec.clone())
        .with_storage(StorageArgs { v2: false })
        .with_rpc(RpcServerArgs {
            http: true,
            http_addr: address.ip(),
            http_port: address.port(),
            http_corsdomain: Some("*".to_string()),
            http_api: Some(RpcModuleSelection::All),
            ws: true,
            ws_addr: address.ip(),
            ws_port: address.port(),
            ws_allowed_origins: Some("*".to_string()),
            ws_api: Some(RpcModuleSelection::All),
            ipcdisable: config.ipc_path.is_none(),
            ipcpath: config.ipc_path.clone().unwrap_or_default(),
            disable_auth_server: true,
            ..Default::default()
        })
        .with_datadir_args(DatadirArgs {
            datadir: MaybePlatformPath::<DataDirPath>::from(datadir.path().to_path_buf()),
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
    let time = TimeManager::new(chain_spec.genesis_timestamp());
    let block_env = BlockEnvOverrides::default();
    if fork.is_some()
        && let Some(gas_limit) = config.gas_limit
        && gas_limit != chain_spec.genesis_header().gas_limit
    {
        block_env.set_gas_limit(gas_limit);
    }
    let anvil_state = AnvilState::shared();
    let snapshots = SnapshotManager::default();
    let logging = LoggingState::new(!config.silent);
    let instance_id = B256::random();
    let rpc_module = Arc::new(Mutex::new(None));
    let launcher = AnvilNodeLauncher::new(
        runtime.clone(),
        node_config.datadir(),
        node_config.tree_config(),
        anvil_state.clone(),
        config.slots_in_an_epoch,
        fork.clone(),
    );

    let builder = NodeBuilder::new(node_config)
        .with_database(Arc::new(db))
        .with_types_and_provider::<EthereumNode, AnvilProvider<AnvilNodeTypes>>()
        .with_components(
            EthereumNode::components()
                .network(NoopNetworkBuilder::eth())
                .pool(AnvilPoolBuilder {
                    state: impersonation.clone(),
                    order: config.transaction_order,
                })
                .executor(AnvilExecutorBuilder {
                    state: impersonation.clone(),
                    block_env: block_env.clone(),
                    anvil_state: anvil_state.clone(),
                })
                .consensus(NoopConsensusBuilder),
        )
        .with_add_ons(
            EthereumAddOns::default().with_rpc_middleware(NodeInfoLayer::new(logging.clone())),
        )
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
            move |ctx| {
                let eth_api = ctx.registry.eth_api().clone();
                {
                    let mut signers = eth_api.signers().write();
                    signers.push(Box::new(DevSigner::new(signer_accounts)));
                    signers.push(Box::new(ImpersonatedSigner::new(impersonation.clone())));
                }
                let rpc = AnvilRpc::new(
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
                    fork,
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
                module.merge(anvil_module.clone())?;
                module.merge(evm_module.clone())?;
                module.merge(eth_module.clone())?;
                module.merge(personal_module.clone())?;
                *rpc_module.lock().expect("rpc module lock") = Some(module);

                ctx.modules.merge_configured(anvil_module)?;
                ctx.modules.merge_configured(evm_module)?;
                ctx.modules.merge_configured(eth_module)?;
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
    let miner = AnvilMiner::new(
        node.add_ons_handle.beacon_engine_handle.clone(),
        node.payload_builder_handle.clone(),
        LocalPayloadAttributesBuilder::new(chain_spec),
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
    let address = node
        .add_ons_handle
        .rpc_server_handles
        .rpc
        .http_local_addr()
        .ok_or_else(|| eyre::eyre!("the http server did not start"))?;

    Ok((
        EthApi::new(module, instance_id),
        NodeHandle {
            config,
            address,
            rpc_server_handles: node.add_ons_handle.rpc_server_handles,
            node_exit_future,
            _datadir: datadir,
            _runtime: runtime,
        },
    ))
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
async fn clear_applied_state_writes(
    mut notifications: CanonStateNotifications,
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
