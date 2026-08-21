//! Monad-specific execution and context helpers for the in-memory backend.

use super::{
    AnvilInspector, Backend, ClientFork, DatabaseRef, EnvelopeExecutionKind, MonadExecutionContext,
    storage::BlockchainStorage,
};
use crate::eth::{
    backend::{
        db::MonadBlockReplayProfile, executor::build_tx_env_for_pending,
        replay::HistoricalReplayTransaction,
    },
    error::BlockchainError,
};
use alloy_consensus::{BlockHeader, constants::EMPTY_ROOT_HASH};
use alloy_evm::{Database, Evm, EvmEnv, EvmFactory, RecoveredTx};
use alloy_monad_evm::{MonadContext, MonadEvm, MonadEvmFactory};
use alloy_network::{BlockResponse, Network};
use alloy_primitives::B256;
use alloy_rpc_types::{BlockNumberOrTag as BlockNumber, BlockTransactions};
use anvil_core::eth::{
    block::Block,
    transaction::{MaybeImpersonatedTransaction, PendingTransaction},
};
use foundry_evm::{
    backend::DatabaseError,
    core::{
        FromAnyRpcTransaction,
        evm::{
            EvmEnvFor, FoundryEvmFactory, MonadBlockParticipants, MonadEvmNetwork,
            monad_block_participants, monad_context_from_participants,
        },
    },
    hardfork::FoundryHardfork,
};
use foundry_primitives::FoundryTxEnvelope;
use monad_revm::{MonadChainContext, MonadHardfork, instructions::monad_gas_params};
use revm::{
    Inspector,
    context::{Transaction, TxEnv},
    context_interface::{
        result::{HaltReason, ResultAndState},
        transaction::AuthorizationTr,
    },
    database_interface::WrapDatabaseRef,
};

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

pub(super) struct PreparedExecution {
    pub(super) context: Option<MonadChainContext>,
    pub(super) kind: EnvelopeExecutionKind,
    pub(super) hardfork: MonadHardfork,
}

/// Caches the fork blocks needed to construct the next Monad block's ancestor context.
pub(super) async fn cache_fork_context(fork: &ClientFork) -> Result<(), BlockchainError> {
    let block_number = fork.block_number();
    let block =
        fork.block_by_number_full(block_number).await?.ok_or(BlockchainError::BlockNotFound)?;
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

pub(super) fn advance_block(context: &mut MonadChainContext) {
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

    pub(super) fn monad_context_before_mined_transaction(
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
            execution.context.unwrap_or_else(|| factory.chain_context_for_transaction(&tx_env));
        let mut evm = factory.create_evm_with_inspector(WrapDatabaseRef(db), monad_env, inspector);
        evm.ctx_mut().chain = context;
        self.inject_precompiles(evm.precompiles_mut(), evm_env);
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
