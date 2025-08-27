use polkadot_sdk::{
    sc_basic_authorship, sc_consensus, sc_consensus_manual_seal,
    sc_executor::WasmExecutor,
    sc_network_types::{self, multiaddr::Multiaddr},
    sc_rpc_api::DenyUnsafe,
    sc_service::{self, error::Error as ServiceError, Configuration, RpcHandlers, TaskManager},
    sc_transaction_pool::{self, TransactionPoolWrapper},
    sc_utils::mpsc::tracing_unbounded,
    sp_io,
    sp_keystore::KeystorePtr,
    sp_timestamp,
    substrate_frame_rpc_system::SystemApiServer,
};
use std::sync::Arc;
use substrate_runtime::{OpaqueBlock as Block, RuntimeApi};

use crate::AnvilNodeConfig;

pub type FullClient =
    sc_service::TFullClient<Block, RuntimeApi, WasmExecutor<sp_io::SubstrateHostFunctions>>;

pub type Backend = sc_service::TFullBackend<Block>;

pub type TransactionPoolHandle = sc_transaction_pool::TransactionPoolHandle<Block, FullClient>;

type SelectChain = sc_consensus::LongestChain<Backend, Block>;

pub struct Service {
    pub task_manager: TaskManager,
    pub client: Arc<FullClient>,
    pub backend: Arc<Backend>,
    pub tx_pool: Arc<TransactionPoolHandle>,
    pub rpc_handlers: RpcHandlers,
}

/// Builds a new service for a full client.
pub fn new(
    _anvil_config: &AnvilNodeConfig,
    config: Configuration,
) -> Result<Service, ServiceError> {
    let (client, backend, keystore_container, mut task_manager) =
        sc_service::new_full_parts::<Block, RuntimeApi, _>(
            &config,
            None,
            sc_service::new_wasm_executor(&config.executor),
        )?;
    let client = Arc::new(client);

    let transaction_pool = Arc::from(
        sc_transaction_pool::Builder::new(
            task_manager.spawn_essential_handle(),
            client.clone(),
            config.role.is_authority().into(),
        )
        .with_options(config.transaction_pool.clone())
        .build(),
    );

    // Inform the tx pool about imported and finalized blocks.
    task_manager.spawn_handle().spawn(
        "txpool-notifications",
        Some("transaction-pool"),
        sc_transaction_pool::notification_future(client.clone(), transaction_pool.clone()),
    );

    let rpc_handlers = spawn_rpc_server(
        &mut task_manager,
        client.clone(),
        config,
        transaction_pool.clone(),
        keystore_container.keystore(),
        backend.clone(),
    )?;

    let proposer = sc_basic_authorship::ProposerFactory::new(
        task_manager.spawn_handle(),
        client.clone(),
        transaction_pool.clone(),
        None,
        None,
    );

    // Implement a dummy block production mechanism for now, just build an instantly finalized block
    // every 6 seconds. This will have to change.
    let default_block_time = 6000;
    let (mut sink, commands_stream) = futures::channel::mpsc::channel(1024);
    task_manager.spawn_handle().spawn("block_authoring", "anvil-polkadot", async move {
        loop {
            futures_timer::Delay::new(std::time::Duration::from_millis(default_block_time)).await;
            sink.try_send(sc_consensus_manual_seal::EngineCommand::SealNewBlock {
                create_empty: true,
                finalize: true,
                parent_hash: None,
                sender: None,
            })
            .unwrap();
        }
    });

    let params = sc_consensus_manual_seal::ManualSealParams {
        block_import: client.clone(),
        env: proposer,
        client: client.clone(),
        pool: transaction_pool.clone(),
        select_chain: SelectChain::new(backend.clone()),
        commands_stream: Box::pin(commands_stream),
        consensus_data_provider: None,
        create_inherent_data_providers: move |_, ()| async move {
            Ok(sp_timestamp::InherentDataProvider::from_system_time())
        },
    };
    let authorship_future = sc_consensus_manual_seal::run_manual_seal(params);

    task_manager.spawn_essential_handle().spawn_blocking(
        "manual-seal",
        "substrate",
        authorship_future,
    );

    Ok(Service { task_manager, client, backend, tx_pool: transaction_pool, rpc_handlers })
}

fn spawn_rpc_server(
    task_manager: &mut TaskManager,
    client: Arc<FullClient>,
    mut config: Configuration,
    transaction_pool: Arc<TransactionPoolWrapper<Block, FullClient>>,
    keystore: KeystorePtr,
    backend: Arc<Backend>,
) -> Result<RpcHandlers, ServiceError> {
    let rpc_extensions_builder = {
        let client = client.clone();
        let pool = transaction_pool.clone();

        Box::new(move |_| {
            Ok(polkadot_sdk::substrate_frame_rpc_system::System::new(client.clone(), pool.clone())
                .into_rpc())
        })
    };

    let (system_rpc_tx, system_rpc_rx) = tracing_unbounded("mpsc_system_rpc", 10_000);

    let rpc_id_provider = config.rpc.id_provider.take();

    let gen_rpc_module = || {
        sc_service::gen_rpc_module(
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
            &*rpc_extensions_builder,
            None,
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
