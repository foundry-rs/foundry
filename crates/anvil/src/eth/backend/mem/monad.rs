//! Monad-specific execution and context helpers for the in-memory backend.

use super::{Backend, ClientFork, MonadExecutionContext, storage::BlockchainStorage};
use crate::eth::{
    backend::{
        cheats::CheatsManager,
        db::MonadBlockReplayProfile,
        env::{BlobExcessGasAndPrice, EvmEnv, SpecId},
        evm::PreparedTx,
        replay::HistoricalReplayTransaction,
    },
    error::{BlockchainError, InvalidTransactionError},
    fees::FeeManager,
    pool::transactions::PoolTransaction,
};
use alloy_consensus::{BlockHeader, Header, Transaction, constants::EMPTY_ROOT_HASH};
use alloy_eips::eip1559::BaseFeeParams;
use alloy_network::{BlockResponse, Network, TransactionResponse};
use alloy_primitives::{Address, B256, U256, map::HashSet};
use alloy_rpc_types::{AccessList, BlockNumberOrTag as BlockNumber, BlockTransactions};
use anvil_core::eth::{
    block::Block,
    transaction::{MaybeImpersonatedTransaction, PendingTransaction},
};
use evm2::AccountInfo;
use eyre::Result;
use foundry_evm_core::{evm::monad::is_protocol_system_legacy, utils::get_blob_params};
use foundry_evm_hardforks::{FoundryHardfork, MonadHardfork};
use foundry_primitives::{FoundryReceiptEnvelope, FoundryTxEnvelope};
use monad_evm2::{MonadChainContext, staking::constants::SYSTEM_ADDRESS};
use tracing::debug;

/// Senders and EIP-7702 authorities that participated in one Monad block.
pub type MonadBlockParticipants = HashSet<Address>;

/// The sender and recovered EIP-7702 authorities of one transaction.
struct Participant {
    sender: Address,
    authorities: Vec<Address>,
}

impl Participant {
    fn new(sender: Address, tx: &impl Transaction, cheats: &CheatsManager) -> Self {
        let authorities = tx
            .authorization_list()
            .unwrap_or_default()
            .iter()
            .filter_map(|authorization| {
                authorization.recover_authority().ok().or_else(|| {
                    let signature = authorization.signature().ok()?;
                    cheats.get_recover_override(&signature.as_bytes().into())
                })
            })
            .collect();
        Self { sender, authorities }
    }

    fn from_pending(
        pending: &PendingTransaction<FoundryTxEnvelope>,
        cheats: &CheatsManager,
    ) -> Self {
        Self::new(*pending.sender(), pending.transaction.as_ref(), cheats)
    }
}

/// Collects all senders and EIP-7702 authorities of a block's transactions.
fn participants_of(transactions: &[Participant]) -> MonadBlockParticipants {
    transactions
        .iter()
        .flat_map(|tx| std::iter::once(tx.sender).chain(tx.authorities.iter().copied()))
        .collect()
}

/// Collects all senders and EIP-7702 authorities of the given pending transactions.
pub(super) fn block_participants<'a>(
    transactions: impl IntoIterator<Item = &'a PendingTransaction<FoundryTxEnvelope>>,
    cheats: &CheatsManager,
) -> MonadBlockParticipants {
    participants_of(
        &transactions
            .into_iter()
            .map(|tx| Participant::from_pending(tx, cheats))
            .collect::<Vec<_>>(),
    )
}

/// Builds Monad context from cached ancestor participants and the current block transactions.
fn context_from_participants(
    grandparent_senders_and_authorities: MonadBlockParticipants,
    parent_senders_and_authorities: MonadBlockParticipants,
    current: &[Participant],
    current_tx_index: usize,
) -> MonadChainContext {
    MonadChainContext {
        grandparent_senders_and_authorities,
        parent_senders_and_authorities,
        current_block_senders: current.iter().map(|tx| tx.sender).collect(),
        current_block_authorities: current.iter().map(|tx| tx.authorities.clone()).collect(),
        current_tx_index,
        ..Default::default()
    }
}

/// Returns whether `tx`, attributed to the system sender, is a canonical Monad protocol system
/// envelope, whatever its signature recovers to.
fn is_protocol_system_envelope(tx: &FoundryTxEnvelope) -> bool {
    match tx {
        FoundryTxEnvelope::Legacy(tx) => is_protocol_system_legacy(tx.tx()),
        _ => false,
    }
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

/// Monad execution state staged while rewinding to a retained canonical block.
pub(super) struct MonadRollbackProfile {
    hardfork: FoundryHardfork,
    fees: FeeManager,
    block_blob_excess_gas_and_price: Option<BlobExcessGasAndPrice>,
    publish_inferred_hardfork: bool,
}

impl MonadRollbackProfile {
    /// Rewinds cfg/spec, gas params, and the retained block's fee environment.
    pub(super) fn apply_env(&self, env: &mut EvmEnv) {
        env.cfg_env.spec = SpecId::from(self.hardfork);
        env.block_env.blob_excess_gas_and_price = self.block_blob_excess_gas_and_price;
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
                monad_evm2::page::page_index(U256::from_be_slice(slot.as_slice()))
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
fn append_transaction(chain: &mut MonadChainContext, tx: &PreparedTx, cheats: &CheatsManager) {
    let participant = Participant::new(tx.sender, &tx.tx, cheats);
    chain.current_tx_index = chain.current_block_senders.len();
    chain.current_block_senders.push(participant.sender);
    chain.current_block_authorities.push(participant.authorities);
}

pub(super) fn resolve_execution_context(
    context: Option<MonadExecutionContext<'_>>,
    tx: &PreparedTx,
    cheats: &CheatsManager,
) -> Option<MonadChainContext> {
    match context {
        Some(MonadExecutionContext::Exact(context)) => Some(*context),
        Some(MonadExecutionContext::Next(context)) => {
            append_transaction(context, tx, cheats);
            Some(context.clone())
        }
        // The EVM derives the standalone context from the transaction itself.
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
pub(super) fn rollback_transaction(chain: &mut MonadChainContext) {
    chain.current_block_senders.pop();
    chain.current_block_authorities.pop();
    chain.current_tx_index = chain.current_block_senders.len();
}

impl<N: Network> Backend<N> {
    /// Returns the remote rollback ancestor for a Monad transaction-hash fork.
    pub(crate) async fn monad_rollback_block(
        &self,
        number: u64,
    ) -> Result<Option<Block>, BlockchainError> {
        let Some(fork) = self.get_fork().filter(|fork| {
            fork.transaction_hash().is_some() && fork.predates_fork_inclusive(number)
        }) else {
            return Ok(None);
        };
        let Some(block) = fork.block_by_number(number).await? else {
            return Ok(None);
        };
        let header = Header::try_from(block.header().inner.clone())
            .map_err(|err| BlockchainError::Internal(err.to_string()))?;
        Ok(Some(Block { header: header.into(), body: Default::default() }))
    }

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
            participants_of(&self.monad_historical_replay_participants(transactions));

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
        let spec_id = SpecId::from(replay.hardfork());
        evm_env.cfg_env.spec = spec_id;
        self.fees.set_execution_rules(
            spec_id,
            self.networks.base_fee_params(replay.timestamp),
            None,
        );
        self.fees.set_blob_params(get_blob_params(replay.source_chain_id, replay.timestamp));

        if replay.inferred_hardfork {
            *self.hardfork.write() = replay.hardfork();
            if let Some(fork) = self.fork.read().clone() {
                fork.config.write().hardfork = Some(replay.hardfork());
            }
        }
    }

    /// Stages the Monad fork protocol profile and next-block fee state for a retained block.
    pub(super) async fn prepare_monad_rollback_profile(
        &self,
        common_block: &Block,
    ) -> Option<MonadRollbackProfile> {
        // Local nodes keep their configured profile, even when the execution chain ID changes.
        let fork = self.get_fork()?;

        let explicit_hardfork = self.node_config.read().await.hardfork;
        let block_hash = common_block.header.hash_slow();
        let stored_hardfork = self
            .blockchain
            .storage
            .read()
            .monad_block_replay_profiles
            .get(&block_hash)
            .map(|profile| profile.hardfork);
        let endpoint_hardfork = {
            let config = fork.config.read();
            if config.block_number != common_block.header.number()
                || config.block_hash != block_hash
            {
                None
            } else {
                match config.endpoint_identity.hardfork {
                    Some(FoundryHardfork::Monad(hardfork)) => Some(hardfork),
                    _ => None,
                }
            }
        };
        let source_chain_id = fork.chain_id();
        let scheduled_hardfork = MonadHardfork::from_chain_and_timestamp(
            source_chain_id,
            common_block.header.timestamp(),
        );
        let hardfork = explicit_hardfork.unwrap_or_else(|| {
            FoundryHardfork::Monad(
                stored_hardfork
                    .or(endpoint_hardfork)
                    .or(scheduled_hardfork)
                    .unwrap_or_else(|| self.monad_hardfork()),
            )
        });
        let spec_id = SpecId::from(hardfork);
        let timestamp = common_block.header.timestamp();
        let blob_params = get_blob_params(source_chain_id, timestamp);
        let block_base_fee = common_block.header.base_fee_per_gas.unwrap_or_default();
        let is_fork_root = fork.block_number() == common_block.header.number();

        let fees = self.fees.detached();
        fees.set_execution_rules(spec_id, BaseFeeParams::ethereum(), None);
        fees.set_blob_params(blob_params);
        // The fee manager stores values for the next block. Seed it with the retained block while
        // deriving those values so the zero-base-fee sentinel and Osaka blob target are applied to
        // the correct parent.
        fees.set_base_fee(block_base_fee);
        let (next_block_base_fee, next_block_excess_blob_gas) = if is_fork_root {
            if let Some(child) = self.get_block(common_block.header.number().saturating_add(1)) {
                (
                    child.header.base_fee_per_gas().unwrap_or_default(),
                    child.header.excess_blob_gas().unwrap_or_default(),
                )
            } else {
                let snapshot = self.fees.snapshot();
                (snapshot.base_fee, snapshot.blob_excess_gas_and_price.excess_blob_gas)
            }
        } else {
            (
                fees.get_next_block_base_fee_from_header(&common_block.header),
                blob_params.next_block_excess_blob_gas_osaka(
                    common_block.header.excess_blob_gas.unwrap_or_default(),
                    common_block.header.blob_gas_used.unwrap_or_default(),
                    block_base_fee,
                ),
            )
        };
        fees.set_base_fee(next_block_base_fee);
        fees.set_blob_excess_gas_and_price(BlobExcessGasAndPrice::new(
            next_block_excess_blob_gas,
            blob_params.update_fraction as u64,
        ));

        Some(MonadRollbackProfile {
            hardfork,
            fees,
            block_blob_excess_gas_and_price: common_block.header.excess_blob_gas.map(|excess| {
                BlobExcessGasAndPrice::new(excess, blob_params.update_fraction as u64)
            }),
            publish_inferred_hardfork: explicit_hardfork.is_none(),
        })
    }

    /// Returns transaction fee defaults for the retained block's staged profile.
    pub(crate) async fn monad_rollback_fee_defaults(
        &self,
        common_block: &Block,
        suggested_tip: u128,
    ) -> Option<(u128, u128)> {
        let profile = self.prepare_monad_rollback_profile(common_block).await?;
        let gas_price = if profile.fees.is_eip1559() {
            let base_fee = profile.fees.base_fee() as u128;
            if profile.fees.is_min_priority_fee_enforced() {
                base_fee.saturating_add(suggested_tip)
            } else {
                base_fee
            }
        } else {
            profile.fees.raw_gas_price()
        };
        Some((gas_price, profile.fees.get_next_block_blob_base_fee_per_gas()))
    }

    /// Rewinds state and publishes the Monad profile selected for the retained block.
    pub(crate) async fn rollback_monad(
        &self,
        common_block: Block,
        mining_guard: &tokio::sync::MutexGuard<'_, ()>,
    ) -> Result<(), BlockchainError>
    where
        N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope>,
    {
        let profile = self.prepare_monad_rollback_profile(&common_block).await;
        self.rollback(common_block, mining_guard).await?;
        if let Some(profile) = profile {
            profile.apply_env(&mut self.evm_env.write());
            self.publish_monad_rollback_profile(profile);
        }
        Ok(())
    }

    /// Publishes the staged Monad fee rules and inferred hardfork after a rewind.
    pub(super) fn publish_monad_rollback_profile(&self, profile: MonadRollbackProfile) {
        self.fees.replace_from(&profile.fees);
        if profile.publish_inferred_hardfork {
            *self.hardfork.write() = profile.hardfork;
            if let Some(fork) = self.fork.read().clone() {
                fork.config.write().hardfork = Some(profile.hardfork);
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
    /// Returns whether the transaction was recognized as a canonical protocol system call. Its
    /// chain ID, nonce and reward mint are validated by the protocol replay at execution, which
    /// rejects the transaction atomically; it pays no gas, so ordinary fee and balance validation
    /// does not apply.
    pub(super) fn validate_monad_mining_pool_transaction_for(
        &self,
        pool_tx: &PoolTransaction<FoundryTxEnvelope>,
    ) -> bool {
        pool_tx.is_replay
            && *pool_tx.pending_transaction.sender() == SYSTEM_ADDRESS
            && self.is_monad_protocol_envelope(pool_tx.pending_transaction.transaction.as_ref())
    }

    /// Returns whether `tx` is a canonical Monad protocol system envelope of this Monad chain,
    /// whatever its signature recovers to.
    pub(crate) fn is_monad_protocol_envelope(&self, tx: &FoundryTxEnvelope) -> bool {
        self.is_monad() && is_protocol_system_envelope(tx)
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

    /// Collects retained local transactions' participants using their authoritative mined
    /// senders.
    fn monad_participants_from_storage(
        &self,
        storage: &BlockchainStorage<N>,
        transactions: &[MaybeImpersonatedTransaction<FoundryTxEnvelope>],
    ) -> Result<Vec<Participant>, BlockchainError> {
        transactions
            .iter()
            .map(|transaction| {
                let pending = Self::monad_pending_mined_transaction_from_storage(
                    storage,
                    transaction.clone(),
                )?;
                Ok(Participant::from_pending(&pending, self.cheats()))
            })
            .collect()
    }

    /// Collects a historical replay prefix's participants using its prepared authoritative
    /// senders.
    fn monad_historical_replay_participants(
        &self,
        transactions: &[HistoricalReplayTransaction],
    ) -> Vec<Participant> {
        transactions
            .iter()
            .map(|replay| {
                Participant::new(
                    replay.transaction.signer(),
                    replay.transaction.inner(),
                    self.cheats(),
                )
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
                None => participants_of(
                    &self.monad_participants_from_storage(storage, &block.body.transactions)?,
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
        let participants = transactions
            .iter()
            .map(|tx| Participant::new(tx.from(), tx, self.cheats()))
            .collect::<Vec<_>>();
        Ok((block.header().number(), block.header().parent_hash(), participants_of(&participants)))
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
                let participants = self.monad_participants_from_storage(storage, &transactions)?;
                Ok((hash, participants_of(&participants)))
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
        Ok(context_from_participants(grandparent, parent, &[], 0))
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
        current: &[Participant],
        current_tx_index: usize,
    ) -> Result<MonadChainContext, BlockchainError> {
        let (_, grandparent_hash, parent) =
            self.monad_block_participants(block.header.parent_hash)?;
        let grandparent = if grandparent_hash.is_zero() {
            MonadBlockParticipants::default()
        } else {
            self.monad_block_participants(grandparent_hash)?.2
        };
        Ok(context_from_participants(grandparent, parent, current, current_tx_index))
    }

    fn monad_context_for_mined_block(
        &self,
        block: &Block,
    ) -> Result<MonadChainContext, BlockchainError> {
        let current = self.monad_participants_from_storage(
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
        let current = self.monad_participants_from_storage(
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
