use crate::{
    AnvilNodeConfig,
    substrate_node::{
        genesis::DevelopmentGenesisBlockBuilder,
        host::{PublicKeyToHashOverride, SenderAddressRecoveryOverride},
        mining_engine::{MiningEngine, MiningMode, run_mining_engine},
        rpc::spawn_rpc_server,
    },
};
use anvil::eth::backend::time::TimeManager;
use polkadot_sdk::{
    sc_basic_authorship, sc_consensus, sc_consensus_manual_seal, sc_executor,
    sc_service::{
        self, Configuration, RpcHandlers, SpawnTaskHandle, TaskManager,
        error::Error as ServiceError,
    },
    sc_transaction_pool, sp_io, sp_timestamp,
    sp_wasm_interface::ExtendedHostFunctions,
};
use std::sync::Arc;
use substrate_runtime::{OpaqueBlock as Block, RuntimeApi};
use tokio_stream::wrappers::ReceiverStream;

type Executor = sc_executor::WasmExecutor<
    ExtendedHostFunctions<
        ExtendedHostFunctions<sp_io::SubstrateHostFunctions, SenderAddressRecoveryOverride>,
        PublicKeyToHashOverride,
    >,
>;
pub type FullClient = sc_service::TFullClient<Block, RuntimeApi, Executor>;

pub type Backend = sc_service::TFullBackend<Block>;
pub type TransactionPoolHandle = sc_transaction_pool::TransactionPoolHandle<Block, FullClient>;
type SelectChain = sc_consensus::LongestChain<Backend, Block>;

#[derive(Clone)]
pub struct Service {
    pub spawn_handle: SpawnTaskHandle,
    pub client: Arc<FullClient>,
    pub backend: Arc<Backend>,
    pub tx_pool: Arc<TransactionPoolHandle>,
    pub rpc_handlers: RpcHandlers,
    pub mining_engine: Arc<MiningEngine>,
    pub genesis_block_number: u64,
}

/// Builds a new service for a full client.
pub fn new(
    anvil_config: &AnvilNodeConfig,
    config: Configuration,
) -> Result<(Service, TaskManager), ServiceError> {
    let backend = sc_service::new_db_backend(config.db_config())?;

    let wasm_executor = sc_service::new_wasm_executor(&config.executor);
    let genesis_block_builder = DevelopmentGenesisBlockBuilder::new(
        anvil_config.get_genesis_number(),
        config.chain_spec.as_storage_builder(),
        !config.no_genesis(),
        backend.clone(),
        wasm_executor.clone(),
    )?;

    let (client, backend, keystore_container, mut task_manager) =
        sc_service::new_full_parts_with_genesis_builder(
            &config,
            None,
            wasm_executor,
            backend,
            genesis_block_builder,
            false,
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

    let (seal_engine_command_sender, commands_stream) = tokio::sync::mpsc::channel(1024);
    let commands_stream = ReceiverStream::new(commands_stream);

    let mining_mode =
        MiningMode::new(anvil_config.block_time, anvil_config.mixed_mining, anvil_config.no_mining);
    let time_manager = Arc::new(TimeManager::new_with_milliseconds(
        sp_timestamp::Timestamp::from(
            anvil_config
                .get_genesis_timestamp()
                .checked_mul(1000)
                .ok_or(ServiceError::Application("Genesis timestamp overflow".into()))?,
        )
        .into(),
    ));
    let mining_engine = Arc::new(MiningEngine::new(
        mining_mode,
        transaction_pool.clone(),
        time_manager.clone(),
        seal_engine_command_sender,
    ));

    let rpc_handlers = spawn_rpc_server(
        anvil_config.get_genesis_number(),
        &mut task_manager,
        client.clone(),
        config,
        transaction_pool.clone(),
        keystore_container.keystore(),
        backend.clone(),
    )?;

    task_manager.spawn_handle().spawn(
        "mining_engine_task",
        Some("consensus"),
        run_mining_engine(mining_engine.clone()),
    );

    let proposer = sc_basic_authorship::ProposerFactory::new(
        task_manager.spawn_handle(),
        client.clone(),
        transaction_pool.clone(),
        None,
        None,
    );

    let create_inherent_data_providers = {
        move |_, ()| {
            let next_timestamp = time_manager.next_timestamp();
            async move { Ok(sp_timestamp::InherentDataProvider::new(next_timestamp.into())) }
        }
    };

    let params = sc_consensus_manual_seal::ManualSealParams {
        block_import: client.clone(),
        env: proposer,
        client: client.clone(),
        pool: transaction_pool.clone(),
        select_chain: SelectChain::new(backend.clone()),
        commands_stream: Box::pin(commands_stream),
        consensus_data_provider: None,
        create_inherent_data_providers,
    };
    let authorship_future = sc_consensus_manual_seal::run_manual_seal(params);

    task_manager.spawn_essential_handle().spawn_blocking(
        "manual-seal",
        "substrate",
        authorship_future,
    );

    Ok((
        Service {
            spawn_handle: task_manager.spawn_handle(),
            client,
            backend,
            tx_pool: transaction_pool,
            rpc_handlers,
            mining_engine,
            genesis_block_number: anvil_config.get_genesis_number(),
        },
        task_manager,
    ))
}
