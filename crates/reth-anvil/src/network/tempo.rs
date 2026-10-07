//! The Tempo network: Tempo's node types, pool, and RPC with the anvil executor wrapper and a
//! dev block builder.
//!
//! Tempo's EVM config fixes its EVM factory, and its payload and pool builders take that config
//! as a concrete type, so the anvil wrapper cannot reach them from the outside. This module
//! builds the pool and the blocks itself with the wrapped config, and reuses the rest of Tempo.

use super::{
    AnvilAdapter, AnvilComponents, AnvilNetwork, NodeOf, Prepared,
    tempo_genesis::tempo_genesis_alloc,
    tempo_payload::{TempoAnvilEvmConfig, TempoAnvilPool, TempoDevPayloadBuilderBuilder},
};
use crate::{
    api::NodeIdentity,
    config::NodeConfig,
    console::ConsoleBuffer,
    evm::{
        AnvilExecutionPayload, AnvilExecutorBuilder, AnvilNextBlockEnv, ConsoleEvmFactory,
        EvmSettings,
    },
    fork::{AnvilPrimitives, ForkBackend, ForkGenesisAccount, ForkNetwork, TxPosition},
    logging::{LoggingState, NodeInfoLayer},
    pending::{AnvilEthApiBuilder, AnvilPendingEnv},
    time::AnvilPayloadAttributes,
};
use alloy_consensus::{BlockHeader, TxReceipt, transaction::Recovered};
use alloy_eips::Encodable2718;
use alloy_evm::Database;
use alloy_genesis::Genesis;
use alloy_primitives::{Address, B256, Bytes, U256};
use eyre::Result;
use foundry_evm_hardforks::EthereumHardfork;
use reth_ethereum::{
    evm::primitives::ConfigureEvm,
    network::primitives::BasicNetworkPrimitives,
    node::{
        api::{
            FullNodeComponents, FullNodeTypes, InvalidPayloadAttributesError, NewPayloadError,
            PayloadAttributesBuilder, PayloadValidator,
        },
        builder::{
            AddOnsContext, BuilderContext,
            components::{
                BasicPayloadServiceBuilder, ComponentsBuilder, NoopConsensusBuilder,
                NoopNetworkBuilder, PoolBuilder, spawn_maintenance_tasks,
            },
            rpc::{
                BasicEngineValidatorBuilder, EthApiBuilder, EthApiCtx, NoopEngineApiBuilder,
                PayloadValidatorBuilder, RpcAddOns,
            },
        },
    },
    pool::{
        Pool, PriceBumpConfig, TransactionValidationTaskExecutor, blobstore::InMemoryBlobStore,
    },
    primitives::{SealedBlock, SealedHeader},
    provider::{ChainSpecProvider, ProviderError},
};
use reth_rpc_eth_api::{RpcConverter, RpcNodeCore, helpers::pending_block::BuildPendingEnv};
use revm::Inspector;
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tempo_alloy::{TempoNetwork, rpc::TempoTransactionReceipt};
use tempo_chainspec::{TempoChainSpec, spec::DEV};
use tempo_evm::{FeeTokenResolver, TempoEvmFactory, TempoNextBlockEnvAttributes, TempoStateAccess};
use tempo_hardfork::TempoHardfork;
use tempo_node::{
    TempoNode, TempoPayloadTypes,
    engine::TempoEngineValidator,
    node::TempoExecutorBuilder,
    rpc::{TempoEthApi, TempoEthApiBounds, TempoReceiptConverter},
};
use tempo_payload_types::{TempoExecutionData, TempoPayloadAttributes};
use tempo_precompiles::{
    TIP_FEE_MANAGER_ADDRESS, error::Result as TempoResult, storage::StorageActions,
};
use tempo_primitives::{
    Block, TempoHeader, TempoPrimitives, TempoReceipt, TempoTxEnvelope, TempoTxType,
};
use tempo_revm::TempoTxEnv;
use tempo_transaction_pool::{
    AA2dPool, AA2dPoolConfig, TempoTransactionPool,
    amm::AmmLiquidityCache,
    maintain::maintain_tempo_pool,
    ordering::TempoTipOrdering,
    tt_2d_pool::DEFAULT_MAX_TXS_PER_LANE,
    validator::{
        DEFAULT_AA_VALID_AFTER_MAX_SECS, DEFAULT_MAX_TEMPO_AUTHORIZATIONS,
        TempoTransactionValidator,
    },
};
use tower::layer::util::Identity;

/// The Tempo network.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tempo;

impl AnvilNetwork for Tempo {
    type Node = TempoNode;
    type Components = ComponentsBuilder<
        AnvilAdapter<TempoNode>,
        TempoAnvilPoolBuilder,
        BasicPayloadServiceBuilder<TempoDevPayloadBuilderBuilder>,
        NoopNetworkBuilder<BasicNetworkPrimitives<TempoPrimitives, TempoTxEnvelope>>,
        AnvilExecutorBuilder<TempoExecutorBuilder>,
        NoopConsensusBuilder,
    >;
    type AddOns = RpcAddOns<
        NodeOf<Self>,
        TempoAnvilEthApiBuilder,
        TempoAnvilEngineValidatorBuilder,
        NoopEngineApiBuilder,
        BasicEngineValidatorBuilder<TempoAnvilEngineValidatorBuilder>,
        NodeInfoLayer,
    >;
    type Attributes = TempoAttributesBuilder;

    async fn prepare(config: &mut NodeConfig) -> Result<Prepared<TempoNode>> {
        if config.is_fork() {
            let (fork, accounts) = ForkBackend::<TempoFork>::setup(config).await?;
            config.apply_fork(fork.chain_id(), fork.header().header(), fork.gas_price());
            if let Some(info) = fork.node_info() {
                config.adopt_fork_identity(info);
            }
            let chain_spec = tempo_fork_chain_spec(config, fork.header(), &accounts)?;
            return Ok(Prepared { chain_spec: Arc::new(chain_spec), fork: Some(fork) });
        }
        eyre::ensure!(
            config.init_state.as_ref().is_none_or(|state| state.block.is_none()),
            "loading a Tempo state dump with blocks is not supported yet"
        );
        Ok(Prepared { chain_spec: Arc::new(tempo_chain_spec(config)?), fork: None })
    }

    fn components(anvil: &AnvilComponents) -> Self::Components {
        ComponentsBuilder::default()
            .node_types::<AnvilAdapter<TempoNode>>()
            .pool(TempoAnvilPoolBuilder)
            .executor(AnvilExecutorBuilder {
                inner: TempoExecutorBuilder::default(),
                state: anvil.impersonation.clone(),
                block_env: anvil.block_env.clone(),
                anvil_state: anvil.anvil_state.clone(),
                settings: EvmSettings::from_config(&anvil.config),
                console: anvil.console.clone(),
            })
            .payload(BasicPayloadServiceBuilder::new(TempoDevPayloadBuilderBuilder))
            .network(NoopNetworkBuilder::default())
            .consensus(NoopConsensusBuilder)
    }

    fn add_ons(anvil: &AnvilComponents, logging: LoggingState) -> Self::AddOns {
        RpcAddOns::new(
            TempoAnvilEthApiBuilder {
                pending: AnvilPendingEnv::new(anvil.time.clone(), anvil.block_env.clone()),
            },
            TempoAnvilEngineValidatorBuilder,
            NoopEngineApiBuilder::default(),
            BasicEngineValidatorBuilder::default(),
            NodeInfoLayer::new(logging),
            Identity::new(),
        )
    }

    fn payload_attributes(_chain_spec: Arc<TempoChainSpec>) -> Self::Attributes {
        TempoAttributesBuilder
    }

    fn identity(config: &NodeConfig) -> Result<NodeIdentity> {
        Ok(NodeIdentity {
            network: Some("tempo"),
            hardfork: Some(config.get_tempo_hardfork()?.to_string()),
        })
    }
}

/// Builds the chain spec of a Tempo dev chain: Tempo's dev chain config with the Tempo
/// hardforks up to the configured one, the configured chain id, and anvil's genesis with
/// Tempo's precompiles, fee tokens, and fee token liquidity.
fn tempo_chain_spec(config: &NodeConfig) -> Result<TempoChainSpec> {
    let hardfork = config.get_tempo_hardfork()?;
    let mut genesis = config.genesis_for(EthereumHardfork::Osaka)?;
    genesis.config = tempo_genesis_config(config.get_chain_id(), hardfork);
    let dev_accounts: Vec<Address> =
        config.genesis_accounts.iter().map(|account| account.address()).collect();
    let alloc =
        tempo_genesis_alloc(config.get_chain_id(), genesis.timestamp, hardfork, &dev_accounts)
            .map_err(|error| eyre::eyre!("failed to build the Tempo genesis: {error}"))?;
    merge_alloc(&mut genesis, alloc);
    Ok(TempoChainSpec::from_genesis(genesis))
}

/// Builds the chain spec of a fork of a Tempo chain at `header`: the Tempo hardforks up to the
/// one active at the fork block, or the configured one, and the remote fork block as genesis.
/// The fork keeps the remote Tempo state; anvil adds no Tempo genesis state to it.
fn tempo_fork_chain_spec(
    config: &NodeConfig,
    header: &SealedHeader<TempoHeader>,
    accounts: &[(Address, ForkGenesisAccount)],
) -> Result<TempoChainSpec> {
    let hardfork = config.tempo_hardfork_at(header.timestamp())?;
    let mut genesis = config.fork_genesis(header.header(), accounts);
    genesis.config = tempo_genesis_config(config.get_chain_id(), hardfork);
    // Tempo mainnet maps the zero validator token to a sentinel that fails fee collection, so
    // fork blocks go to the fee manager unless a coinbase is set, as Tempo's simulation does.
    if genesis.coinbase.is_zero() {
        genesis.coinbase = TIP_FEE_MANAGER_ADDRESS;
    }
    let mut spec = TempoChainSpec::from_genesis(genesis);
    spec.inner.genesis_header = header.clone();
    Ok(spec)
}

/// Returns Tempo's dev chain config with the given chain id and the Tempo hardforks up to
/// `hardfork` active from genesis.
///
/// The epoch length is the largest possible, so no block of the dev chain ends an epoch: from T8
/// on, the last block of an epoch must carry the outcome of a key generation ceremony, which a
/// dev chain has none of.
fn tempo_genesis_config(chain_id: u64, hardfork: TempoHardfork) -> alloy_genesis::ChainConfig {
    let mut chain_config = DEV.inner.genesis().config.clone();
    chain_config.chain_id = chain_id;
    for fork in TempoHardfork::VARIANTS.iter().filter(|fork| **fork > hardfork) {
        chain_config.extra_fields.remove(&format!("{}Time", fork.name().to_lowercase()));
    }
    chain_config.extra_fields.insert("epochLength".to_string(), u64::MAX.into());
    chain_config
}

/// Adds the Tempo genesis accounts to the genesis, keeping the balance and nonce anvil gives an
/// account that Tempo's genesis also writes to.
fn merge_alloc(genesis: &mut Genesis, alloc: Vec<(Address, alloy_genesis::GenesisAccount)>) {
    for (address, account) in alloc {
        match genesis.alloc.get_mut(&address) {
            Some(existing) => {
                if account.code.is_some() {
                    existing.code = account.code;
                }
                if let Some(storage) = account.storage {
                    existing.storage.get_or_insert_default().extend(storage);
                }
            }
            None => {
                genesis.alloc.insert(address, account);
            }
        }
    }
}

/// Builds the payload attributes of the next Tempo dev block. Anvil keeps time in seconds, so the
/// millisecond part of the timestamp is zero; the miner sets the timestamp itself.
#[derive(Clone, Copy, Debug, Default)]
pub struct TempoAttributesBuilder;

impl PayloadAttributesBuilder<TempoPayloadAttributes, TempoHeader> for TempoAttributesBuilder {
    fn build(&self, parent: &SealedHeader<TempoHeader>) -> TempoPayloadAttributes {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        TempoPayloadAttributes::new(None, now.max(parent.timestamp() + 1), 0, Bytes::new(), None)
    }
}

/// Builds the Tempo pool with the wrapped EVM config, as Tempo's pool builder does with Tempo's,
/// with anvil's replacement rule: any higher fee replaces a pooled transaction.
#[derive(Clone, Copy, Debug, Default)]
pub struct TempoAnvilPoolBuilder;

impl<Node> PoolBuilder<Node, TempoAnvilEvmConfig> for TempoAnvilPoolBuilder
where
    Node: FullNodeTypes<Types = TempoNode>,
{
    type Pool = TempoAnvilPool<Node::Provider>;

    async fn build_pool(
        self,
        ctx: &BuilderContext<Node>,
        evm_config: TempoAnvilEvmConfig,
    ) -> Result<Self::Pool> {
        let mut pool_config = ctx.pool_config();
        pool_config.max_inflight_delegated_slot_limit = pool_config.max_account_slots;
        pool_config.price_bumps =
            PriceBumpConfig { default_price_bump: 0, replace_blob_tx_price_bump: 0 };

        let blob_store = InMemoryBlobStore::default();
        let validator =
            TransactionValidationTaskExecutor::eth_builder(ctx.provider().clone(), evm_config)
                .with_max_tx_input_bytes(ctx.config().txpool.max_tx_input_bytes)
                .with_local_transactions_config(pool_config.local_transactions_config.clone())
                .set_tx_fee_cap(ctx.config().rpc.rpc_tx_fee_cap)
                .with_max_tx_gas_limit(ctx.config().txpool.max_tx_gas_limit)
                .set_block_gas_limit(ctx.chain_spec().inner.genesis().gas_limit)
                .disable_balance_check()
                .with_minimum_priority_fee(ctx.config().txpool.minimum_priority_fee)
                .with_additional_tasks(ctx.config().txpool.additional_validation_tasks)
                .with_custom_tx_type(TempoTxType::AA as u8)
                .no_eip4844()
                .build_with_tasks(ctx.task_executor().clone(), blob_store.clone());

        let aa_2d_pool = AA2dPool::new(AA2dPoolConfig {
            price_bump_config: pool_config.price_bumps,
            pending_limit: pool_config.pending_limit,
            queued_limit: pool_config.queued_limit,
            max_txs_per_sender: pool_config.max_account_slots,
            max_txs_per_lane: DEFAULT_MAX_TXS_PER_LANE,
        });
        let amm_liquidity_cache = AmmLiquidityCache::new(ctx.provider())?;
        let validator = validator.map(move |validator| {
            TempoTransactionValidator::new(
                validator,
                DEFAULT_AA_VALID_AFTER_MAX_SECS,
                DEFAULT_MAX_TEMPO_AUTHORIZATIONS,
                amm_liquidity_cache.clone(),
            )
        });
        let protocol_pool =
            Pool::new(validator, TempoTipOrdering::default(), blob_store, pool_config.clone());
        let pool = TempoTransactionPool::new(protocol_pool, aa_2d_pool);

        spawn_maintenance_tasks(ctx, pool.clone(), &pool_config)?;
        ctx.task_executor().spawn_critical_os_thread(
            "tempo-txpool-maintenance",
            "txpool maintenance - tempo pool",
            maintain_tempo_pool(pool.clone()),
        );
        Ok(pool)
    }
}

/// Tempo's payload validator, except that payload attributes may carry a timestamp below the
/// parent's, as anvil mines blocks with an earlier timestamp when `evm_setTime` asks for one.
#[derive(Clone, Copy, Debug, Default)]
pub struct TempoAnvilEngineValidator;

impl PayloadValidator<TempoPayloadTypes> for TempoAnvilEngineValidator {
    type Block = Block;

    fn convert_payload_to_block(
        &self,
        payload: TempoExecutionData,
    ) -> Result<SealedBlock<Block>, NewPayloadError> {
        TempoEngineValidator::new().convert_payload_to_block(payload)
    }

    fn validate_payload_attributes_against_header(
        &self,
        _attr: &TempoPayloadAttributes,
        _header: &TempoHeader,
    ) -> Result<(), InvalidPayloadAttributesError> {
        Ok(())
    }
}

/// Builds the [`TempoAnvilEngineValidator`].
#[derive(Clone, Copy, Debug, Default)]
pub struct TempoAnvilEngineValidatorBuilder;

impl<Node> PayloadValidatorBuilder<Node> for TempoAnvilEngineValidatorBuilder
where
    Node: FullNodeComponents<Types = TempoNode>,
{
    type Validator = TempoAnvilEngineValidator;

    async fn build(self, _ctx: &AddOnsContext<'_, Node>) -> Result<Self::Validator> {
        Ok(TempoAnvilEngineValidator)
    }
}

/// Builds Tempo's `eth` API with [`AnvilPendingEnv`] as the pending block environment.
#[derive(Clone, Debug)]
pub struct TempoAnvilEthApiBuilder {
    pending: AnvilPendingEnv,
}

impl Default for TempoAnvilEthApiBuilder {
    fn default() -> Self {
        Self { pending: AnvilEthApiBuilder::default().pending }
    }
}

impl<N> EthApiBuilder<N> for TempoAnvilEthApiBuilder
where
    N: FullNodeComponents<
            Types = TempoNode,
            Pool = <N as RpcNodeCore>::Pool,
            Evm = <N as RpcNodeCore>::Evm,
        > + FullNodeTypes<Provider = <N as RpcNodeCore>::Provider>
        + TempoEthApiBounds,
    <N as RpcNodeCore>::Provider: ChainSpecProvider<ChainSpec = TempoChainSpec>,
    <<N as RpcNodeCore>::Evm as ConfigureEvm>::NextBlockEnvCtx:
        BuildPendingEnv<TempoHeader> + AnvilNextBlockEnv,
{
    type EthApi = TempoEthApi<N>;

    async fn build_eth_api(self, ctx: EthApiCtx<'_, N>) -> Result<Self::EthApi> {
        let chain_spec = FullNodeComponents::provider(ctx.components).chain_spec();
        let pending = self.pending.with_chain(chain_spec.clone());
        let eth_api = ctx
            .eth_api_builder()
            .modify_gas_oracle_config(|config| config.default_suggested_fee = Some(U256::ZERO))
            .map_converter(|_| RpcConverter::new(TempoReceiptConverter::new(chain_spec)).erased())
            .with_pending_env_builder(pending)
            .build();
        Ok(TempoEthApi::new(eth_api))
    }
}

impl FeeTokenResolver for TempoAnvilEvmConfig {
    fn resolve_fee_token<S, M>(
        &self,
        state: &mut S,
        tx: &TempoTxEnv,
        fee_payer: Address,
        spec: TempoHardfork,
        actions: StorageActions,
    ) -> TempoResult<Address>
    where
        S: TempoStateAccess<M>,
    {
        self.inner().resolve_fee_token(state, tx, fee_payer, spec, actions)
    }
}

impl AnvilNextBlockEnv for TempoNextBlockEnvAttributes {
    fn set_timestamp(&mut self, timestamp: u64) {
        self.inner.set_timestamp(timestamp);
    }

    fn set_suggested_fee_recipient(&mut self, recipient: Address) {
        self.inner.set_suggested_fee_recipient(recipient);
    }

    fn set_prev_randao(&mut self, prev_randao: B256) {
        self.inner.set_prev_randao(prev_randao);
    }

    fn set_gas_limit(&mut self, gas_limit: u64) {
        self.inner.set_gas_limit(gas_limit);
    }

    fn override_parent_beacon_block_root(&mut self, root: B256) {
        self.inner.override_parent_beacon_block_root(root);
    }

    fn ensure_parent_beacon_block_root(&mut self) {
        self.inner.ensure_parent_beacon_block_root();
    }
}

impl AnvilPayloadAttributes for TempoPayloadAttributes {
    fn set_timestamp(&mut self, timestamp: u64) {
        (**self).set_timestamp(timestamp);
    }

    fn set_suggested_fee_recipient(&mut self, recipient: Address) {
        (**self).set_suggested_fee_recipient(recipient);
    }

    fn set_prev_randao(&mut self, prev_randao: B256) {
        (**self).set_prev_randao(prev_randao);
    }

    fn parent_beacon_block_root(&self) -> Option<B256> {
        (**self).parent_beacon_block_root()
    }

    fn set_parent_beacon_block_root(&mut self, root: B256) {
        (**self).set_parent_beacon_block_root(root);
    }

    fn clear_parent_beacon_block_root(&mut self) {
        (**self).clear_parent_beacon_block_root();
    }

    fn set_withdrawals_active(&mut self, active: bool) {
        (**self).set_withdrawals_active(active);
    }
}

impl AnvilExecutionPayload for TempoExecutionData {
    fn raw_transactions(&self) -> Vec<Bytes> {
        self.block
            .sealed_block()
            .body()
            .transactions
            .iter()
            .map(|tx| tx.encoded_2718().into())
            .collect()
    }
}

impl ConsoleEvmFactory for TempoEvmFactory {
    fn console<DB: Database, I: Inspector<Self::Context<DB>>>(
        _evm: &Self::Evm<DB, I>,
    ) -> Option<&ConsoleBuffer> {
        None
    }
}

/// The Tempo fork network.
#[derive(Clone, Copy, Debug, Default)]
pub struct TempoFork;

impl ForkNetwork for TempoFork {
    type Network = TempoNetwork;
    type Primitives = TempoPrimitives;

    fn uncles(response: &<TempoNetwork as alloy_network::Network>::BlockResponse) -> (B256, usize) {
        (response.header.hash, response.uncles.len())
    }

    fn block(
        response: <TempoNetwork as alloy_network::Network>::BlockResponse,
        _uncles: Vec<<TempoNetwork as alloy_network::Network>::BlockResponse>,
    ) -> Result<SealedBlock<Block>, ProviderError> {
        let hash = response.header.hash;
        let header = response.header.inner.inner;
        let transactions =
            response.transactions.into_transactions().map(|tx| tx.into_inner()).collect();
        let body = alloy_consensus::BlockBody {
            transactions,
            ommers: Vec::new(),
            withdrawals: response.withdrawals,
        };
        Ok(SealedBlock::new_unchecked(Block { header, body }, hash))
    }

    fn receipt(response: TempoTransactionReceipt) -> Result<Option<TempoReceipt>, ProviderError> {
        let receipt = response.inner.inner.receipt;
        Ok(Some(TempoReceipt {
            tx_type: receipt.tx_type,
            success: receipt.status(),
            cumulative_gas_used: receipt.cumulative_gas_used(),
            logs: receipt.logs.into_iter().map(|log| log.inner).collect(),
        }))
    }

    fn transaction(
        response: alloy_rpc_types_eth::Transaction<TempoTxEnvelope>,
    ) -> Result<Option<(TempoTxEnvelope, Option<TxPosition>)>, ProviderError> {
        let position =
            match (response.block_hash, response.block_number, response.transaction_index) {
                (Some(hash), Some(number), Some(index)) => Some((hash, number, index)),
                _ => None,
            };
        let tx: Recovered<TempoTxEnvelope> = response.inner;
        Ok(Some((tx.into_inner(), position)))
    }
}

impl AnvilPrimitives for TempoPrimitives {
    type Fork = TempoFork;
}
