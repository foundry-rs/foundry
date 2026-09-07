//! Monad-specific execution and context helpers for the in-memory backend.

use super::{
    AnvilInspector, Backend, ClientFork, DatabaseRef, EnvelopeExecutionKind, MonadExecutionContext,
    storage::BlockchainStorage,
};
use crate::eth::{
    backend::{
        db::MonadBlockReplayProfile,
        executor::{
            AnvilBlockExecutor, ExecutedPoolTransactions, PoolTransactionHooks, PoolTxGasConfig,
            build_tx_env_for_pending, execute_pool_transactions,
        },
        replay::{
            ExecutedHistoricalReplay, HistoricalReplayTransaction, execute_historical_replay_with,
        },
    },
    error::{BlockchainError, InvalidTransactionError},
    pool::transactions::PoolTransaction,
};
use alloy_consensus::{
    BlockHeader, Transaction as _, constants::EMPTY_ROOT_HASH, transaction::Recovered,
};
use alloy_evm::{
    Database, Evm, EvmEnv, EvmFactory, RecoveredTx,
    block::{BlockExecutionError, BlockExecutionResult, BlockExecutor, StateDB},
};
use alloy_monad_evm::{MonadContext, MonadEvm, MonadEvmFactory};
use alloy_network::{BlockResponse, Network};
use alloy_primitives::{B256, U256};
use alloy_rpc_types::{AccessList, BlockNumberOrTag as BlockNumber, BlockTransactions};
use anvil_core::eth::{
    block::Block,
    transaction::{MaybeImpersonatedTransaction, PendingTransaction},
};
use eyre::{Context, Result};
use foundry_evm::{
    backend::DatabaseError,
    core::{
        FoundryChain, FromAnyRpcTransaction,
        evm::{
            EvmEnvFor, FoundryEvmFactory, MonadBlockParticipants, MonadEvmNetwork,
            monad_block_participants, monad_context_from_participants, protocol_system_call,
        },
    },
    hardfork::FoundryHardfork,
    utils::get_blob_params,
};
use foundry_primitives::{FoundryReceiptEnvelope, FoundryTxEnvelope};
use monad_revm::{MonadChainContext, MonadHardfork, instructions::monad_gas_params};
use revm::{
    Inspector,
    context::{Transaction, TxEnv},
    context_interface::{
        result::{HaltReason, InvalidTransaction, ResultAndState},
        transaction::AuthorizationTr,
    },
    database_interface::WrapDatabaseRef,
    primitives::hardfork::SpecId,
    state::AccountInfo,
};
use std::sync::Arc;
use tracing::debug;

pub(super) struct PreparedExecution {
    pub(super) context: Option<MonadChainContext>,
    pub(super) kind: EnvelopeExecutionKind,
    pub(super) hardfork: MonadHardfork,
}

pub(super) struct ForkReplay {
    hardfork: FoundryHardfork,
    context: Option<MonadChainContext>,
    participants: MonadBlockParticipants,
    execution_chain_id: u64,
    source_chain_id: u64,
    timestamp: u64,
    inferred_hardfork: bool,
}

impl ForkReplay {
    pub(super) const fn hardfork(&self) -> FoundryHardfork {
        self.hardfork
    }

    pub(super) const fn take_context(&mut self) -> Option<MonadChainContext> {
        self.context.take()
    }

    pub(super) fn store_metadata<N: Network>(
        &mut self,
        storage: &mut BlockchainStorage<N>,
        block_hash: B256,
    ) {
        store_block_metadata(
            storage,
            block_hash,
            std::mem::take(&mut self.participants),
            self.execution_chain_id,
            self.hardfork,
        );
    }
}

pub(super) fn normalize_access_list(
    mut access_list: AccessList,
    hardfork: MonadHardfork,
) -> AccessList {
    if MonadHardfork::MonadTen.is_enabled_in(hardfork) {
        for item in &mut access_list.0 {
            item.storage_keys.sort_unstable();
            item.storage_keys.dedup_by_key(|slot| {
                monad_revm::page::page_index(U256::from_be_slice(slot.as_slice()))
            });
        }
    }
    access_list
}

/// Caches the fork blocks needed to construct the next Monad block's ancestor context.
pub(super) async fn cache_fork_context(fork: &ClientFork) -> Result<(), BlockchainError> {
    let block =
        fork.block_by_hash_full(fork.block_hash()).await?.ok_or(BlockchainError::BlockNotFound)?;
    let parent_hash = block.header().parent_hash();
    if !parent_hash.is_zero() {
        fork.block_by_hash_full(parent_hash).await?.ok_or(BlockchainError::BlockNotFound)?;
    }
    Ok(())
}

/// Adds a candidate transaction to the current Monad block context.
fn append_transaction(chain: &mut MonadChainContext, tx: &TxEnv) {
    chain.current_tx_index = chain.current_block_senders.len();
    chain.current_block_senders.push(tx.caller());
    chain.current_block_authorities.push(
        tx.authorization_list().filter_map(|authorization| authorization.authority()).collect(),
    );
}

pub(super) fn prepare_transaction<DB: alloy_evm::Database>(
    evm: &mut MonadEvm<DB, AnvilInspector>,
    tx: &TxEnv,
) {
    append_transaction(&mut evm.ctx_mut().chain, tx);
}

pub(super) fn resolve_execution_context(
    context: Option<MonadExecutionContext<'_>>,
    tx: &TxEnv,
) -> Option<MonadChainContext> {
    match context {
        Some(MonadExecutionContext::Exact(context)) => Some(*context),
        Some(MonadExecutionContext::Next(context)) => {
            append_transaction(context, tx);
            Some(context.clone())
        }
        None => None,
    }
}

pub(super) fn advance_block_context(context: &mut Option<MonadChainContext>) {
    let Some(context) = context else { return };
    let current = context
        .current_block_senders
        .iter()
        .copied()
        .chain(context.current_block_authorities.iter().flatten().copied())
        .collect();
    context.grandparent_senders_and_authorities =
        std::mem::replace(&mut context.parent_senders_and_authorities, current);
    context.current_block_senders.clear();
    context.current_block_authorities.clear();
    context.current_tx_index = 0;
}

/// Removes a candidate transaction that failed before block inclusion.
pub(super) fn rollback_transaction<DB: alloy_evm::Database>(
    evm: &mut MonadEvm<DB, AnvilInspector>,
) {
    let chain = &mut evm.ctx_mut().chain;
    chain.current_block_senders.pop();
    chain.current_block_authorities.pop();
    chain.current_tx_index = chain.current_block_senders.len();
}

impl<N: Network> Backend<N> {
    /// Prepares the Monad-specific inputs for replaying a historical transaction prefix.
    pub(super) async fn prepare_monad_fork_replay(
        &self,
        source_chain_id: u64,
        execution_chain_id: u64,
        timestamp: u64,
        parent_hash: B256,
        transactions: &[HistoricalReplayTransaction],
    ) -> Result<Option<ForkReplay>> {
        if !self.is_monad() {
            return Ok(None);
        }

        let explicit_hardfork = self.node_config.read().await.hardfork;
        // The target block's schedule can cross a hardfork boundary after the selected parent.
        // Explicit overrides still take precedence over timestamp inference.
        let hardfork = explicit_hardfork
            .or_else(|| {
                MonadHardfork::from_chain_and_timestamp(source_chain_id, timestamp)
                    .map(FoundryHardfork::Monad)
            })
            .unwrap_or_else(|| self.hardfork());
        let context = self.monad_context_for_child_of_block_hash(parent_hash).await?;
        let participants =
            monad_block_participants(&self.monad_historical_replay_tx_envs(transactions));

        Ok(Some(ForkReplay {
            hardfork,
            context: Some(context),
            participants,
            execution_chain_id,
            source_chain_id,
            timestamp,
            inferred_hardfork: explicit_hardfork.is_none(),
        }))
    }

    /// Applies the Monad execution rules selected for a completed fork replay.
    pub(super) fn finalize_monad_fork_replay(&self, replay: &ForkReplay, evm_env: &mut EvmEnv) {
        let spec_id = SpecId::from(replay.hardfork);
        evm_env.cfg_env.set_spec_and_mainnet_gas_params(spec_id);
        self.fees.set_execution_rules(
            spec_id,
            self.networks.base_fee_params(replay.timestamp),
            None,
        );
        self.fees.set_blob_params(get_blob_params(replay.source_chain_id, replay.timestamp));

        if replay.inferred_hardfork {
            *self.hardfork.write() = replay.hardfork;
            if let Some(fork) = self.fork.read().clone() {
                fork.config.write().hardfork = Some(replay.hardfork);
            }
        }
    }

    /// Validates Monad-specific transaction type restrictions.
    pub(super) const fn validate_monad_transaction_type(
        &self,
        tx: &FoundryTxEnvelope,
    ) -> Result<(), InvalidTransactionError> {
        if self.is_monad() && tx.is_eip4844() {
            return Err(InvalidTransactionError::MonadBlobTransactionUnsupported);
        }
        Ok(())
    }

    /// Validates Monad's gas-only transaction balance requirement.
    ///
    /// Returns whether the transaction was validated as a Monad transaction.
    pub(super) fn validate_monad_transaction_funds(
        &self,
        pending: &PendingTransaction<FoundryTxEnvelope>,
        account: &AccountInfo,
        evm_env: &EvmEnv,
    ) -> Result<bool, InvalidTransactionError> {
        if !self.is_monad() {
            return Ok(false);
        }

        let tx = &pending.transaction;
        let effective_gas_price = tx.effective_gas_price(Some(evm_env.block_env.basefee));
        let required = U256::from(tx.gas_limit()) * U256::from(effective_gas_price);
        if account.balance < required {
            debug!(target: "backend", "[{:?}] insufficient balance={}, required={} account={:?}", tx.hash(), account.balance, required, *pending.sender());
            return Err(InvalidTransactionError::InsufficientFunds);
        }
        Ok(true)
    }

    /// Validates a forced Monad protocol transaction selected for mining.
    ///
    /// Returns whether the transaction was recognized and fully validated as a protocol call.
    pub(super) fn validate_monad_mining_pool_transaction_for(
        &self,
        pool_tx: &PoolTransaction<FoundryTxEnvelope>,
        account: &AccountInfo,
        evm_env: &EvmEnv,
    ) -> Result<bool, InvalidTransactionError> {
        if !self.is_monad() || !pool_tx.is_replay {
            return Ok(false);
        }

        let tx_env: TxEnv = build_tx_env_for_pending(&pool_tx.pending_transaction, self.cheats());
        let Some(system_call) = protocol_system_call(&tx_env).map_err(|err| {
            InvalidTransactionError::Revm(InvalidTransaction::Str(err.to_string().into()))
        })?
        else {
            return Ok(false);
        };
        if system_call.chain_id.is_some_and(|chain_id| chain_id != evm_env.cfg_env.chain_id) {
            return Err(InvalidTransactionError::InvalidChainId);
        }
        if system_call.nonce < account.nonce {
            return Err(InvalidTransactionError::NonceTooLow);
        }
        if system_call.nonce > account.nonce {
            return Err(InvalidTransactionError::NonceTooHigh);
        }
        if system_call.nonce == u64::MAX {
            return Err(InvalidTransactionError::NonceMaxValue);
        }
        Ok(true)
    }

    /// Executes a candidate block through a concrete Monad EVM.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub(super) fn execute_with_monad_block_executor<DB>(
        &self,
        db: DB,
        evm_env: &EvmEnv,
        parent_hash: B256,
        spec_id: SpecId,
        hardfork: FoundryHardfork,
        pool_transactions: &[Arc<PoolTransaction<FoundryTxEnvelope>>],
        gas_config: &PoolTxGasConfig,
        inspector_tx_config: &crate::mem::inspector::InspectorTxConfig,
        validator: &dyn Fn(
            &PoolTransaction<FoundryTxEnvelope>,
            &AccountInfo,
        ) -> Result<(), InvalidTransactionError>,
    ) -> Result<
        (ExecutedPoolTransactions<FoundryTxEnvelope>, BlockExecutionResult<FoundryReceiptEnvelope>),
        BlockchainError,
    >
    where
        DB: StateDB<Error = DatabaseError>,
    {
        let hardfork = MonadHardfork::from(hardfork);
        let monad_env = Self::build_monad_evm_env(evm_env, hardfork);
        let inspector = self.build_mining_inspector();
        let mut evm =
            MonadEvmFactory::default().create_evm_with_inspector(db, monad_env, inspector);
        let transaction_context = self
            .monad_context_for_child_of(parent_hash)
            .expect("Monad ancestor context must be available before block execution");
        evm.ctx_mut().chain = transaction_context;
        self.inject_precompiles(evm.precompiles_mut(), evm_env);

        let mut executor = AnvilBlockExecutor::new(evm, parent_hash, spec_id, None)
            .with_max_blob_gas_per_block(gas_config.max_blob_gas_per_block);
        executor
            .apply_pre_execution_changes()
            .map_err(|err| BlockchainError::Internal(err.to_string()))?;
        let mut hooks = PoolTransactionHooks {
            before_transaction: prepare_transaction,
            execute_transaction: |executor: &mut AnvilBlockExecutor<_>,
                                  tx_env: TxEnv,
                                  recovered: Recovered<FoundryTxEnvelope>,
                                  is_replay: bool| {
                if !is_replay {
                    return executor.execute_transaction_without_commit((tx_env, recovered));
                }
                match protocol_system_call(&tx_env) {
                    Ok(None) => {
                        return executor.execute_transaction_without_commit((tx_env, recovered));
                    }
                    Ok(Some(_)) => {}
                    Err(err) => return Err(BlockExecutionError::msg(err)),
                }
                executor.execute_transaction_without_commit_with(
                    (tx_env, recovered),
                    |evm, tx_env, transaction_hash| {
                        MonadEvmFactory::default()
                            .try_transact_system_replay(evm, &tx_env)
                            .map_err(|err| {
                                BlockExecutionError::msg(format!(
                                    "failed to replay Monad transaction {transaction_hash}: {err}"
                                ))
                            })?
                            .ok_or_else(|| {
                                BlockExecutionError::msg(format!(
                                    "Monad transaction {transaction_hash} is not a canonical replay \
                                     envelope"
                                ))
                            })
                    },
                )
            },
            on_execution_error: rollback_transaction,
        };
        let pool_result = execute_pool_transactions(
            &mut executor,
            pool_transactions,
            gas_config,
            inspector_tx_config,
            self.cheats(),
            validator,
            &mut hooks,
        );
        let (evm, block_result) =
            executor.finish().map_err(|err| BlockchainError::Internal(err.to_string()))?;
        drop(evm);
        Ok((pool_result, block_result))
    }

    /// Executes a historical transaction prefix through a concrete Monad EVM.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn execute_with_monad_replay_block_executor<DB>(
        &self,
        db: DB,
        evm_env: &EvmEnv,
        parent_hash: B256,
        hardfork: FoundryHardfork,
        transactions: &[HistoricalReplayTransaction],
        inspector_tx_config: &crate::mem::inspector::InspectorTxConfig,
        transaction_context: Option<MonadChainContext>,
    ) -> Result<ExecutedHistoricalReplay>
    where
        DB: StateDB<Error = DatabaseError>,
    {
        let hardfork = MonadHardfork::from(hardfork);
        let monad_env = Self::build_monad_evm_env(evm_env, hardfork);
        let inspector = self.build_mining_inspector();
        let mut evm =
            MonadEvmFactory::default().create_evm_with_inspector(db, monad_env, inspector);
        evm.ctx_mut().chain = transaction_context
            .ok_or_else(|| eyre::eyre!("Monad replay ancestor context is unavailable"))?;
        self.inject_precompiles(evm.precompiles_mut(), evm_env);

        let mut executor = AnvilBlockExecutor::new(evm, parent_hash, *evm_env.spec_id(), None)
            .with_state_changes();
        executor
            .apply_pre_execution_changes()
            .wrap_err("failed to apply replay block-start transitions")?;
        let (transactions, transaction_infos) = execute_historical_replay_with(
            &mut executor,
            transactions,
            inspector_tx_config,
            |evm, tx_env, _transaction_hash| {
                prepare_transaction(evm, &tx_env);
                let result = match MonadEvmFactory::default()
                    .try_transact_system_replay(evm, &tx_env)
                    .map_err(BlockExecutionError::msg)
                {
                    Ok(Some(result)) => Ok(result),
                    Ok(None) => evm.transact(tx_env).map_err(BlockExecutionError::msg),
                    Err(err) => Err(err),
                };
                if result.is_err() {
                    rollback_transaction(evm);
                }
                result
            },
        )?;
        let state_changes = executor.take_state_changes();
        let (evm, block_result) =
            executor.finish().wrap_err("failed to finish replay block execution")?;
        drop(evm);
        Ok(ExecutedHistoricalReplay {
            block_result,
            transactions,
            transaction_infos,
            state_changes,
        })
    }

    /// Reconstructs a locally mined transaction using its authoritative stored sender.
    pub(super) fn monad_pending_mined_transaction_from_storage(
        storage: &BlockchainStorage<N>,
        transaction: MaybeImpersonatedTransaction<FoundryTxEnvelope>,
    ) -> Result<PendingTransaction<FoundryTxEnvelope>, BlockchainError> {
        let transaction_hash = transaction.hash();
        let mined =
            storage.transactions.get(&transaction_hash).ok_or(BlockchainError::DataUnavailable)?;
        Ok(PendingTransaction::with_sender(transaction, mined.info.from))
    }

    /// Converts retained local transactions using their authoritative mined senders.
    fn monad_tx_envs_from_storage(
        &self,
        storage: &BlockchainStorage<N>,
        transactions: &[MaybeImpersonatedTransaction<FoundryTxEnvelope>],
    ) -> Result<Vec<TxEnv>, BlockchainError> {
        transactions
            .iter()
            .map(|transaction| {
                let pending = Self::monad_pending_mined_transaction_from_storage(
                    storage,
                    transaction.clone(),
                )?;
                Ok(build_tx_env_for_pending::<FoundryTxEnvelope, TxEnv>(&pending, self.cheats()))
            })
            .collect()
    }

    /// Converts a historical replay prefix using its prepared authoritative senders.
    pub(super) fn monad_historical_replay_tx_envs(
        &self,
        transactions: &[HistoricalReplayTransaction],
    ) -> Vec<TxEnv> {
        transactions
            .iter()
            .map(|replay| {
                let pending = PendingTransaction::with_sender(
                    MaybeImpersonatedTransaction::new(replay.transaction.tx().clone()),
                    replay.transaction.signer(),
                );
                build_tx_env_for_pending::<FoundryTxEnvelope, TxEnv>(&pending, self.cheats())
            })
            .collect()
    }

    /// Returns a block's number, parent hash, and cached participants.
    fn monad_block_participants_from_storage(
        &self,
        storage: &BlockchainStorage<N>,
        hash: B256,
    ) -> Result<(u64, B256, MonadBlockParticipants), BlockchainError> {
        if let Some(block) = storage.blocks.get(&hash) {
            let participants = match storage.monad_block_participants.get(&hash) {
                Some(participants) => participants.clone(),
                None if block.body.transactions.is_empty()
                    && block.header.transactions_root() != EMPTY_ROOT_HASH =>
                {
                    return Err(BlockchainError::DataUnavailable);
                }
                None => monad_block_participants(
                    &self.monad_tx_envs_from_storage(storage, &block.body.transactions)?,
                ),
            };
            return Ok((block.header.number(), block.header.parent_hash, participants));
        }

        let fork = self.get_fork().ok_or(BlockchainError::BlockNotFound)?;
        let block = fork
            .storage
            .read()
            .blocks
            .get(&hash)
            .cloned()
            .ok_or(BlockchainError::DataUnavailable)?;
        let BlockTransactions::Full(transactions) = block.transactions() else {
            return Err(BlockchainError::DataUnavailable);
        };
        let tx_envs = transactions
            .iter()
            .map(TxEnv::from_any_rpc_transaction)
            .collect::<eyre::Result<Vec<_>>>()?;
        Ok((
            block.header().number(),
            block.header().parent_hash(),
            monad_block_participants(&tx_envs),
        ))
    }

    fn monad_block_participants(
        &self,
        hash: B256,
    ) -> Result<(u64, B256, MonadBlockParticipants), BlockchainError> {
        self.monad_block_participants_from_storage(&self.blockchain.storage.read(), hash)
    }

    /// Rebuilds participant metadata for locally stored blocks with transaction bodies.
    pub(super) fn rebuild_monad_block_participant_cache(
        &self,
        storage: &mut BlockchainStorage<N>,
    ) -> Result<(), BlockchainError> {
        let participants = storage
            .blocks
            .iter()
            .filter(|(hash, block)| {
                !storage.monad_block_participants.contains_key(*hash)
                    && (!block.body.transactions.is_empty()
                        || block.header.transactions_root() == EMPTY_ROOT_HASH)
            })
            .map(|(hash, block)| (*hash, block.body.transactions.clone()))
            .map(|(hash, transactions)| {
                let tx_envs = self.monad_tx_envs_from_storage(storage, &transactions)?;
                Ok((hash, monad_block_participants(&tx_envs)))
            })
            .collect::<Result<Vec<_>, BlockchainError>>()?;

        for (hash, participants) in participants {
            if storage.blocks.contains_key(&hash) {
                storage.monad_block_participants.insert(hash, participants);
            }
        }
        Ok(())
    }

    /// Builds the initial context for a block whose parent is `parent_hash`.
    pub(super) fn monad_context_for_child_of(
        &self,
        parent_hash: B256,
    ) -> Result<MonadChainContext, BlockchainError> {
        self.monad_context_for_child_of_in_storage(&self.blockchain.storage.read(), parent_hash)
    }

    /// Builds the initial context from staged storage.
    pub(super) fn monad_context_for_child_of_in_storage(
        &self,
        storage: &BlockchainStorage<N>,
        parent_hash: B256,
    ) -> Result<MonadChainContext, BlockchainError> {
        let (_, grandparent_hash, parent) =
            self.monad_block_participants_from_storage(storage, parent_hash)?;
        let grandparent = if grandparent_hash.is_zero() {
            MonadBlockParticipants::default()
        } else {
            self.monad_block_participants_from_storage(storage, grandparent_hash)?.2
        };
        Ok(monad_context_from_participants(grandparent, parent, &[], 0))
    }

    async fn monad_context_for_child_of_block(
        &self,
        block: alloy_network::AnyRpcBlock,
        block_hash: B256,
    ) -> Result<MonadChainContext, BlockchainError> {
        let parent_hash = block.header().parent_hash();
        if !parent_hash.is_zero() {
            self.block_by_hash_full(parent_hash).await?.ok_or(BlockchainError::DataUnavailable)?;
        }
        self.monad_context_for_child_of(block_hash)
    }

    /// Fetches the full blocks required to build context on top of `block_number`.
    pub(super) async fn monad_context_for_child_of_block_number(
        &self,
        block_number: u64,
    ) -> Result<MonadChainContext, BlockchainError> {
        let block = self
            .block_by_number_full(BlockNumber::Number(block_number))
            .await?
            .ok_or(BlockchainError::BlockNotFound)?;
        let block_hash = block.header().hash;
        self.monad_context_for_child_of_block(block, block_hash).await
    }

    /// Fetches the full blocks required to build context on top of `block_hash`.
    pub(super) async fn monad_context_for_child_of_block_hash(
        &self,
        block_hash: B256,
    ) -> Result<MonadChainContext, BlockchainError> {
        let block =
            self.block_by_hash_full(block_hash).await?.ok_or(BlockchainError::BlockNotFound)?;
        self.monad_context_for_child_of_block(block, block_hash).await
    }

    fn monad_context_for_mined_transactions(
        &self,
        block: &Block,
        current: &[TxEnv],
        current_tx_index: usize,
    ) -> Result<MonadChainContext, BlockchainError> {
        let (_, grandparent_hash, parent) =
            self.monad_block_participants(block.header.parent_hash)?;
        let grandparent = if grandparent_hash.is_zero() {
            MonadBlockParticipants::default()
        } else {
            self.monad_block_participants(grandparent_hash)?.2
        };
        Ok(monad_context_from_participants(grandparent, parent, current, current_tx_index))
    }

    fn monad_context_for_mined_block(
        &self,
        block: &Block,
    ) -> Result<MonadChainContext, BlockchainError> {
        let current = self.monad_tx_envs_from_storage(
            &self.blockchain.storage.read(),
            &block.body.transactions,
        )?;
        self.monad_context_for_mined_transactions(block, &current, 0)
    }

    pub(super) fn active_monad_context_for_mined_block(
        &self,
        block: &Block,
    ) -> Result<Option<MonadChainContext>, BlockchainError> {
        self.is_monad().then(|| self.monad_context_for_mined_block(block)).transpose()
    }

    fn monad_context_before_mined_transaction(
        &self,
        block: &Block,
        current_tx_index: usize,
    ) -> Result<MonadChainContext, BlockchainError> {
        let current = self.monad_tx_envs_from_storage(
            &self.blockchain.storage.read(),
            &block.body.transactions,
        )?;
        if current_tx_index > current.len() {
            return Err(BlockchainError::DataUnavailable);
        }
        self.monad_context_for_mined_transactions(
            block,
            &current[..current_tx_index],
            current_tx_index,
        )
    }

    pub(super) fn active_monad_context_before_mined_transaction(
        &self,
        block: &Block,
        current_tx_index: usize,
    ) -> Result<Option<MonadChainContext>, BlockchainError> {
        self.is_monad()
            .then(|| self.monad_context_before_mined_transaction(block, current_tx_index))
            .transpose()
    }

    /// Builds the Monad [`EvmEnv`] (spec and gas params) from a base env.
    pub(super) fn build_monad_evm_env(
        evm_env: &EvmEnv,
        hardfork: MonadHardfork,
    ) -> EvmEnvFor<MonadEvmNetwork> {
        EvmEnv::new(
            evm_env.cfg_env.clone().with_spec_and_gas_params(hardfork, monad_gas_params(hardfork)),
            evm_env.block_env.clone(),
        )
    }

    /// Monad path of [`Backend::transact_call_with_inspector_ref`].
    pub(super) fn transact_monad_with_inspector_ref<'db, I, DB>(
        &self,
        db: &'db DB,
        evm_env: &EvmEnv,
        inspector: &mut I,
        tx_env: TxEnv,
        execution: PreparedExecution,
    ) -> Result<ResultAndState<HaltReason>, BlockchainError>
    where
        DB: DatabaseRef + ?Sized,
        I: Inspector<MonadContext<WrapDatabaseRef<&'db DB>>>,
        WrapDatabaseRef<&'db DB>: Database<Error = DatabaseError>,
    {
        let monad_env = Self::build_monad_evm_env(evm_env, execution.hardfork);
        let factory = MonadEvmFactory::default();
        let context =
            execution.context.unwrap_or_else(|| MonadChainContext::for_transaction(&tx_env));
        let mut evm = factory.create_evm_with_inspector(WrapDatabaseRef(db), monad_env, inspector);
        evm.ctx_mut().chain = context;
        self.inject_configured_precompiles(evm.precompiles_mut(), evm_env);
        match execution.kind {
            EnvelopeExecutionKind::Transaction => Ok(evm.transact(tx_env)?),
            EnvelopeExecutionKind::Replay => {
                if let Some(result) = factory.try_transact_system_replay(&mut evm, &tx_env)? {
                    Ok(result)
                } else {
                    Ok(evm.transact(tx_env)?)
                }
            }
        }
    }
}

pub(super) fn store_block_metadata<N: Network>(
    storage: &mut BlockchainStorage<N>,
    block_hash: B256,
    participants: MonadBlockParticipants,
    execution_chain_id: u64,
    hardfork: FoundryHardfork,
) {
    storage.monad_block_participants.insert(block_hash, participants);
    storage.monad_block_replay_profiles.insert(
        block_hash,
        MonadBlockReplayProfile { execution_chain_id, hardfork: MonadHardfork::from(hardfork) },
    );
}
