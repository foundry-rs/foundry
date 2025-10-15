use crate::substrate_node::service::{Backend, Client, TransactionPoolHandle};
use jsonrpsee::RpcModule;
use polkadot_sdk::{
    parachains_common::opaque::Block,
    sc_chain_spec::ChainSpec,
    sc_client_api::{Backend as ClientBackend, HeaderBackend},
    sc_client_db::{BlocksPruning, PruningMode},
    sc_network_types::{self, multiaddr::Multiaddr},
    sc_rpc::{
        author::AuthorApiServer,
        chain::ChainApiServer,
        offchain::OffchainApiServer,
        state::{ChildStateApiServer, StateApiServer},
        system::{Request, SystemApiServer, SystemInfo},
    },
    sc_rpc_api::DenyUnsafe,
    sc_rpc_spec_v2::{
        archive::ArchiveApiServer,
        chain_head::ChainHeadApiServer,
        chain_spec::ChainSpecApiServer,
        transaction::{TransactionApiServer, TransactionBroadcastApiServer},
    },
    sc_service::{
        self, Configuration, RpcHandlers, SpawnTaskHandle, TaskManager,
        error::Error as ServiceError,
    },
    sc_utils::mpsc::{TracingUnboundedSender, tracing_unbounded},
    sp_keystore::KeystorePtr,
    substrate_frame_rpc_system::SystemApiServer as _,
};
use std::sync::Arc;

pub fn spawn_rpc_server(
    genesis_number: u64,
    task_manager: &mut TaskManager,
    client: Arc<Client>,
    mut config: Configuration,
    transaction_pool: Arc<TransactionPoolHandle>,
    keystore: KeystorePtr,
    backend: Arc<Backend>,
) -> Result<RpcHandlers, ServiceError> {
    let (system_rpc_tx, system_rpc_rx) = tracing_unbounded("mpsc_system_rpc", 10_000);

    let rpc_id_provider = config.rpc.id_provider.take();

    let gen_rpc_module = || {
        gen_rpc_module(
            genesis_number,
            task_manager.spawn_handle(),
            client.clone(),
            transaction_pool.clone(),
            keystore.clone(),
            system_rpc_tx.clone(),
            config.impl_name.clone(),
            config.impl_version.clone(),
            config.chain_spec.as_ref(),
            &config.state_pruning,
            config.blocks_pruning,
            backend.clone(),
        )
    };

    let rpc_server_handle = sc_service::start_rpc_servers(
        &config.rpc,
        config.prometheus_registry(),
        &config.tokio_handle,
        gen_rpc_module,
        rpc_id_provider,
    )?;

    let listen_addrs = rpc_server_handle
        .listen_addrs()
        .iter()
        .map(|socket_addr| {
            let mut multiaddr: Multiaddr = socket_addr.ip().into();
            multiaddr.push(sc_network_types::multiaddr::Protocol::Tcp(socket_addr.port()));
            multiaddr
        })
        .collect();

    let in_memory_rpc = {
        let mut module = gen_rpc_module()?;
        module.extensions_mut().insert(DenyUnsafe::No);
        module
    };

    let in_memory_rpc_handle = RpcHandlers::new(Arc::new(in_memory_rpc), listen_addrs);

    task_manager.keep_alive((config.base_path, rpc_server_handle, system_rpc_rx));

    Ok(in_memory_rpc_handle)
}

// Re-implement RPC module generation without the check on the genesis block number.
// The code is identical to the one in
// https://github.com/paritytech/polkadot-sdk/blob/9e0636567bebf312b065ca3acb285a8b32499df7/substrate/client/service/src/builder.rs#L754
// apart from the creation of the RPC builder inside the function and the genesis number check.
#[allow(clippy::too_many_arguments)]
fn gen_rpc_module(
    genesis_number: u64,
    spawn_handle: SpawnTaskHandle,
    client: Arc<Client>,
    transaction_pool: Arc<TransactionPoolHandle>,
    keystore: KeystorePtr,
    system_rpc_tx: TracingUnboundedSender<Request<Block>>,
    impl_name: String,
    impl_version: String,
    chain_spec: &dyn ChainSpec,
    state_pruning: &Option<PruningMode>,
    blocks_pruning: BlocksPruning,
    backend: Arc<Backend>,
) -> Result<RpcModule<()>, ServiceError> {
    // Different from the original code, we create the RPC builder inside the function.
    let rpc_builder = {
        let client = client.clone();
        let pool = transaction_pool.clone();

        Box::new(move |_| {
            let rpc_builder_ext: Result<_, ServiceError> = Ok(
                polkadot_sdk::substrate_frame_rpc_system::System::new(client.clone(), pool.clone())
                    .into_rpc(),
            );
            rpc_builder_ext
        })
    };

    let system_info = SystemInfo {
        chain_name: chain_spec.name().into(),
        impl_name,
        impl_version,
        properties: chain_spec.properties(),
        chain_type: chain_spec.chain_type(),
    };

    let mut rpc_api = RpcModule::new(());
    let task_executor = Arc::new(spawn_handle);

    let (chain, state, child_state) = {
        let chain =
            polkadot_sdk::sc_rpc::chain::new_full(client.clone(), task_executor.clone()).into_rpc();
        let (state, child_state) =
            polkadot_sdk::sc_rpc::state::new_full(client.clone(), task_executor.clone());
        let state = state.into_rpc();
        let child_state = child_state.into_rpc();

        (chain, state, child_state)
    };

    const MAX_TRANSACTION_PER_CONNECTION: usize = 16;

    let transaction_broadcast_rpc_v2 =
        polkadot_sdk::sc_rpc_spec_v2::transaction::TransactionBroadcast::new(
            client.clone(),
            transaction_pool.clone(),
            task_executor.clone(),
            MAX_TRANSACTION_PER_CONNECTION,
        )
        .into_rpc();

    let transaction_v2 = polkadot_sdk::sc_rpc_spec_v2::transaction::Transaction::new(
        client.clone(),
        transaction_pool.clone(),
        task_executor.clone(),
        None,
    )
    .into_rpc();

    let chain_head_v2 = polkadot_sdk::sc_rpc_spec_v2::chain_head::ChainHead::new(
        client.clone(),
        backend.clone(),
        task_executor.clone(),
        // Defaults to sensible limits for the `ChainHead`.
        polkadot_sdk::sc_rpc_spec_v2::chain_head::ChainHeadConfig::default(),
    )
    .into_rpc();

    let is_archive_node = state_pruning.as_ref().map(|sp| sp.is_archive()).unwrap_or(false)
        && blocks_pruning.is_archive();
    // Different from the original code, we use the genesis number to get the genesis hash.
    let Some(genesis_hash) = client.hash(genesis_number as u32).ok().flatten() else {
        return Err(ServiceError::Application(
            format!("Genesis hash not found for genesis block number {genesis_number}").into(),
        ));
    };
    if is_archive_node {
        let archive_v2 = polkadot_sdk::sc_rpc_spec_v2::archive::Archive::new(
            client.clone(),
            backend.clone(),
            genesis_hash,
            task_executor.clone(),
        )
        .into_rpc();
        rpc_api.merge(archive_v2).map_err(|e| ServiceError::Application(e.into()))?;
    }

    let chain_spec_v2 = polkadot_sdk::sc_rpc_spec_v2::chain_spec::ChainSpec::new(
        chain_spec.name().into(),
        genesis_hash,
        chain_spec.properties(),
    )
    .into_rpc();

    let author = polkadot_sdk::sc_rpc::author::Author::new(
        client,
        transaction_pool,
        keystore,
        task_executor.clone(),
    )
    .into_rpc();

    let system = polkadot_sdk::sc_rpc::system::System::new(system_info, system_rpc_tx).into_rpc();

    if let Some(storage) = backend.offchain_storage() {
        let offchain = polkadot_sdk::sc_rpc::offchain::Offchain::new(storage).into_rpc();

        rpc_api.merge(offchain).map_err(|e| ServiceError::Application(e.into()))?;
    }

    // Part of the RPC v2 spec.
    rpc_api.merge(transaction_v2).map_err(|e| ServiceError::Application(e.into()))?;
    rpc_api.merge(transaction_broadcast_rpc_v2).map_err(|e| ServiceError::Application(e.into()))?;
    rpc_api.merge(chain_head_v2).map_err(|e| ServiceError::Application(e.into()))?;
    rpc_api.merge(chain_spec_v2).map_err(|e| ServiceError::Application(e.into()))?;

    // Part of the old RPC spec.
    rpc_api.merge(chain).map_err(|e| ServiceError::Application(e.into()))?;
    rpc_api.merge(author).map_err(|e| ServiceError::Application(e.into()))?;
    rpc_api.merge(system).map_err(|e| ServiceError::Application(e.into()))?;
    rpc_api.merge(state).map_err(|e| ServiceError::Application(e.into()))?;
    rpc_api.merge(child_state).map_err(|e| ServiceError::Application(e.into()))?;
    // Additional [`RpcModule`]s defined in the node to fit the specific blockchain
    let extra_rpcs = rpc_builder(task_executor)?;
    rpc_api.merge(extra_rpcs).map_err(|e| ServiceError::Application(e.into()))?;

    Ok(rpc_api)
}
