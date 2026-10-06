use crate::{state::SharedAnvilState, state_provider::AnvilStateProvider};
use alloy_consensus::{BlockHeader, transaction::TransactionMeta};
use alloy_eips::{BlockHashOrNumber, BlockId, BlockNumHash, BlockNumberOrTag};
use alloy_primitives::{Address, B256, BlockHash, BlockNumber, TxHash, TxNumber};
use alloy_rpc_types_engine::ForkchoiceState;
use reth_chain_state::{
    CanonStateNotifications, CanonStateSubscriptions, ExecutedBlock, ForkChoiceNotifications,
    ForkChoiceSubscriptions, NewCanonicalChain, PersistedBlockNotifications,
    PersistedBlockSubscriptions,
};
use reth_db_api::models::{AccountBeforeTx, BlockNumberAddress, StoredBlockBodyIndices};
use reth_ethereum::{
    chainspec::{ChainInfo, ChainSpecProvider},
    node::api::{BlockTy, HeaderTy, ReceiptTy, TxTy},
    primitives::{RecoveredBlock, SealedHeader, SealedOrRecoveredBlock, StorageEntry},
    provider::{
        BlockExecutionOutput, BlockExecutionResult, BlockSource, ExecutionOutcome, ProviderError,
        RecoveredBlockAndExecutionOutput, RocksDBProviderFactory, StaticFileProviderFactory,
        StaticFileSegment, TransactionVariant,
        providers::{
            BlockchainProvider, ProviderNodeTypes, RocksDBProvider, StaticFileProvider,
            StaticFileProviderRWRefMut,
        },
    },
    storage::{
        BalProvider, BalStoreHandle, BlockBodyIndicesProvider, BlockExecutionWriter,
        BlockHashReader, BlockIdReader, BlockNumReader, BlockReader, BlockReaderIdExt,
        CanonChainTracker, ChangeSetReader, DBProvider, DatabaseProviderFactory, HeaderProvider,
        NodePrimitivesProvider, PruneCheckpointReader, ReceiptProvider, ReceiptProviderIdExt,
        StageCheckpointReader, StateProviderBox, StateProviderFactory, StateRangeProviderFactory,
        StateRangeView, StateReader, StorageChangeSetReader, TransactionsProvider,
        errors::provider::ProviderResult,
    },
    trie::ComputedTrieData,
};
use reth_prune_types::{PruneCheckpoint, PruneSegment};
use reth_stages_types::{StageCheckpoint, StageId};
use std::{
    ops::{RangeBounds, RangeInclusive},
    sync::Arc,
    time::Instant,
};

/// The node provider: reth's [`BlockchainProvider`] with anvil state writes served on top of the
/// latest and pending state.
///
/// State lookups by block hash stay untouched because the engine executes blocks against them. The
/// block executor applies the same writes inside the next block, so execution and the state root
/// catch up with the overlay.
#[derive(Debug)]
pub struct AnvilProvider<N: ProviderNodeTypes> {
    inner: BlockchainProvider<N>,
    state: SharedAnvilState,
    slots_in_an_epoch: u64,
}

impl<N: ProviderNodeTypes> Clone for AnvilProvider<N> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            state: self.state.clone(),
            slots_in_an_epoch: self.slots_in_an_epoch,
        }
    }
}

impl<N: ProviderNodeTypes> AnvilProvider<N> {
    /// Wraps the given provider.
    pub const fn new(
        inner: BlockchainProvider<N>,
        state: SharedAnvilState,
        slots_in_an_epoch: u64,
    ) -> Self {
        Self { inner, state, slots_in_an_epoch }
    }

    /// Rewinds the canonical chain to the given header.
    ///
    /// This drops the in-memory blocks above the header and removes the persisted blocks above it,
    /// so reads see the rewound chain at once. The engine learns about the rewind when the next
    /// block builds on the header: it then reorgs its own view onto that block.
    pub fn rewind_to(&self, header: &SealedHeader<HeaderTy<N>>) -> ProviderResult<()> {
        let in_memory = self.inner.canonical_in_memory_state();
        let old: Vec<_> = in_memory
            .canonical_chain()
            .filter(|state| state.number() > header.number())
            .map(|state| state.block())
            .collect();
        let target = match in_memory.state_by_number(header.number()) {
            Some(state) => state.block(),
            None => self.executed_block_from_storage(header)?,
        };
        in_memory.update_chain(NewCanonicalChain::Reorg { new: Vec::new(), old: old.clone() });

        if self.inner.last_block_number()? > header.number() {
            let provider = self.inner.database_provider_rw()?;
            provider.remove_block_and_execution_above(header.number())?;
            provider.commit()?;
        }

        in_memory.set_canonical_head(header.clone());
        // Subscribers such as the RPC caches and the pool learn about the rewind the same way
        // they learn about a reorg.
        let reorg = NewCanonicalChain::Reorg { new: vec![target], old };
        in_memory.notify_canon_state(reorg.to_chain_notification());
        Ok(())
    }

    /// Rebuilds the executed block for a persisted canonical header.
    fn executed_block_from_storage(
        &self,
        header: &SealedHeader<HeaderTy<N>>,
    ) -> ProviderResult<ExecutedBlock<N::Primitives>> {
        let block = self
            .inner
            .recovered_block(header.hash().into(), TransactionVariant::WithHash)?
            .ok_or(ProviderError::BlockHashNotFound(header.hash()))?;
        let outcome = self.inner.get_state(header.number())?.unwrap_or_default();
        let output = BlockExecutionOutput {
            state: outcome.bundle,
            result: BlockExecutionResult {
                receipts: outcome.receipts.into_iter().next().unwrap_or_default(),
                requests: outcome.requests.into_iter().next().unwrap_or_default(),
                gas_used: header.gas_used(),
                blob_gas_used: header.blob_gas_used().unwrap_or_default(),
            },
        };
        Ok(ExecutedBlock::new(Arc::new(block), Arc::new(output), ComputedTrieData::default()))
    }

    /// Returns the canonical block `depth` blocks behind the head, or genesis.
    fn num_hash_at_depth(&self, depth: u64) -> ProviderResult<Option<BlockNumHash>> {
        let number = self.inner.best_block_number()?.saturating_sub(depth);
        Ok(self.inner.block_hash(number)?.map(|hash| BlockNumHash::new(number, hash)))
    }

    /// Returns the wrapped provider.
    pub const fn inner(&self) -> &BlockchainProvider<N> {
        &self.inner
    }

    fn overlay(&self, provider: StateProviderBox) -> StateProviderBox {
        Box::new(AnvilStateProvider::new(self.state.clone(), provider))
    }

    /// Wraps the provider when `number` is the canonical head.
    fn overlay_if_head_number(
        &self,
        number: BlockNumber,
        provider: StateProviderBox,
    ) -> ProviderResult<StateProviderBox> {
        if number == self.inner.best_block_number()? {
            return Ok(self.overlay(provider));
        }
        Ok(provider)
    }

    /// Wraps the provider when `hash` is the canonical head.
    fn overlay_if_head_hash(
        &self,
        hash: BlockHash,
        provider: StateProviderBox,
    ) -> ProviderResult<StateProviderBox> {
        if hash == self.inner.chain_info()?.best_hash {
            return Ok(self.overlay(provider));
        }
        Ok(provider)
    }
}

impl<N: ProviderNodeTypes> NodePrimitivesProvider for AnvilProvider<N> {
    type Primitives = N::Primitives;
}

impl<N: ProviderNodeTypes> BalProvider for AnvilProvider<N> {
    fn bal_store(&self) -> &BalStoreHandle {
        self.inner.bal_store()
    }
}

impl<N: ProviderNodeTypes> StateRangeProviderFactory for AnvilProvider<N> {
    fn state_range_provider(&self, state_root: B256) -> ProviderResult<Option<StateRangeView>> {
        self.inner.state_range_provider(state_root)
    }
}

impl<N: ProviderNodeTypes> DatabaseProviderFactory for AnvilProvider<N> {
    type DB = N::DB;
    type Provider = <BlockchainProvider<N> as DatabaseProviderFactory>::Provider;
    type ProviderRW = <BlockchainProvider<N> as DatabaseProviderFactory>::ProviderRW;

    fn database_provider_ro(&self) -> ProviderResult<Self::Provider> {
        self.inner.database_provider_ro()
    }

    fn database_provider_rw(&self) -> ProviderResult<Self::ProviderRW> {
        self.inner.database_provider_rw()
    }
}

impl<N: ProviderNodeTypes> StaticFileProviderFactory for AnvilProvider<N> {
    fn static_file_provider(&self) -> StaticFileProvider<Self::Primitives> {
        self.inner.static_file_provider()
    }

    fn get_static_file_writer(
        &self,
        block: BlockNumber,
        segment: StaticFileSegment,
    ) -> ProviderResult<StaticFileProviderRWRefMut<'_, Self::Primitives>> {
        self.inner.get_static_file_writer(block, segment)
    }
}

impl<N: ProviderNodeTypes> RocksDBProviderFactory for AnvilProvider<N> {
    fn rocksdb_provider(&self) -> RocksDBProvider {
        self.inner.rocksdb_provider()
    }

    fn set_pending_rocksdb_batch(&self, batch: rocksdb::WriteBatchWithTransaction<true>) {
        self.inner.set_pending_rocksdb_batch(batch)
    }

    fn commit_pending_rocksdb_batches(&self) -> ProviderResult<()> {
        self.inner.commit_pending_rocksdb_batches()
    }
}

impl<N: ProviderNodeTypes> HeaderProvider for AnvilProvider<N> {
    type Header = HeaderTy<N>;

    fn header(&self, block_hash: BlockHash) -> ProviderResult<Option<Self::Header>> {
        self.inner.header(block_hash)
    }

    fn header_by_number(&self, num: BlockNumber) -> ProviderResult<Option<Self::Header>> {
        self.inner.header_by_number(num)
    }

    fn headers_range(
        &self,
        range: impl RangeBounds<BlockNumber>,
    ) -> ProviderResult<Vec<Self::Header>> {
        self.inner.headers_range(range)
    }

    fn sealed_header(
        &self,
        number: BlockNumber,
    ) -> ProviderResult<Option<SealedHeader<Self::Header>>> {
        self.inner.sealed_header(number)
    }

    fn sealed_headers_range(
        &self,
        range: impl RangeBounds<BlockNumber>,
    ) -> ProviderResult<Vec<SealedHeader<Self::Header>>> {
        self.inner.sealed_headers_range(range)
    }

    fn sealed_headers_while(
        &self,
        range: impl RangeBounds<BlockNumber>,
        predicate: impl FnMut(&SealedHeader<Self::Header>) -> bool,
    ) -> ProviderResult<Vec<SealedHeader<Self::Header>>> {
        self.inner.sealed_headers_while(range, predicate)
    }
}

impl<N: ProviderNodeTypes> BlockHashReader for AnvilProvider<N> {
    fn block_hash(&self, number: u64) -> ProviderResult<Option<B256>> {
        self.inner.block_hash(number)
    }

    fn canonical_hashes_range(
        &self,
        start: BlockNumber,
        end: BlockNumber,
    ) -> ProviderResult<Vec<B256>> {
        self.inner.canonical_hashes_range(start, end)
    }
}

impl<N: ProviderNodeTypes> BlockNumReader for AnvilProvider<N> {
    fn chain_info(&self) -> ProviderResult<ChainInfo> {
        self.inner.chain_info()
    }

    fn best_block_number(&self) -> ProviderResult<BlockNumber> {
        self.inner.best_block_number()
    }

    fn last_block_number(&self) -> ProviderResult<BlockNumber> {
        self.inner.last_block_number()
    }

    fn earliest_block_number(&self) -> ProviderResult<BlockNumber> {
        self.inner.earliest_block_number()
    }

    fn block_number(&self, hash: B256) -> ProviderResult<Option<BlockNumber>> {
        self.inner.block_number(hash)
    }
}

impl<N: ProviderNodeTypes> BlockIdReader for AnvilProvider<N> {
    fn pending_block_num_hash(&self) -> ProviderResult<Option<BlockNumHash>> {
        self.inner.pending_block_num_hash()
    }

    /// The engine never finalizes blocks, so snapshots can rewind to any block. The `safe` and
    /// `finalized` tags follow anvil instead: one and two epochs behind the head.
    fn safe_block_num_hash(&self) -> ProviderResult<Option<BlockNumHash>> {
        self.num_hash_at_depth(self.slots_in_an_epoch)
    }

    fn finalized_block_num_hash(&self) -> ProviderResult<Option<BlockNumHash>> {
        self.num_hash_at_depth(self.slots_in_an_epoch * 2)
    }
}

impl<N: ProviderNodeTypes> BlockReader for AnvilProvider<N> {
    type Block = BlockTy<N>;

    fn find_block_by_hash(
        &self,
        hash: B256,
        source: BlockSource,
    ) -> ProviderResult<Option<Self::Block>> {
        self.inner.find_block_by_hash(hash, source)
    }

    fn find_sealed_or_recovered_block(
        &self,
        hash: B256,
        source: BlockSource,
    ) -> ProviderResult<Option<SealedOrRecoveredBlock<Self::Block>>> {
        self.inner.find_sealed_or_recovered_block(hash, source)
    }

    fn block(&self, id: BlockHashOrNumber) -> ProviderResult<Option<Self::Block>> {
        self.inner.block(id)
    }

    fn pending_block(&self) -> ProviderResult<Option<Arc<RecoveredBlock<Self::Block>>>> {
        self.inner.pending_block()
    }

    fn pending_block_and_receipts(
        &self,
    ) -> ProviderResult<Option<RecoveredBlockAndExecutionOutput<Self::Block, Self::Receipt>>> {
        self.inner.pending_block_and_receipts()
    }

    fn recovered_block(
        &self,
        id: BlockHashOrNumber,
        transaction_kind: TransactionVariant,
    ) -> ProviderResult<Option<RecoveredBlock<Self::Block>>> {
        self.inner.recovered_block(id, transaction_kind)
    }

    fn sealed_block_with_senders(
        &self,
        id: BlockHashOrNumber,
        transaction_kind: TransactionVariant,
    ) -> ProviderResult<Option<RecoveredBlock<Self::Block>>> {
        self.inner.sealed_block_with_senders(id, transaction_kind)
    }

    fn block_range(&self, range: RangeInclusive<BlockNumber>) -> ProviderResult<Vec<Self::Block>> {
        self.inner.block_range(range)
    }

    fn block_with_senders_range(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> ProviderResult<Vec<RecoveredBlock<Self::Block>>> {
        self.inner.block_with_senders_range(range)
    }

    fn recovered_block_range(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> ProviderResult<Vec<RecoveredBlock<Self::Block>>> {
        self.inner.recovered_block_range(range)
    }

    fn block_by_transaction_id(&self, id: TxNumber) -> ProviderResult<Option<BlockNumber>> {
        self.inner.block_by_transaction_id(id)
    }
}

impl<N: ProviderNodeTypes> TransactionsProvider for AnvilProvider<N> {
    type Transaction = TxTy<N>;

    fn transaction_id(&self, tx_hash: TxHash) -> ProviderResult<Option<TxNumber>> {
        self.inner.transaction_id(tx_hash)
    }

    fn transaction_by_id(&self, id: TxNumber) -> ProviderResult<Option<Self::Transaction>> {
        self.inner.transaction_by_id(id)
    }

    fn transaction_by_id_unhashed(
        &self,
        id: TxNumber,
    ) -> ProviderResult<Option<Self::Transaction>> {
        self.inner.transaction_by_id_unhashed(id)
    }

    fn transaction_by_hash(&self, hash: TxHash) -> ProviderResult<Option<Self::Transaction>> {
        self.inner.transaction_by_hash(hash)
    }

    fn transaction_by_hash_with_meta(
        &self,
        tx_hash: TxHash,
    ) -> ProviderResult<Option<(Self::Transaction, TransactionMeta)>> {
        self.inner.transaction_by_hash_with_meta(tx_hash)
    }

    fn transactions_by_block(
        &self,
        id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<Self::Transaction>>> {
        self.inner.transactions_by_block(id)
    }

    fn transactions_by_block_range(
        &self,
        range: impl RangeBounds<BlockNumber>,
    ) -> ProviderResult<Vec<Vec<Self::Transaction>>> {
        self.inner.transactions_by_block_range(range)
    }

    fn transactions_by_tx_range(
        &self,
        range: impl RangeBounds<TxNumber>,
    ) -> ProviderResult<Vec<Self::Transaction>> {
        self.inner.transactions_by_tx_range(range)
    }

    fn senders_by_tx_range(
        &self,
        range: impl RangeBounds<TxNumber>,
    ) -> ProviderResult<Vec<Address>> {
        self.inner.senders_by_tx_range(range)
    }

    fn transaction_sender(&self, id: TxNumber) -> ProviderResult<Option<Address>> {
        self.inner.transaction_sender(id)
    }
}

impl<N: ProviderNodeTypes> ReceiptProvider for AnvilProvider<N> {
    type Receipt = ReceiptTy<N>;

    fn receipt(&self, id: TxNumber) -> ProviderResult<Option<Self::Receipt>> {
        self.inner.receipt(id)
    }

    fn receipt_by_hash(&self, hash: TxHash) -> ProviderResult<Option<Self::Receipt>> {
        self.inner.receipt_by_hash(hash)
    }

    fn receipts_by_block(
        &self,
        block: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<Self::Receipt>>> {
        self.inner.receipts_by_block(block)
    }

    fn receipts_by_tx_range(
        &self,
        range: impl RangeBounds<TxNumber>,
    ) -> ProviderResult<Vec<Self::Receipt>> {
        self.inner.receipts_by_tx_range(range)
    }

    fn receipts_by_block_range(
        &self,
        block_range: RangeInclusive<BlockNumber>,
    ) -> ProviderResult<Vec<Vec<Self::Receipt>>> {
        self.inner.receipts_by_block_range(block_range)
    }
}

impl<N: ProviderNodeTypes> ReceiptProviderIdExt for AnvilProvider<N> {
    fn receipts_by_block_id(&self, block: BlockId) -> ProviderResult<Option<Vec<Self::Receipt>>> {
        self.inner.receipts_by_block_id(block)
    }
}

impl<N: ProviderNodeTypes> BlockBodyIndicesProvider for AnvilProvider<N> {
    fn block_body_indices(
        &self,
        number: BlockNumber,
    ) -> ProviderResult<Option<StoredBlockBodyIndices>> {
        self.inner.block_body_indices(number)
    }

    fn block_body_indices_range(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> ProviderResult<Vec<StoredBlockBodyIndices>> {
        self.inner.block_body_indices_range(range)
    }
}

impl<N: ProviderNodeTypes> StageCheckpointReader for AnvilProvider<N> {
    fn get_stage_checkpoint(&self, id: StageId) -> ProviderResult<Option<StageCheckpoint>> {
        self.inner.get_stage_checkpoint(id)
    }

    fn get_stage_checkpoint_progress(&self, id: StageId) -> ProviderResult<Option<Vec<u8>>> {
        self.inner.get_stage_checkpoint_progress(id)
    }

    fn get_all_checkpoints(&self) -> ProviderResult<Vec<(String, StageCheckpoint)>> {
        self.inner.get_all_checkpoints()
    }
}

impl<N: ProviderNodeTypes> PruneCheckpointReader for AnvilProvider<N> {
    fn get_prune_checkpoint(
        &self,
        segment: PruneSegment,
    ) -> ProviderResult<Option<PruneCheckpoint>> {
        self.inner.get_prune_checkpoint(segment)
    }

    fn get_prune_checkpoints(&self) -> ProviderResult<Vec<(PruneSegment, PruneCheckpoint)>> {
        self.inner.get_prune_checkpoints()
    }
}

impl<N: ProviderNodeTypes> ChainSpecProvider for AnvilProvider<N> {
    type ChainSpec = N::ChainSpec;

    fn chain_spec(&self) -> Arc<N::ChainSpec> {
        self.inner.chain_spec()
    }
}

impl<N: ProviderNodeTypes> StateProviderFactory for AnvilProvider<N> {
    type Primitives = N::Primitives;

    fn latest(&self) -> ProviderResult<StateProviderBox> {
        Ok(self.overlay(self.inner.latest()?))
    }

    fn state_with_block_appended(
        &self,
        parent_hash: BlockHash,
        block: ExecutedBlock<N::Primitives>,
    ) -> ProviderResult<StateProviderBox> {
        self.inner.state_with_block_appended(parent_hash, block)
    }

    fn state_by_block_number_or_tag(
        &self,
        number_or_tag: BlockNumberOrTag,
    ) -> ProviderResult<StateProviderBox> {
        match number_or_tag {
            BlockNumberOrTag::Latest => self.latest(),
            BlockNumberOrTag::Pending => self.pending(),
            BlockNumberOrTag::Number(number) => {
                let provider = self.inner.state_by_block_number_or_tag(number_or_tag)?;
                self.overlay_if_head_number(number, provider)
            }
            BlockNumberOrTag::Finalized | BlockNumberOrTag::Safe | BlockNumberOrTag::Earliest => {
                self.inner.state_by_block_number_or_tag(number_or_tag)
            }
        }
    }

    fn history_by_block_number(&self, block: BlockNumber) -> ProviderResult<StateProviderBox> {
        let provider = self.inner.history_by_block_number(block)?;
        self.overlay_if_head_number(block, provider)
    }

    fn history_by_block_hash(&self, block: BlockHash) -> ProviderResult<StateProviderBox> {
        let provider = self.inner.history_by_block_hash(block)?;
        self.overlay_if_head_hash(block, provider)
    }

    fn state_by_block_hash(&self, block: BlockHash) -> ProviderResult<StateProviderBox> {
        self.inner.state_by_block_hash(block)
    }

    fn pending(&self) -> ProviderResult<StateProviderBox> {
        Ok(self.overlay(self.inner.pending()?))
    }

    fn pending_state_by_hash(&self, block_hash: B256) -> ProviderResult<Option<StateProviderBox>> {
        self.inner.pending_state_by_hash(block_hash)
    }

    fn maybe_pending(&self) -> ProviderResult<Option<StateProviderBox>> {
        Ok(self.inner.maybe_pending()?.map(|provider| self.overlay(provider)))
    }
}

impl<N: ProviderNodeTypes> CanonChainTracker for AnvilProvider<N> {
    type Header = HeaderTy<N>;

    fn on_forkchoice_update_received(&self, update: &ForkchoiceState) {
        self.inner.on_forkchoice_update_received(update)
    }

    fn last_received_update_timestamp(&self) -> Option<Instant> {
        self.inner.last_received_update_timestamp()
    }

    fn set_canonical_head(&self, header: SealedHeader<Self::Header>) {
        self.inner.set_canonical_head(header)
    }

    fn set_safe(&self, header: SealedHeader<Self::Header>) {
        self.inner.set_safe(header)
    }

    fn set_finalized(&self, header: SealedHeader<Self::Header>) {
        self.inner.set_finalized(header)
    }
}

impl<N: ProviderNodeTypes> BlockReaderIdExt for AnvilProvider<N> {
    fn block_by_id(&self, id: BlockId) -> ProviderResult<Option<Self::Block>> {
        self.inner.block_by_id(id)
    }

    fn header_by_number_or_tag(
        &self,
        id: BlockNumberOrTag,
    ) -> ProviderResult<Option<Self::Header>> {
        self.inner.header_by_number_or_tag(id)
    }

    fn sealed_header_by_number_or_tag(
        &self,
        id: BlockNumberOrTag,
    ) -> ProviderResult<Option<SealedHeader<Self::Header>>> {
        self.inner.sealed_header_by_number_or_tag(id)
    }

    fn sealed_header_by_id(
        &self,
        id: BlockId,
    ) -> ProviderResult<Option<SealedHeader<Self::Header>>> {
        self.inner.sealed_header_by_id(id)
    }

    fn header_by_id(&self, id: BlockId) -> ProviderResult<Option<Self::Header>> {
        self.inner.header_by_id(id)
    }
}

impl<N: ProviderNodeTypes> CanonStateSubscriptions for AnvilProvider<N> {
    type Primitives = N::Primitives;

    fn subscribe_to_canonical_state(&self) -> CanonStateNotifications<Self::Primitives> {
        self.inner.subscribe_to_canonical_state()
    }
}

impl<N: ProviderNodeTypes> ForkChoiceSubscriptions for AnvilProvider<N> {
    type Header = HeaderTy<N>;

    fn subscribe_safe_block(&self) -> ForkChoiceNotifications<Self::Header> {
        self.inner.subscribe_safe_block()
    }

    fn subscribe_finalized_block(&self) -> ForkChoiceNotifications<Self::Header> {
        self.inner.subscribe_finalized_block()
    }
}

impl<N: ProviderNodeTypes> PersistedBlockSubscriptions for AnvilProvider<N> {
    fn subscribe_persisted_block(&self) -> PersistedBlockNotifications {
        self.inner.subscribe_persisted_block()
    }
}

impl<N: ProviderNodeTypes> StorageChangeSetReader for AnvilProvider<N> {
    fn storage_changeset(
        &self,
        block_number: BlockNumber,
    ) -> ProviderResult<Vec<(BlockNumberAddress, StorageEntry)>> {
        self.inner.storage_changeset(block_number)
    }

    fn get_storage_before_block(
        &self,
        block_number: BlockNumber,
        address: Address,
        storage_key: B256,
    ) -> ProviderResult<Option<StorageEntry>> {
        self.inner.get_storage_before_block(block_number, address, storage_key)
    }

    fn storage_changesets_range(
        &self,
        range: impl RangeBounds<BlockNumber>,
    ) -> ProviderResult<Vec<(BlockNumberAddress, StorageEntry)>> {
        self.inner.storage_changesets_range(range)
    }
}

impl<N: ProviderNodeTypes> ChangeSetReader for AnvilProvider<N> {
    fn account_block_changeset(
        &self,
        block_number: BlockNumber,
    ) -> ProviderResult<Vec<AccountBeforeTx>> {
        self.inner.account_block_changeset(block_number)
    }

    fn get_account_before_block(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> ProviderResult<Option<AccountBeforeTx>> {
        self.inner.get_account_before_block(block_number, address)
    }

    fn account_changesets_range(
        &self,
        range: impl RangeBounds<BlockNumber>,
    ) -> ProviderResult<Vec<(BlockNumber, AccountBeforeTx)>> {
        self.inner.account_changesets_range(range)
    }
}

impl<N: ProviderNodeTypes> StateReader for AnvilProvider<N> {
    type Receipt = ReceiptTy<N>;

    fn get_state(
        &self,
        block: BlockNumber,
    ) -> ProviderResult<Option<ExecutionOutcome<Self::Receipt>>> {
        self.inner.get_state(block)
    }
}
