use crate::{
    api::{AnvilApiServer, AnvilRpc},
    block_env::BlockEnvOverrides,
    evm::AnvilExecutorBuilder,
    impersonation::{ImpersonatedSigner, ImpersonationState},
    launcher::AnvilNodeLauncher,
    mining::{MiningController, run_automine_task, run_interval_mining_task},
    pool::AnvilPoolBuilder,
    provider::AnvilProvider,
    state::{AnvilState, SharedAnvilState},
    time::TimeManager,
};
use alloy_consensus::BlockHeader;
use alloy_primitives::B256;
use eyre::Result;
use reth_ethereum::{
    chainspec::{ChainSpec, DEV},
    engine::local::MiningMode,
    node::{
        EthereumNode,
        api::NodeTypesWithDBAdapter,
        builder::{
            DebugNodeLauncher, LaunchNode, NodeBuilder, NodeHandle,
            components::{NoopConsensusBuilder, NoopNetworkBuilder},
            rpc::RethRpcServerHandles,
        },
        core::{
            args::{DatadirArgs, RpcServerArgs, StorageArgs},
            dirs::{DataDirPath, MaybePlatformPath},
            exit::NodeExitFuture,
            node_config::NodeConfig,
        },
        node::EthereumAddOns,
    },
    provider::{
        CanonStateNotifications, CanonStateSubscriptions,
        db::{
            ClientVersion, DatabaseEnv, init_db,
            mdbx::{DatabaseArguments, GIGABYTE, MEGABYTE},
        },
    },
    tasks::Runtime,
};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::broadcast::error::RecvError;

/// The reth node types used by reth-anvil.
type AnvilNodeTypes = NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>;

/// Launch options for a reth-anvil node.
#[derive(Clone, Debug)]
pub struct RethAnvilConfig {
    /// The chain to run.
    pub chain_spec: Arc<ChainSpec>,
    /// The RPC server settings.
    pub rpc: RpcServerArgs,
}

impl Default for RethAnvilConfig {
    fn default() -> Self {
        Self { chain_spec: DEV.clone(), rpc: RpcServerArgs::default().with_http() }
    }
}

/// A running reth-anvil node.
#[derive(Debug)]
pub struct RethAnvilHandle {
    /// The RPC server handles.
    pub rpc_server_handles: RethRpcServerHandles,
    /// Resolves when the node exits.
    pub node_exit_future: NodeExitFuture,
    /// Owns the temporary data directory for the lifetime of the node.
    _datadir: TempDir,
}

/// Launches a reth dev node with the `anvil_*` namespace and anvil's mining, time, and
/// impersonation controls.
///
/// The node runs on a fresh MDBX database in a temporary directory that is removed when the
/// returned handle drops.
pub async fn launch(config: RethAnvilConfig, runtime: Runtime) -> Result<RethAnvilHandle> {
    let RethAnvilConfig { chain_spec, rpc } = config;
    let datadir = tempfile::tempdir()?;
    let node_config = NodeConfig::new(chain_spec.clone())
        .dev()
        .with_storage(StorageArgs { v2: false })
        .with_rpc(rpc)
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
    let mining = MiningController::default();
    let time = TimeManager::new(chain_spec.genesis_timestamp());
    let block_env = BlockEnvOverrides::default();
    let anvil_state = AnvilState::shared();
    let trigger_stream = mining.trigger_stream();
    let launcher = DebugNodeLauncher::new(AnvilNodeLauncher::new(
        runtime,
        node_config.datadir(),
        node_config.tree_config(),
        anvil_state.clone(),
    ));

    let builder = NodeBuilder::new(node_config)
        .with_database(Arc::new(db))
        .with_types_and_provider::<EthereumNode, AnvilProvider<AnvilNodeTypes>>()
        .with_components(
            EthereumNode::components()
                .network(NoopNetworkBuilder::eth())
                .pool(AnvilPoolBuilder { state: impersonation.clone() })
                .executor(AnvilExecutorBuilder {
                    state: impersonation.clone(),
                    block_env: block_env.clone(),
                    anvil_state: anvil_state.clone(),
                })
                .consensus(NoopConsensusBuilder),
        )
        .with_add_ons(EthereumAddOns::default())
        .extend_rpc_modules({
            let mining = mining.clone();
            let time = time.clone();
            let block_env = block_env.clone();
            let anvil_state = anvil_state.clone();
            move |ctx| {
                let eth_api = ctx.registry.eth_api().clone();
                eth_api
                    .signers()
                    .write()
                    .push(Box::new(ImpersonatedSigner::new(impersonation.clone())));
                let rpc = AnvilRpc::new(
                    impersonation,
                    mining,
                    time,
                    block_env,
                    anvil_state,
                    chain_spec,
                    B256::random(),
                    ctx.pool().clone(),
                    ctx.provider().clone(),
                    eth_api,
                );
                ctx.modules.merge_configured(rpc.into_rpc())?;
                Ok(())
            }
        });
    let NodeHandle { node, node_exit_future } = launcher
        .launch_node(builder)
        .map_debug_payload_attributes(time.payload_attributes_hook(block_env))
        .with_mining_mode(MiningMode::trigger(trigger_stream))
        .await?;

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

    Ok(RethAnvilHandle {
        rpc_server_handles: node.rpc_server_handles.clone(),
        node_exit_future,
        _datadir: datadir,
    })
}

/// Drops the read overlay for state writes once the block that applied them is canonical.
async fn clear_applied_state_writes(
    mut notifications: CanonStateNotifications,
    state: SharedAnvilState,
) {
    loop {
        match notifications.recv().await {
            Ok(notification) => {
                let tip = notification.committed().tip().number();
                state.write().on_canonical_block(tip);
            }
            Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return,
        }
    }
}
