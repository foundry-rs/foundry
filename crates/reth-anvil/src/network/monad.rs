//! The Monad network: reth's Ethereum node types with Monad's EVM, the anvil pool and executor
//! wrappers, and Monad's pool rules.
//!
//! Monad executes Ethereum transactions with its own gas schedule, code size limits, reserve
//! balances, and precompiles. Reserve balances depend on the senders and EIP-7702 authorities of
//! the parent and grandparent blocks, which [`MonadContextEvmFactory`] looks up for every EVM it
//! creates, and on the transactions earlier in the same block, which [`MonadBlockExecutor`] tracks.

use super::{AnvilAdapter, AnvilComponents, AnvilNetwork, NodeOf, Prepared};
use crate::{
    api::NodeIdentity,
    config::NodeConfig,
    engine::AnvilEngineValidatorBuilder,
    evm::{AnvilEvm, AnvilEvmFactory, AnvilExecutorBuilder, EvmSettings, ForkHashDb},
    fork::ForkInfo,
    impersonation::ImpersonationState,
    logging::{LoggingState, NodeInfoLayer},
    pending::AnvilEthApiBuilder,
    pool::{AnvilPoolBuilder, BalanceRule, PoolSettings},
};
use alloy_consensus::Transaction as ConsensusTransaction;
use alloy_eips::{BlockHashOrNumber, Decodable2718, eip2930::AccessList};
use alloy_evm::{
    Database, Evm, EvmEnv, EvmFactory,
    block::{
        BlockExecutionError, BlockExecutionResult, BlockExecutor, BlockExecutorFactory,
        ExecutableTx, GasOutput, StateDB,
    },
    eth::{EthBlockExecutionCtx, EthBlockExecutor, EthBlockExecutorFactory, NextEvmEnvAttributes},
    precompiles::PrecompilesMap,
};
use alloy_monad_evm::{MonadContext, MonadEvm, MonadEvmFactory};
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_rpc_types_engine::ExecutionData;
use eyre::Result;
use monad_revm::{
    MonadCfgEnv, MonadChainContext, MonadHardfork, instructions::monad_gas_params, page::page_index,
};
use reth_ethereum::{
    Block, EthPrimitives, Receipt, TransactionSigned,
    chainspec::{ChainSpec, EthChainSpec},
    engine::local::LocalPayloadAttributesBuilder,
    evm::{
        EthBlockAssembler, RethReceiptBuilder,
        primitives::{
            ConfigureEngineEvm, ConfigureEvm, EvmEnvFor, ExecutableTxIterator, ExecutionCtxFor,
            NextBlockEnvAttributes,
        },
        revm_spec_by_timestamp_and_block_number,
    },
    node::{
        EthereumAddOns, EthereumNode, EthereumPayloadBuilder,
        builder::{
            BuilderContext, FullNodeTypes, NodeTypes,
            components::{
                BasicPayloadServiceBuilder, ComponentsBuilder, ExecutorBuilder,
                NoopConsensusBuilder, NoopNetworkBuilder,
            },
            rpc::{BasicEngineApiBuilder, BasicEngineValidatorBuilder, RpcAddOns},
        },
    },
    primitives::{Header, SealedBlock, SealedHeader, SignedTransaction},
    provider::TransactionVariant,
    storage::{BlockNumReader, BlockReader, errors::any::AnyError},
};
use revm::{
    Inspector,
    context::{BlockEnv, Cfg, CfgEnv, DBErrorMarker, Transaction as _, TxEnv},
    context_interface::{
        block::BlobExcessGasAndPrice,
        result::{EVMError, HaltReason},
        transaction::AuthorizationTr,
    },
    inspector::NoOpInspector,
    primitives::{HashSet, hardfork::SpecId},
};
use std::{
    borrow::Cow,
    convert::Infallible,
    fmt::{self, Debug},
    sync::Arc,
};
use tower::layer::util::Identity;

/// The Monad network.
#[derive(Clone, Copy, Debug, Default)]
pub struct Monad;

impl AnvilNetwork for Monad {
    type Node = EthereumNode;
    type Components = ComponentsBuilder<
        AnvilAdapter<EthereumNode>,
        AnvilPoolBuilder,
        BasicPayloadServiceBuilder<EthereumPayloadBuilder>,
        NoopNetworkBuilder,
        AnvilExecutorBuilder<MonadExecutorBuilder>,
        NoopConsensusBuilder,
    >;
    type AddOns = EthereumAddOns<
        NodeOf<Self>,
        AnvilEthApiBuilder,
        AnvilEngineValidatorBuilder,
        BasicEngineApiBuilder<AnvilEngineValidatorBuilder>,
        BasicEngineValidatorBuilder<AnvilEngineValidatorBuilder>,
        NodeInfoLayer,
    >;
    type Attributes = LocalPayloadAttributesBuilder<ChainSpec>;

    async fn prepare(config: &mut NodeConfig) -> Result<Prepared<EthereumNode>> {
        super::ethereum::prepare(config).await
    }

    fn components(anvil: &AnvilComponents) -> Self::Components {
        let hardfork = anvil.config.get_monad_hardfork().unwrap_or_default();
        EthereumNode::components()
            .network(NoopNetworkBuilder::eth())
            .pool(AnvilPoolBuilder {
                state: anvil.impersonation.clone(),
                order: anvil.order.clone(),
                settings: PoolSettings {
                    balance_rule: if anvil.config.disable_pool_balance_checks {
                        BalanceRule::None
                    } else {
                        BalanceRule::GasOnly
                    },
                    reject_blob_transactions: true,
                    ..PoolSettings::from_config(&anvil.config)
                },
            })
            .executor(AnvilExecutorBuilder {
                inner: MonadExecutorBuilder {
                    hardfork,
                    fork: anvil.fork.clone(),
                    console: anvil.console.is_some(),
                    impersonation: anvil.impersonation.clone(),
                },
                state: anvil.impersonation.clone(),
                block_env: anvil.block_env.clone(),
                anvil_state: anvil.anvil_state.clone(),
                settings: EvmSettings::from_config(&anvil.config),
                console: anvil.console.clone(),
            })
            .consensus(NoopConsensusBuilder)
    }

    fn add_ons(anvil: &AnvilComponents, logging: LoggingState) -> Self::AddOns {
        EthereumAddOns::new(RpcAddOns::new(
            AnvilEthApiBuilder::new(anvil.time.clone(), anvil.block_env.clone()),
            AnvilEngineValidatorBuilder,
            BasicEngineApiBuilder::default(),
            BasicEngineValidatorBuilder::default(),
            NodeInfoLayer::new(logging),
            Identity::new(),
        ))
    }

    fn payload_attributes(chain_spec: Arc<ChainSpec>) -> Self::Attributes {
        LocalPayloadAttributesBuilder::new(chain_spec)
    }

    fn identity(config: &NodeConfig) -> Result<NodeIdentity> {
        Ok(NodeIdentity {
            network: Some("monad"),
            hardfork: Some(config.get_monad_hardfork()?.to_string()),
        })
    }
}

/// Builds the [`MonadEvmConfig`] of a node.
#[derive(Clone, Debug)]
pub struct MonadExecutorBuilder {
    /// The Monad hardfork the node runs.
    pub hardfork: MonadHardfork,
    /// The fork, if any.
    pub fork: Option<Arc<dyn ForkInfo>>,
    /// Whether to collect `console.log` calls.
    pub console: bool,
    /// The impersonation state, for the `ecrecover` override.
    pub impersonation: ImpersonationState,
}

impl<Types, Node> ExecutorBuilder<Node> for MonadExecutorBuilder
where
    Types: NodeTypes<ChainSpec = ChainSpec, Primitives = EthPrimitives>,
    Node: FullNodeTypes<Types = Types>,
{
    type EVM = MonadEvmConfig;

    async fn build_evm(self, ctx: &BuilderContext<Node>) -> Result<Self::EVM> {
        Ok(MonadEvmConfig::new(
            ctx.chain_spec(),
            self.hardfork,
            Arc::new(ctx.provider().clone()),
            self.fork,
            self.console,
            self.impersonation,
        ))
    }
}

/// The senders and EIP-7702 authorities of a block.
pub type BlockParticipants = HashSet<Address>;

/// A canonical block as the Monad chain context sees it.
#[derive(Clone, Debug)]
pub struct AncestorBlock {
    /// The hash of the block's parent.
    pub parent_hash: B256,
    /// The block's senders and authorities.
    pub participants: BlockParticipants,
}

/// Looks up canonical blocks for the Monad chain context.
pub trait ParticipantsLookup: Send + Sync {
    /// Returns the number of the chain head.
    fn head_number(&self) -> Option<u64>;

    /// Returns the block, if it exists.
    fn block(&self, id: BlockHashOrNumber) -> Option<AncestorBlock>;
}

impl<P> ParticipantsLookup for P
where
    P: BlockReader<Block = Block> + BlockNumReader + Send + Sync,
{
    fn head_number(&self) -> Option<u64> {
        self.best_block_number().ok()
    }

    fn block(&self, id: BlockHashOrNumber) -> Option<AncestorBlock> {
        let block = self.recovered_block(id, TransactionVariant::NoHash).ok()??;
        let authorities = block.body().transactions.iter().flat_map(|tx| {
            ConsensusTransaction::authorization_list(tx)
                .into_iter()
                .flatten()
                .filter_map(|authorization| authorization.recover_authority().ok())
        });
        Some(AncestorBlock {
            parent_hash: block.header().parent_hash,
            participants: block.senders().iter().copied().chain(authorities).collect(),
        })
    }
}

/// Monad's EVM factory, with the chain context of the block each EVM executes in: the
/// participants of the parent and grandparent blocks.
#[derive(Clone)]
pub struct MonadContextEvmFactory {
    inner: MonadEvmFactory,
    participants: Arc<dyn ParticipantsLookup>,
}

impl Debug for MonadContextEvmFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MonadContextEvmFactory").finish_non_exhaustive()
    }
}

impl MonadContextEvmFactory {
    /// Creates the factory.
    pub fn new(participants: Arc<dyn ParticipantsLookup>) -> Self {
        Self { inner: MonadEvmFactory::default(), participants }
    }

    /// Returns the chain context of a block built on the given parent.
    pub fn context_for_parent(&self, parent: BlockHashOrNumber) -> MonadChainContext {
        let parent = self.participants.block(parent);
        let grandparent =
            parent.as_ref().and_then(|parent| self.participants.block(parent.parent_hash.into()));
        let participants = |block: Option<AncestorBlock>| {
            block.map(|block| block.participants).unwrap_or_default()
        };
        MonadChainContext {
            parent_senders_and_authorities: participants(parent),
            grandparent_senders_and_authorities: participants(grandparent),
            ..Default::default()
        }
    }

    /// Returns the chain context for an EVM whose block environment names `number`.
    ///
    /// A block above the head is being built on the head. An RPC call at the latest block runs
    /// on top of it, like anvil's pending block. An environment at an older block replays that
    /// block, so its parent is the block before it.
    fn chain_context(&self, number: u64) -> MonadChainContext {
        let head = self.participants.head_number().unwrap_or_default();
        let parent = if number >= head { head } else { number.saturating_sub(1) };
        self.context_for_parent(parent.into())
    }
}

impl EvmFactory for MonadContextEvmFactory {
    type Evm<DB: Database, I: Inspector<MonadContext<DB>>> = MonadEvm<DB, I>;
    type Context<DB: Database> = MonadContext<DB>;
    type Tx = TxEnv;
    type Error<DBError: DBErrorMarker> = EVMError<DBError>;
    type HaltReason = HaltReason;
    type Spec = MonadHardfork;
    type BlockEnv = BlockEnv;
    type Precompiles = PrecompilesMap;

    fn create_evm<DB: Database>(
        &self,
        db: DB,
        input: EvmEnv<MonadHardfork>,
    ) -> Self::Evm<DB, NoOpInspector> {
        let number = input.block_env.number.saturating_to::<u64>();
        let mut evm = self.inner.create_evm(db, input);
        evm.ctx_mut().chain = self.chain_context(number);
        evm
    }

    fn create_evm_with_inspector<DB: Database, I: Inspector<MonadContext<DB>>>(
        &self,
        db: DB,
        input: EvmEnv<MonadHardfork>,
        inspector: I,
    ) -> Self::Evm<DB, I> {
        let number = input.block_env.number.saturating_to::<u64>();
        let mut evm = self.inner.create_evm_with_inspector(db, input, inspector);
        evm.ctx_mut().chain = self.chain_context(number);
        evm
    }
}

/// EVMs that carry a Monad chain context.
pub trait MonadChain {
    /// Returns the chain context.
    fn chain_mut(&mut self) -> &mut MonadChainContext;
}

impl<DB: Database, I, P> MonadChain for MonadEvm<DB, I, P> {
    fn chain_mut(&mut self) -> &mut MonadChainContext {
        &mut self.ctx_mut().chain
    }
}

impl<E: MonadChain, DB> MonadChain for AnvilEvm<E, DB> {
    fn chain_mut(&mut self) -> &mut MonadChainContext {
        self.inner_mut().chain_mut()
    }
}

/// Monad's EVM factory behind the anvil adapter, which serves the block hashes of a fork.
type EvmFactoryOf = AnvilEvmFactory<MonadContextEvmFactory>;

/// The EVM the factory creates.
type EvmOf<DB, I> = AnvilEvm<MonadEvm<ForkHashDb<DB>, I>, DB>;

/// The Ethereum block executor factory with Monad's EVM.
type InnerExecutorFactory =
    EthBlockExecutorFactory<RethReceiptBuilder, Arc<ChainSpec>, EvmFactoryOf>;

/// Block executor factory that tracks the current block's participants in the Monad chain
/// context.
#[derive(Clone, Debug)]
pub struct MonadBlockExecutorFactory {
    inner: InnerExecutorFactory,
    hardfork: MonadHardfork,
}

impl BlockExecutorFactory for MonadBlockExecutorFactory {
    type EvmFactory = EvmFactoryOf;
    type TxExecutionResult = <InnerExecutorFactory as BlockExecutorFactory>::TxExecutionResult;
    type ExecutionCtx<'a> = EthBlockExecutionCtx<'a>;
    type Transaction = TransactionSigned;
    type Receipt = Receipt;
    type Executor<'a, DB: StateDB, I: Inspector<MonadContext<ForkHashDb<DB>>>> = MonadBlockExecutor<
        EthBlockExecutor<'a, EvmOf<DB, I>, &'a Arc<ChainSpec>, &'a RethReceiptBuilder>,
    >;

    fn evm_factory(&self) -> &Self::EvmFactory {
        self.inner.evm_factory()
    }

    fn create_executor<'a, DB, I>(
        &'a self,
        evm: EvmOf<DB, I>,
        ctx: Self::ExecutionCtx<'a>,
    ) -> Self::Executor<'a, DB, I>
    where
        DB: StateDB,
        I: Inspector<MonadContext<ForkHashDb<DB>>>,
    {
        let mut evm = evm;
        *evm.chain_mut() =
            self.inner.evm_factory().inner().context_for_parent(ctx.parent_hash.into());
        MonadBlockExecutor { inner: self.inner.create_executor(evm, ctx), hardfork: self.hardfork }
    }
}

/// Block executor that adds every transaction to the Monad chain context before it executes,
/// and removes it again when it does not make it into the block.
#[derive(Debug)]
pub struct MonadBlockExecutor<E> {
    inner: E,
    hardfork: MonadHardfork,
}

impl<E> BlockExecutor for MonadBlockExecutor<E>
where
    E: BlockExecutor<Transaction = TransactionSigned, Evm: Evm<Tx = TxEnv> + MonadChain>,
{
    type Transaction = E::Transaction;
    type Receipt = E::Receipt;
    type Evm = E::Evm;
    type Result = E::Result;

    fn apply_pre_execution_changes(&mut self) -> Result<(), BlockExecutionError> {
        self.inner.apply_pre_execution_changes()
    }

    fn execute_transaction_without_commit(
        &mut self,
        tx: impl ExecutableTx<Self>,
    ) -> Result<Self::Result, BlockExecutionError> {
        let (mut tx_env, recovered) = tx.into_parts();
        tx_env.access_list = normalize_access_list(tx_env.access_list, self.hardfork);
        {
            let chain = self.inner.evm_mut().chain_mut();
            chain.current_tx_index = chain.current_block_senders.len();
            chain.current_block_senders.push(tx_env.caller());
            chain.current_block_authorities.push(
                tx_env
                    .authorization_list()
                    .filter_map(|authorization| authorization.authority())
                    .collect(),
            );
        }
        let result = self.inner.execute_transaction_without_commit((tx_env, recovered));
        if result.is_err() {
            let chain = self.inner.evm_mut().chain_mut();
            chain.current_block_senders.pop();
            chain.current_block_authorities.pop();
            chain.current_tx_index = chain.current_block_senders.len();
        }
        result
    }

    fn commit_transaction(&mut self, output: Self::Result) -> GasOutput {
        self.inner.commit_transaction(output)
    }

    fn finish(
        self,
    ) -> Result<(Self::Evm, BlockExecutionResult<Self::Receipt>), BlockExecutionError> {
        self.inner.finish()
    }

    fn evm_mut(&mut self) -> &mut Self::Evm {
        self.inner.evm_mut()
    }

    fn evm(&self) -> &Self::Evm {
        self.inner.evm()
    }

    fn receipts(&self) -> &[Self::Receipt] {
        self.inner.receipts()
    }
}

/// Collapses the storage keys of an access list to one per storage page, as MIP-8 charges them.
fn normalize_access_list(mut access_list: AccessList, hardfork: MonadHardfork) -> AccessList {
    if MonadHardfork::MonadTen.is_enabled_in(hardfork) {
        for item in &mut access_list.0 {
            item.storage_keys.sort_unstable();
            item.storage_keys.dedup_by_key(|slot| page_index(U256::from_be_slice(slot.as_slice())));
        }
    }
    access_list
}

/// The EVM configuration of Monad: Ethereum block building and execution with Monad's EVM.
#[derive(Clone, Debug)]
pub struct MonadEvmConfig {
    chain_spec: Arc<ChainSpec>,
    hardfork: MonadHardfork,
    executor_factory: MonadBlockExecutorFactory,
    block_assembler: EthBlockAssembler<ChainSpec>,
}

impl MonadEvmConfig {
    /// Creates the configuration for the given hardfork. `participants` resolves the ancestor
    /// blocks' senders and authorities.
    pub fn new(
        chain_spec: Arc<ChainSpec>,
        hardfork: MonadHardfork,
        participants: Arc<dyn ParticipantsLookup>,
        fork: Option<Arc<dyn ForkInfo>>,
        console: bool,
        impersonation: ImpersonationState,
    ) -> Self {
        let executor_factory = MonadBlockExecutorFactory {
            inner: EthBlockExecutorFactory::new(
                RethReceiptBuilder::default(),
                chain_spec.clone(),
                AnvilEvmFactory::new(
                    MonadContextEvmFactory::new(participants),
                    Vec::new(),
                    fork,
                    console,
                    impersonation,
                ),
            ),
            hardfork,
        };
        Self {
            block_assembler: EthBlockAssembler::new(chain_spec.clone()),
            chain_spec,
            hardfork,
            executor_factory,
        }
    }

    /// Turns an Ethereum EVM environment into Monad's: the Monad hardfork and gas schedule, and
    /// Monad's code size, initcode size, and transaction gas limits.
    ///
    /// The limits go into the environment itself, because the pool reads them from there.
    fn monad_env(&self, env: EvmEnv<SpecId>) -> EvmEnv<MonadHardfork> {
        let EvmEnv { mut cfg_env, block_env } = env;
        // The Ethereum environment carries the EIP-7825 cap; Monad has its own.
        cfg_env.tx_gas_limit_cap = None;
        let cfg = MonadCfgEnv::from(
            cfg_env.with_spec_and_gas_params(self.hardfork, monad_gas_params(self.hardfork)),
        );
        let (code_size, initcode_size, gas_cap) =
            (Cfg::max_code_size(&cfg), Cfg::max_initcode_size(&cfg), Cfg::tx_gas_limit_cap(&cfg));
        let mut cfg_env = CfgEnv::from(cfg);
        cfg_env.limit_contract_code_size = Some(code_size);
        cfg_env.limit_contract_initcode_size = Some(initcode_size);
        cfg_env.tx_gas_limit_cap = Some(gas_cap);
        EvmEnv { cfg_env, block_env }
    }
}

impl ConfigureEvm for MonadEvmConfig {
    type Primitives = EthPrimitives;
    type Error = Infallible;
    type NextBlockEnvCtx = NextBlockEnvAttributes;
    type BlockExecutorFactory = MonadBlockExecutorFactory;
    type BlockAssembler = EthBlockAssembler<ChainSpec>;

    fn block_executor_factory(&self) -> &Self::BlockExecutorFactory {
        &self.executor_factory
    }

    fn block_assembler(&self) -> &Self::BlockAssembler {
        &self.block_assembler
    }

    fn evm_env(&self, header: &Header) -> Result<EvmEnvFor<Self>, Self::Error> {
        Ok(self.monad_env(EvmEnv::for_eth_block(
            header,
            &*self.chain_spec,
            self.chain_spec.chain().id(),
            self.chain_spec.blob_params_at_timestamp(header.timestamp),
        )))
    }

    fn next_evm_env(
        &self,
        parent: &Header,
        attributes: &NextBlockEnvAttributes,
    ) -> Result<EvmEnvFor<Self>, Self::Error> {
        Ok(self.monad_env(EvmEnv::for_eth_next_block(
            parent,
            NextEvmEnvAttributes {
                timestamp: attributes.timestamp,
                suggested_fee_recipient: attributes.suggested_fee_recipient,
                prev_randao: attributes.prev_randao,
                gas_limit: attributes.gas_limit,
                slot_number: attributes.slot_number,
            },
            self.chain_spec.next_block_base_fee(parent, attributes.timestamp).unwrap_or_default(),
            &*self.chain_spec,
            self.chain_spec.chain().id(),
            self.chain_spec.blob_params_at_timestamp(attributes.timestamp),
        )))
    }

    fn context_for_block<'a>(
        &self,
        block: &'a SealedBlock<Block>,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        Ok(EthBlockExecutionCtx {
            tx_count_hint: Some(block.transaction_count()),
            parent_hash: block.header().parent_hash,
            parent_beacon_block_root: block.header().parent_beacon_block_root,
            ommers: &block.body().ommers,
            withdrawals: block.body().withdrawals.as_ref().map(|w| Cow::Borrowed(w.as_slice())),
            extra_data: block.header().extra_data.clone(),
            slot_number: block.header().slot_number,
        })
    }

    fn context_for_next_block(
        &self,
        parent: &SealedHeader,
        attributes: Self::NextBlockEnvCtx,
    ) -> Result<ExecutionCtxFor<'_, Self>, Self::Error> {
        Ok(EthBlockExecutionCtx {
            tx_count_hint: None,
            parent_hash: parent.hash(),
            parent_beacon_block_root: attributes.parent_beacon_block_root,
            ommers: &[],
            withdrawals: attributes.withdrawals.map(|w| Cow::Owned(w.into_inner())),
            extra_data: attributes.extra_data,
            slot_number: attributes.slot_number,
        })
    }
}

impl ConfigureEngineEvm<ExecutionData> for MonadEvmConfig {
    fn evm_env_for_payload(&self, payload: &ExecutionData) -> Result<EvmEnvFor<Self>, Self::Error> {
        let timestamp = payload.payload.timestamp();
        let block_number = payload.payload.block_number();
        let blob_params = self.chain_spec.blob_params_at_timestamp(timestamp);
        let spec =
            revm_spec_by_timestamp_and_block_number(&*self.chain_spec, timestamp, block_number);

        let mut cfg_env = CfgEnv::new()
            .with_chain_id(self.chain_spec.chain().id())
            .with_spec_and_mainnet_gas_params(spec);
        if let Some(blob_params) = &blob_params {
            cfg_env.set_max_blobs_per_tx(blob_params.max_blobs_per_tx);
        }

        let blob_excess_gas_and_price =
            payload.payload.excess_blob_gas().zip(blob_params).map(|(excess_blob_gas, params)| {
                let blob_gasprice = params.calc_blob_fee(excess_blob_gas);
                BlobExcessGasAndPrice { excess_blob_gas, blob_gasprice }
            });
        let block_env = BlockEnv {
            number: U256::from(block_number),
            beneficiary: payload.payload.fee_recipient(),
            timestamp: U256::from(timestamp),
            difficulty: U256::ZERO,
            prevrandao: Some(payload.payload.as_v1().prev_randao),
            gas_limit: payload.payload.gas_limit(),
            basefee: payload.payload.saturated_base_fee_per_gas(),
            blob_excess_gas_and_price,
            slot_num: payload.payload.as_v4().map(|v4| v4.slot_number).unwrap_or_default(),
        };

        Ok(self.monad_env(EvmEnv { cfg_env, block_env }))
    }

    fn context_for_payload<'a>(
        &self,
        payload: &'a ExecutionData,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        Ok(EthBlockExecutionCtx {
            tx_count_hint: Some(payload.payload.transactions().len()),
            parent_hash: payload.parent_hash(),
            parent_beacon_block_root: payload.sidecar.parent_beacon_block_root(),
            ommers: &[],
            withdrawals: payload.payload.withdrawals().map(|w| Cow::Borrowed(w.as_slice())),
            extra_data: payload.payload.as_v1().extra_data.clone(),
            slot_number: payload.payload.as_v4().map(|v4| v4.slot_number),
        })
    }

    fn tx_iterator_for_payload(
        &self,
        payload: &ExecutionData,
    ) -> Result<impl ExecutableTxIterator<Self>, Self::Error> {
        let txs = payload.payload.transactions().clone();
        let convert = move |tx: Bytes| {
            let tx = TransactionSigned::decode_2718_exact(tx.as_ref()).map_err(AnyError::new)?;
            let signer = tx.try_recover().map_err(AnyError::new)?;
            Ok::<_, AnyError>(tx.with_signer(signer))
        };
        Ok((txs, convert))
    }
}
