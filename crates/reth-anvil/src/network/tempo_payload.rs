//! A sequential block builder for Tempo dev chains.
//!
//! Tempo's payload builder takes Tempo's EVM config as a concrete type, so it cannot build with
//! the anvil wrapper around it. This builder follows Tempo's rules for a dev chain: the block gas
//! limit, the general gas limit for transactions that are not payments, and the validator fees
//! as the payload value. It leaves out Tempo's prewarming, parallel replay, and build budgets,
//! which serve block production under consensus.

use crate::{evm::AnvilEvmConfig, time::TimeManager};
use alloy_consensus::BlockHeader;
use alloy_evm::{
    Database, Evm,
    block::{BlockExecutionError, BlockValidationError},
};
use alloy_primitives::U256;
use alloy_rlp::Encodable;
use reth_basic_payload_builder::{
    BuildArguments, BuildOutcome, MissingPayloadBehaviour, PayloadBuilder, PayloadConfig,
    is_better_payload,
};
use reth_ethereum::{
    chainspec::EthereumHardforks,
    evm::{
        primitives::{
            ConfigureEvm, EvmEnvFor, EvmFor, ExecutionCtxFor,
            execute::{BlockBuilder, BlockBuilderOutcome},
        },
        revm::{cached::CachedReads, database::StateProviderDatabase, db::State},
    },
    node::{
        api::{
            BuiltPayloadExecutedBlock, FullNodeTypes, NextBlockEnvAttributes, PayloadBuilderError,
        },
        builder::{BuilderContext, PayloadBuilderConfig, components::PayloadBuilderBuilder},
    },
    pool::{
        BestTransactions, BestTransactionsAttributes, TransactionPool,
        error::InvalidPoolTransactionError,
    },
    primitives::{
        RecoveredBlock, SealedBlock, SealedHeader, transaction::error::InvalidTransactionError,
    },
    provider::{ChainSpecProvider, StateProviderFactory},
    storage::StateProvider,
};
use reth_payload_builder::EthBuiltPayload;
use revm::context::Cfg;
use std::{sync::Arc, time::Duration};
use tempo_chainspec::{TempoChainSpec, hardfork::TempoHardforks};
use tempo_evm::{TempoEvmConfig, TempoNextBlockEnvAttributes};
use tempo_node::TempoNode;
use tempo_payload_types::{EncodedBlock, TempoBuiltPayload, TempoPayloadAttributes};
use tempo_primitives::{TempoHeader, TempoPrimitives};
use tempo_transaction_pool::{
    TempoTransactionPool,
    transaction::{TempoPoolTransactionError, TempoPooledTransaction},
};

/// The EVM config a Tempo dev node runs: Tempo's, with the anvil wrapper.
pub type TempoAnvilEvmConfig = AnvilEvmConfig<TempoEvmConfig>;

/// Builds the [`TempoDevPayloadBuilder`] of a node.
#[derive(Clone, Copy, Debug, Default)]
pub struct TempoDevPayloadBuilderBuilder;

impl<Node, Pool> PayloadBuilderBuilder<Node, Pool, TempoAnvilEvmConfig>
    for TempoDevPayloadBuilderBuilder
where
    Node: FullNodeTypes<Types = TempoNode>,
    Pool: TransactionPool<Transaction = TempoPooledTransaction> + Unpin + 'static,
{
    type PayloadBuilder = TempoDevPayloadBuilder<Pool, Node::Provider>;

    async fn build_payload_builder(
        self,
        ctx: &BuilderContext<Node>,
        pool: Pool,
        evm_config: TempoAnvilEvmConfig,
    ) -> eyre::Result<Self::PayloadBuilder> {
        Ok(TempoDevPayloadBuilder {
            pool,
            provider: ctx.provider().clone(),
            evm_config,
            gas_limit: ctx.payload_builder_config().gas_limit(),
        })
    }
}

/// Builds Tempo dev blocks one transaction after the other.
#[derive(Clone, Debug)]
pub struct TempoDevPayloadBuilder<Pool, Provider> {
    pool: Pool,
    provider: Provider,
    evm_config: TempoAnvilEvmConfig,
    /// The configured block gas limit, if any; the parent's otherwise.
    gas_limit: Option<u64>,
}

impl<Pool, Provider> PayloadBuilder for TempoDevPayloadBuilder<Pool, Provider>
where
    Pool: TransactionPool<Transaction = TempoPooledTransaction>,
    Provider: StateProviderFactory + ChainSpecProvider<ChainSpec = TempoChainSpec> + Clone,
{
    type Attributes = TempoPayloadAttributes;
    type BuiltPayload = TempoBuiltPayload;

    fn try_build(
        &self,
        args: BuildArguments<Self::Attributes, Self::BuiltPayload>,
    ) -> Result<BuildOutcome<Self::BuiltPayload>, PayloadBuilderError> {
        self.build(args, |attributes| self.pool.best_transactions_with_attributes(attributes))
    }

    fn on_missing_payload(
        &self,
        _args: BuildArguments<Self::Attributes, Self::BuiltPayload>,
    ) -> MissingPayloadBehaviour<Self::BuiltPayload> {
        MissingPayloadBehaviour::AwaitInProgress
    }

    fn build_empty_payload(
        &self,
        config: PayloadConfig<Self::Attributes, tempo_primitives::TempoHeader>,
    ) -> Result<Self::BuiltPayload, PayloadBuilderError> {
        let args = BuildArguments::new(
            CachedReads::default(),
            None,
            None,
            config,
            Default::default(),
            None,
        );
        self.build(args, |_| {
            Box::new(std::iter::empty())
                as Box<
                    dyn BestTransactions<
                        Item = Arc<
                            reth_ethereum::pool::ValidPoolTransaction<TempoPooledTransaction>,
                        >,
                    >,
                >
        })?
        .into_payload()
        .ok_or(PayloadBuilderError::MissingPayload)
    }
}

impl<Pool, Provider> TempoDevPayloadBuilder<Pool, Provider>
where
    Pool: TransactionPool<Transaction = TempoPooledTransaction>,
    Provider: StateProviderFactory + ChainSpecProvider<ChainSpec = TempoChainSpec> + Clone,
{
    fn build<Txs>(
        &self,
        args: BuildArguments<TempoPayloadAttributes, TempoBuiltPayload>,
        best_txs: impl FnOnce(BestTransactionsAttributes) -> Txs,
    ) -> Result<BuildOutcome<TempoBuiltPayload>, PayloadBuilderError>
    where
        Txs: BestTransactions<
            Item = Arc<reth_ethereum::pool::ValidPoolTransaction<TempoPooledTransaction>>,
        >,
    {
        let BuildArguments { mut cached_reads, config, cancel, best_payload, .. } = args;
        let PayloadConfig { parent_header, attributes, .. } = config;

        let state_provider = self.provider.state_by_block_hash(parent_header.hash())?;
        let evm_state_provider = (&state_provider).into_evm_state_provider();
        let state = StateProviderDatabase::new(&evm_state_provider);
        let mut db = State::builder()
            .with_database(cached_reads.as_db_mut(state))
            .with_bundle_update()
            .build();

        let chain_spec = self.provider.chain_spec();
        let block_gas_limit = self.gas_limit.unwrap_or_else(|| parent_header.gas_limit());
        let general_gas_limit =
            chain_spec.general_gas_limit_at(attributes.timestamp, block_gas_limit, 0);
        let hardfork = chain_spec.tempo_hardfork_at(attributes.timestamp);

        let next_attributes = TempoNextBlockEnvAttributes {
            inner: NextBlockEnvAttributes {
                timestamp: attributes.timestamp,
                suggested_fee_recipient: attributes.suggested_fee_recipient,
                prev_randao: attributes.prev_randao,
                gas_limit: block_gas_limit,
                parent_beacon_block_root: attributes.parent_beacon_block_root,
                withdrawals: attributes.withdrawals.clone().map(Into::into),
                extra_data: attributes.extra_data().clone(),
                slot_number: attributes.slot_number,
            },
            general_gas_limit,
            shared_gas_limit: 0,
            timestamp_millis_part: attributes.timestamp_millis_part(),
            consensus_context: attributes.consensus_context(),
        };
        let tx_gas_limit_cap = self
            .evm_config
            .next_evm_env(&parent_header, &next_attributes)
            .map_err(PayloadBuilderError::other)?
            .cfg_env
            .tx_gas_limit_cap();
        let mut builder = self
            .evm_config
            .builder_for_next_block(&mut db, &parent_header, next_attributes)
            .map_err(PayloadBuilderError::other)?;
        builder
            .apply_pre_execution_changes()
            .map_err(|error| PayloadBuilderError::Internal(error.into()))?;

        let base_fee = builder.evm().block().basefee;
        let mut best_txs = best_txs(BestTransactionsAttributes::new(base_fee, None));
        let mut cumulative_gas_used = 0u64;
        let mut non_payment_gas_used = 0u64;
        let mut total_fees = U256::ZERO;

        while let Some(pool_tx) = best_txs.next() {
            if cancel.is_cancelled() {
                return Ok(BuildOutcome::Cancelled);
            }
            // The regular gas a transaction can use is capped, so a transaction above the cap
            // fits on what it can use.
            let gas_limit = pool_tx.gas_limit().min(tx_gas_limit_cap);
            if cumulative_gas_used + gas_limit > block_gas_limit {
                best_txs.mark_invalid(
                    &pool_tx,
                    InvalidPoolTransactionError::ExceedsGasLimit(
                        pool_tx.gas_limit(),
                        block_gas_limit - cumulative_gas_used,
                    ),
                );
                continue;
            }
            let is_payment = if hardfork.is_t5() {
                pool_tx.transaction.is_payment()
            } else {
                pool_tx.transaction.inner().is_payment_v1()
            };
            if !is_payment && non_payment_gas_used + gas_limit > general_gas_limit {
                best_txs.mark_invalid(
                    &pool_tx,
                    InvalidPoolTransactionError::Other(Box::new(
                        TempoPoolTransactionError::ExceedsNonPaymentLimit,
                    )),
                );
                continue;
            }

            let mut block_gas = 0;
            let mut validator_fee = U256::ZERO;
            match builder.execute_transaction_with_result_closure(
                pool_tx.to_consensus(),
                |result| {
                    block_gas = result.block_gas_used();
                    validator_fee = result.validator_fee();
                },
            ) {
                Ok(_) => {}
                Err(BlockExecutionError::Validation(BlockValidationError::InvalidTx {
                    error,
                    ..
                })) => {
                    if !error.is_nonce_too_low() {
                        best_txs.mark_invalid(
                            &pool_tx,
                            InvalidPoolTransactionError::Consensus(
                                InvalidTransactionError::TxTypeNotSupported,
                            ),
                        );
                    }
                    continue;
                }
                Err(error) => return Err(PayloadBuilderError::evm(error)),
            }
            cumulative_gas_used += block_gas;
            if !is_payment {
                non_payment_gas_used += block_gas;
            }
            total_fees += validator_fee;
        }

        if !is_better_payload(best_payload.as_ref(), total_fees) {
            drop(builder);
            return Ok(BuildOutcome::Aborted { fees: total_fees, cached_reads });
        }

        let BlockBuilderOutcome { execution_result, block, hashed_state, trie_updates, .. } =
            builder.finish(state_provider.as_ref(), None)?;
        let requests = chain_spec
            .is_prague_active_at_timestamp(attributes.timestamp)
            .then(|| execution_result.requests.clone());
        let block: Arc<RecoveredBlock<tempo_primitives::Block>> = Arc::new(block);
        let size = block.sealed_block().length();
        let execution_output = reth_ethereum::provider::BlockExecutionOutput {
            result: execution_result,
            state: db.take_bundle(),
        };
        let executed = BuiltPayloadExecutedBlock::<TempoPrimitives> {
            recovered_block: block.clone(),
            execution_output: Arc::new(execution_output),
            hashed_state: Arc::new(hashed_state),
            trie_updates: Arc::new(trie_updates),
        };
        let payload = TempoBuiltPayload::new(
            EthBuiltPayload::new(block, total_fees, requests, None),
            None,
            Some(executed),
            Duration::ZERO,
            Duration::ZERO,
            size,
            EncodedBlock::default(),
        );
        Ok(BuildOutcome::Freeze(payload))
    }
}

/// The pool of a Tempo dev node.
pub type TempoAnvilPool<Provider> = TempoTransactionPool<Provider, TempoPoolEvmConfig>;

/// The EVM config of the Tempo pool: the node's, except that the pool validates a transaction
/// against the block anvil mines next, at the time anvil's clock gives it, and skips the fee
/// balance check when the pool balance checks are off. Only the pool uses it; blocks run on the
/// node's config.
#[derive(Clone, Debug)]
pub struct TempoPoolEvmConfig {
    inner: TempoAnvilEvmConfig,
    time: TimeManager,
    disable_balance_check: bool,
}

impl TempoPoolEvmConfig {
    /// Wraps the node's EVM config with anvil's clock.
    pub const fn new(
        inner: TempoAnvilEvmConfig,
        time: TimeManager,
        disable_balance_check: bool,
    ) -> Self {
        Self { inner, time, disable_balance_check }
    }
}

impl ConfigureEvm for TempoPoolEvmConfig {
    type Primitives = TempoPrimitives;
    type Error = <TempoAnvilEvmConfig as ConfigureEvm>::Error;
    type NextBlockEnvCtx = <TempoAnvilEvmConfig as ConfigureEvm>::NextBlockEnvCtx;
    type BlockExecutorFactory = <TempoAnvilEvmConfig as ConfigureEvm>::BlockExecutorFactory;
    type BlockAssembler = <TempoAnvilEvmConfig as ConfigureEvm>::BlockAssembler;

    fn block_executor_factory(&self) -> &Self::BlockExecutorFactory {
        self.inner.block_executor_factory()
    }

    fn block_assembler(&self) -> &Self::BlockAssembler {
        self.inner.block_assembler()
    }

    fn evm_env(&self, header: &TempoHeader) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.inner.evm_env(header)
    }

    fn next_evm_env(
        &self,
        parent: &TempoHeader,
        attributes: &Self::NextBlockEnvCtx,
    ) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.inner.next_evm_env(parent, attributes)
    }

    fn context_for_block<'a>(
        &self,
        block: &'a SealedBlock<tempo_primitives::Block>,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        self.inner.context_for_block(block)
    }

    fn context_for_next_block(
        &self,
        parent: &SealedHeader<TempoHeader>,
        attributes: Self::NextBlockEnvCtx,
    ) -> Result<ExecutionCtxFor<'_, Self>, Self::Error> {
        self.inner.context_for_next_block(parent, attributes)
    }

    fn evm_with_env<DB: Database>(&self, db: DB, mut evm_env: EvmEnvFor<Self>) -> EvmFor<Self, DB> {
        // The pool checks time bounds and expiring nonces against the next block's timestamp,
        // which anvil's clock sets, not the timestamp of the tip.
        evm_env.block_env.inner.timestamp = U256::from(self.time.current_call_timestamp());
        evm_env.block_env.timestamp_millis_part = 0;
        if self.disable_balance_check {
            evm_env.cfg_env.disable_balance_check = true;
        }
        self.inner.evm_with_env(db, evm_env)
    }
}
