use crate::{
    api::{AnvilApiServer, AnvilRpc},
    block_env::BlockEnvOverrides,
    evm::AnvilExecutorBuilder,
    impersonation::{ImpersonatedSigner, ImpersonationState},
    mining::{MiningController, run_automine_task, run_interval_mining_task},
    pool::AnvilPoolBuilder,
    time::TimeManager,
};
use alloy_primitives::B256;
use eyre::Result;
use reth_ethereum::{
    chainspec::{ChainSpec, DEV},
    engine::local::MiningMode,
    node::{
        EthereumNode,
        builder::{
            NodeBuilder, NodeHandle,
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
    provider::db::{ClientVersion, init_db, mdbx::DatabaseArguments},
    tasks::Runtime,
};
use std::sync::Arc;
use tempfile::TempDir;

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
    let db = init_db(node_config.datadir().db(), DatabaseArguments::new(ClientVersion::default()))?;

    let impersonation = ImpersonationState::default();
    let mining = MiningController::default();
    let time = TimeManager::new(chain_spec.genesis_timestamp());
    let block_env = BlockEnvOverrides::default();
    let trigger_stream = mining.trigger_stream();

    let NodeHandle { node, node_exit_future } = NodeBuilder::new(node_config)
        .with_database(Arc::new(db))
        .with_launch_context(runtime)
        .with_types::<EthereumNode>()
        .with_components(
            EthereumNode::components()
                .network(NoopNetworkBuilder::eth())
                .pool(AnvilPoolBuilder { state: impersonation.clone() })
                .executor(AnvilExecutorBuilder {
                    state: impersonation.clone(),
                    block_env: block_env.clone(),
                })
                .consensus(NoopConsensusBuilder),
        )
        .with_add_ons(EthereumAddOns::default())
        .extend_rpc_modules({
            let mining = mining.clone();
            let time = time.clone();
            let block_env = block_env.clone();
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
                    chain_spec,
                    B256::random(),
                    ctx.pool().clone(),
                    ctx.provider().clone(),
                    eth_api,
                );
                ctx.modules.merge_configured(rpc.into_rpc())?;
                Ok(())
            }
        })
        .launch_with_debug_capabilities()
        .map_debug_payload_attributes(time.payload_attributes_hook(block_env))
        .with_mining_mode(MiningMode::trigger(trigger_stream))
        .await?;

    node.task_executor.spawn_critical_task(
        "reth-anvil automine",
        run_automine_task(node.pool.clone(), mining.clone()),
    );
    node.task_executor
        .spawn_critical_task("reth-anvil interval mining", run_interval_mining_task(mining));

    Ok(RethAnvilHandle {
        rpc_server_handles: node.rpc_server_handles.clone(),
        node_exit_future,
        _datadir: datadir,
    })
}
