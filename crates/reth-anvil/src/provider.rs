use crate::{
    fork::{
        AnvilPrimitives, ForkOf, ForkStateProvider, LocalWrites, decode_remote_tx_number,
        remote_tx_number,
    },
    miner::RewindHooks,
    state::{AnvilState, SharedAnvilState},
    state_dump::{AccountDump, SerializableAccountRecord},
    state_provider::AnvilStateProvider,
};
use alloy_consensus::{BlockHeader, transaction::TransactionMeta};
use alloy_eips::{BlockHashOrNumber, BlockId, BlockNumHash, BlockNumberOrTag};
use alloy_primitives::{
    Address, B256, BlockHash, BlockNumber, Bytes, StorageKey, TxHash, TxNumber,
};
use alloy_rpc_types_engine::ForkchoiceState;
use eyre::Result;
use parking_lot::RwLock;
use reth_chain_state::{
    CanonStateNotifications, CanonStateSubscriptions, CanonicalInMemoryState, ExecutedBlock,
    ForkChoiceNotifications, ForkChoiceSubscriptions, NewCanonicalChain,
    PersistedBlockNotifications, PersistedBlockSubscriptions,
};
use reth_db_api::{
    cursor::{DbCursorRO, DbCursorRW, DbDupCursorRO},
    models::{
        AccountBeforeTx, BlockNumberAddress, ShardedKey, StoredBlockBodyIndices,
        storage_sharded_key::StorageShardedKey,
    },
    tables,
    transaction::{DbTx, DbTxMut},
};
use reth_ethereum::{
    chainspec::{ChainInfo, ChainSpecProvider, EthChainSpec},
    node::api::{BlockTy, HeaderTy, ReceiptTy, TxTy},
    primitives::{
        BlockBody, RecoveredBlock, SealedBlock, SealedHeader, SealedOrRecoveredBlock, StorageEntry,
        header::HeaderMut,
    },
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
        BalProvider, BalStoreHandle, BlockBodyIndicesProvider, BlockHashReader, BlockIdReader,
        BlockNumReader, BlockReader, BlockReaderIdExt, CanonChainTracker, ChangeSetReader,
        DBProvider, DatabaseProviderFactory, HeaderProvider, HistoryWriter, NodePrimitivesProvider,
        PruneCheckpointReader, ReceiptProvider, ReceiptProviderIdExt, StageCheckpointReader,
        StateProviderBox, StateProviderFactory, StateRangeProviderFactory, StateRangeView,
        StateReader, StorageChangeSetReader, TransactionsProvider,
        errors::provider::ProviderResult,
    },
    trie::ComputedTrieData,
};
use reth_prune_types::{PruneCheckpoint, PruneSegment};
use reth_stages_types::{StageCheckpoint, StageId};
use std::{
    collections::BTreeMap,
    ops::{RangeBounds, RangeInclusive},
    sync::Arc,
    time::Instant,
};

/// The node types the provider supports: primitives with a fork network, so remote fork data
/// converts into the local types.
pub trait AnvilNodeTypes: ProviderNodeTypes<Primitives: AnvilPrimitives> {}

impl<N: ProviderNodeTypes<Primitives: AnvilPrimitives>> AnvilNodeTypes for N {}

/// The fork backend of the given node types.
pub type NodeFork<N> = ForkOf<<N as reth_ethereum::node::api::NodeTypes>::Primitives>;

/// The node provider: reth's [`BlockchainProvider`] with anvil state writes served on top of the
/// latest and pending state, and with a remote fork below the local chain.
///
/// State lookups by block hash stay untouched by the anvil state writes because the engine
/// executes blocks against them. The block executor applies the same writes inside the next block,
/// so execution and the state root catch up with the overlay.
///
/// When the node forks a remote chain, blocks below the fork block and the state keys the local
/// chain has not written come from the remote endpoint.
#[derive(Debug)]
pub struct AnvilProvider<N: AnvilNodeTypes> {
    inner: BlockchainProvider<N>,
    state: SharedAnvilState,
    slots_in_an_epoch: u64,
    fork: Option<Arc<NodeFork<N>>>,
    /// The blocks a rewind in progress removes, for the reorg notification once it settles.
    rewinding: Arc<RwLock<Vec<ExecutedBlock<N::Primitives>>>>,
}

impl<N: AnvilNodeTypes> Clone for AnvilProvider<N> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            state: self.state.clone(),
            slots_in_an_epoch: self.slots_in_an_epoch,
            fork: self.fork.clone(),
            rewinding: self.rewinding.clone(),
        }
    }
}

impl<N: AnvilNodeTypes> AnvilProvider<N> {
    /// Wraps the given provider.
    pub fn new(
        inner: BlockchainProvider<N>,
        state: SharedAnvilState,
        slots_in_an_epoch: u64,
        fork: Option<Arc<NodeFork<N>>>,
    ) -> Self {
        Self { inner, state, slots_in_an_epoch, fork, rewinding: Arc::default() }
    }

    /// Returns the sealed genesis header.
    fn genesis_header(&self) -> ProviderResult<SealedHeader<HeaderTy<N>>> {
        let number = self.inner.chain_spec().genesis_header().number();
        self.inner.sealed_header(number)?.ok_or(ProviderError::HeaderNotFound(number.into()))
    }

    /// Returns the canonical block with the given number as an executed block, from memory or
    /// from the database.
    fn executed_block(&self, number: BlockNumber) -> ProviderResult<ExecutedBlock<N::Primitives>> {
        if let Some(state) = self.inner.canonical_in_memory_state().state_by_number(number) {
            return Ok(state.block());
        }
        let header = self
            .inner
            .sealed_header(number)?
            .ok_or(ProviderError::HeaderNotFound(number.into()))?;
        let block = self
            .inner
            .recovered_block(header.hash().into(), TransactionVariant::WithHash)?
            .ok_or(ProviderError::BlockHashNotFound(header.hash()))?;
        let outcome = self.get_state(number)?.unwrap_or_default();
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

    fn remote_for(&self, number: BlockNumber) -> Option<&Arc<NodeFork<N>>> {
        self.fork.as_ref().filter(|fork| fork.predates_fork(number))
    }

    /// Returns the fork when the block with the given hash is a remote block below the fork block.
    fn remote_for_hash(&self, hash: BlockHash) -> ProviderResult<Option<&Arc<NodeFork<N>>>> {
        let Some(fork) = &self.fork else { return Ok(None) };
        if self.inner.block_number(hash)?.is_some() {
            return Ok(None);
        }
        Ok(fork
            .block_number_by_hash(hash)?
            .filter(|number| fork.predates_fork(*number))
            .map(|_| fork))
    }

    /// Returns the fork when the block id names a remote block below the fork block.
    fn remote_for_id(&self, id: BlockHashOrNumber) -> ProviderResult<Option<&Arc<NodeFork<N>>>> {
        match id {
            BlockHashOrNumber::Hash(hash) => self.remote_for_hash(hash),
            BlockHashOrNumber::Number(number) => Ok(self.remote_for(number)),
        }
    }

    /// Layers the fork under the local state at `block`.
    fn with_fork(
        &self,
        provider: StateProviderBox,
        block: BlockNumber,
    ) -> ProviderResult<StateProviderBox> {
        match &self.fork {
            Some(fork) => {
                let writes = LocalWriteIndex {
                    db: self.inner.database_provider_ro()?,
                    in_memory: self.inner.canonical_in_memory_state(),
                };
                Ok(Box::new(ForkStateProvider::new(
                    fork.clone(),
                    Some((provider, Box::new(writes))),
                    block,
                )?))
            }
            None => Ok(provider),
        }
    }

    /// Returns the remote state at `block`, which is below the fork block.
    fn remote_state(
        &self,
        fork: &Arc<NodeFork<N>>,
        block: BlockNumber,
    ) -> ProviderResult<StateProviderBox> {
        Ok(Box::new(ForkStateProvider::new(fork.clone(), None, block)?))
    }

    /// Splits a block range into the part below the fork block and the local part.
    fn split_range(
        &self,
        range: impl RangeBounds<BlockNumber>,
    ) -> (Option<RangeInclusive<BlockNumber>>, RangeInclusive<BlockNumber>) {
        let start = match range.start_bound() {
            std::ops::Bound::Included(start) => *start,
            std::ops::Bound::Excluded(start) => start.saturating_add(1),
            std::ops::Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            std::ops::Bound::Included(end) => *end,
            std::ops::Bound::Excluded(end) => end.saturating_sub(1),
            std::ops::Bound::Unbounded => u64::MAX,
        };
        match &self.fork {
            Some(fork) if fork.predates_fork(start) => {
                let fork_block = fork.block_number();
                let remote_end = end.min(fork_block - 1);
                (Some(start..=remote_end), fork_block.max(start)..=end)
            }
            _ => (None, start..=end),
        }
    }

    /// Copies the remote state read since the last block into the local database.
    ///
    /// The engine validates blocks against the local database, so the remote accounts, slots, and
    /// bytecodes the block builder read must be local before the engine executes the block. The
    /// copies get the fork block as their history entry, so reads treat them as local state from
    /// the fork block on. Local values win: a key the local chain already wrote is left alone.
    pub fn materialize_fork_reads(&self) -> ProviderResult<()> {
        let Some(fork) = &self.fork else { return Ok(()) };
        let reads = fork.take_reads();
        if reads.is_empty() {
            return Ok(());
        }
        let fork_block = fork.block_number();
        let provider = self.inner.database_provider_rw()?;
        let tx = provider.tx_ref();

        let mut account_history = Vec::new();
        for (address, account) in reads.accounts {
            if tx.get::<tables::PlainAccountState>(address)?.is_some() {
                continue;
            }
            tx.put::<tables::PlainAccountState>(address, account)?;
            account_history.push((address, [fork_block]));
        }
        for (hash, code) in reads.codes {
            if tx.get::<tables::Bytecodes>(hash)?.is_none() {
                tx.put::<tables::Bytecodes>(hash, code)?;
            }
        }
        let mut storage_history = Vec::new();
        let mut cursor = tx.cursor_dup_write::<tables::PlainStorageState>()?;
        for ((address, slot), value) in reads.storage {
            if cursor.seek_by_key_subkey(address, slot)?.is_some_and(|entry| entry.key == slot) {
                continue;
            }
            cursor.upsert(address, &StorageEntry::new(slot, value))?;
            storage_history.push(((address, slot), [fork_block]));
        }
        drop(cursor);
        provider.insert_account_history_index(account_history)?;
        provider.insert_storage_history_index(storage_history)?;
        provider.commit()?;
        Ok(())
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

    /// Wraps the state at block `number` with the anvil writes made while that block was the
    /// head: the pending writes for the current head, and for an older block the writes the next
    /// block applied. Anvil writes into the head state directly, so a reader at that block sees
    /// them.
    fn overlay_for_block(
        &self,
        number: BlockNumber,
        provider: StateProviderBox,
    ) -> ProviderResult<StateProviderBox> {
        if number == self.inner.best_block_number()? {
            return Ok(self.overlay(provider));
        }
        let writes = self.state.read().writes_for_block(number + 1).cloned();
        match writes {
            Some(writes) if !writes.is_empty() => Ok(Box::new(AnvilStateProvider::new(
                Arc::new(RwLock::new(AnvilState::from_writes(&writes))),
                provider,
            ))),
            _ => Ok(provider),
        }
    }
}

impl<N: AnvilNodeTypes> NodePrimitivesProvider for AnvilProvider<N> {
    type Primitives = N::Primitives;
}

impl<N: AnvilNodeTypes> BalProvider for AnvilProvider<N> {
    fn bal_store(&self) -> &BalStoreHandle {
        self.inner.bal_store()
    }
}

impl<N: AnvilNodeTypes> StateRangeProviderFactory for AnvilProvider<N> {
    fn state_range_provider(&self, state_root: B256) -> ProviderResult<Option<StateRangeView>> {
        self.inner.state_range_provider(state_root)
    }
}

impl<N: AnvilNodeTypes> DatabaseProviderFactory for AnvilProvider<N> {
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

impl<N: AnvilNodeTypes> StaticFileProviderFactory for AnvilProvider<N> {
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

impl<N: AnvilNodeTypes> RocksDBProviderFactory for AnvilProvider<N> {
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

impl<N: AnvilNodeTypes> HeaderProvider for AnvilProvider<N> {
    type Header = HeaderTy<N>;

    fn header(&self, block_hash: BlockHash) -> ProviderResult<Option<Self::Header>> {
        if let Some(header) = self.inner.header(block_hash)? {
            return Ok(Some(header));
        }
        match &self.fork {
            Some(fork) => fork.header_by_hash(block_hash),
            None => Ok(None),
        }
    }

    fn header_by_number(&self, num: BlockNumber) -> ProviderResult<Option<Self::Header>> {
        match self.remote_for(num) {
            Some(fork) => fork.header_by_number(num),
            None => self.inner.header_by_number(num),
        }
    }

    fn headers_range(
        &self,
        range: impl RangeBounds<BlockNumber>,
    ) -> ProviderResult<Vec<Self::Header>> {
        let (remote, local) = self.split_range(range);
        let mut headers = Vec::new();
        if let (Some(remote), Some(fork)) = (remote, &self.fork) {
            for number in remote {
                headers.extend(fork.header_by_number(number)?);
            }
        }
        if !local.is_empty() {
            headers.extend(self.inner.headers_range(local)?);
        }
        Ok(headers)
    }

    fn sealed_header(
        &self,
        number: BlockNumber,
    ) -> ProviderResult<Option<SealedHeader<Self::Header>>> {
        match self.remote_for(number) {
            Some(fork) => fork.sealed_header(number),
            None => self.inner.sealed_header(number),
        }
    }

    fn sealed_headers_range(
        &self,
        range: impl RangeBounds<BlockNumber>,
    ) -> ProviderResult<Vec<SealedHeader<Self::Header>>> {
        let (remote, local) = self.split_range(range);
        let mut headers = Vec::new();
        if let (Some(remote), Some(fork)) = (remote, &self.fork) {
            for number in remote {
                headers.extend(fork.sealed_header(number)?);
            }
        }
        if !local.is_empty() {
            headers.extend(self.inner.sealed_headers_range(local)?);
        }
        Ok(headers)
    }

    fn sealed_headers_while(
        &self,
        range: impl RangeBounds<BlockNumber>,
        mut predicate: impl FnMut(&SealedHeader<Self::Header>) -> bool,
    ) -> ProviderResult<Vec<SealedHeader<Self::Header>>> {
        let (remote, local) = self.split_range(range);
        let mut headers = Vec::new();
        if let (Some(remote), Some(fork)) = (remote, &self.fork) {
            for number in remote {
                let Some(header) = fork.sealed_header(number)? else { return Ok(headers) };
                if !predicate(&header) {
                    return Ok(headers);
                }
                headers.push(header);
            }
        }
        if !local.is_empty() {
            headers.extend(self.inner.sealed_headers_while(local, predicate)?);
        }
        Ok(headers)
    }
}

impl<N: AnvilNodeTypes> BlockHashReader for AnvilProvider<N> {
    fn block_hash(&self, number: u64) -> ProviderResult<Option<B256>> {
        match self.remote_for(number) {
            Some(fork) => fork.block_hash_by_number(number),
            None => self.inner.block_hash(number),
        }
    }

    fn canonical_hashes_range(
        &self,
        start: BlockNumber,
        end: BlockNumber,
    ) -> ProviderResult<Vec<B256>> {
        let (remote, local) = self.split_range(start..end);
        let mut hashes = Vec::new();
        if let (Some(remote), Some(fork)) = (remote, &self.fork) {
            for number in remote {
                hashes.extend(fork.block_hash_by_number(number)?);
            }
        }
        if !local.is_empty() {
            hashes.extend(self.inner.canonical_hashes_range(*local.start(), *local.end() + 1)?);
        }
        Ok(hashes)
    }
}

impl<N: AnvilNodeTypes> BlockNumReader for AnvilProvider<N> {
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
        if self.fork.is_some() {
            return Ok(0);
        }
        self.inner.earliest_block_number()
    }

    fn block_number(&self, hash: B256) -> ProviderResult<Option<BlockNumber>> {
        if let Some(number) = self.inner.block_number(hash)? {
            return Ok(Some(number));
        }
        match &self.fork {
            Some(fork) => fork.block_number_by_hash(hash),
            None => Ok(None),
        }
    }
}

impl<N: AnvilNodeTypes> BlockIdReader for AnvilProvider<N> {
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

impl<N: AnvilNodeTypes> BlockReader for AnvilProvider<N> {
    type Block = BlockTy<N>;

    fn find_block_by_hash(
        &self,
        hash: B256,
        source: BlockSource,
    ) -> ProviderResult<Option<Self::Block>> {
        if let Some(block) = self.inner.find_block_by_hash(hash, source)? {
            return Ok(Some(block));
        }
        match self.remote_for_hash(hash)? {
            Some(fork) => Ok(fork.block_by_hash(hash)?.map(|block| (*block).clone().into_block())),
            None => Ok(None),
        }
    }

    fn find_sealed_or_recovered_block(
        &self,
        hash: B256,
        source: BlockSource,
    ) -> ProviderResult<Option<SealedOrRecoveredBlock<Self::Block>>> {
        if let Some(block) = self.inner.find_sealed_or_recovered_block(hash, source)? {
            return Ok(Some(block));
        }
        match self.remote_for_hash(hash)? {
            Some(fork) => Ok(fork
                .recovered_block(hash.into())?
                .map(|block| SealedOrRecoveredBlock::Recovered(Arc::new(block)))),
            None => Ok(None),
        }
    }

    fn block(&self, id: BlockHashOrNumber) -> ProviderResult<Option<Self::Block>> {
        match self.remote_for_id(id)? {
            Some(fork) => Ok(fork.block(id)?.map(|block| (*block).clone().into_block())),
            None => self.inner.block(id),
        }
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
        match self.remote_for_id(id)? {
            Some(fork) => fork.recovered_block(id),
            None => self.inner.recovered_block(id, transaction_kind),
        }
    }

    fn sealed_block_with_senders(
        &self,
        id: BlockHashOrNumber,
        transaction_kind: TransactionVariant,
    ) -> ProviderResult<Option<RecoveredBlock<Self::Block>>> {
        match self.remote_for_id(id)? {
            Some(fork) => fork.recovered_block(id),
            None => self.inner.sealed_block_with_senders(id, transaction_kind),
        }
    }

    fn block_range(&self, range: RangeInclusive<BlockNumber>) -> ProviderResult<Vec<Self::Block>> {
        let (remote, local) = self.split_range(range);
        let mut blocks = Vec::new();
        if let (Some(remote), Some(fork)) = (remote, &self.fork) {
            for number in remote {
                blocks.extend(
                    fork.block_by_number(number)?.map(|block| (*block).clone().into_block()),
                );
            }
        }
        if !local.is_empty() {
            blocks.extend(self.inner.block_range(local)?);
        }
        Ok(blocks)
    }

    fn block_with_senders_range(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> ProviderResult<Vec<RecoveredBlock<Self::Block>>> {
        self.recovered_block_range(range)
    }

    fn recovered_block_range(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> ProviderResult<Vec<RecoveredBlock<Self::Block>>> {
        let (remote, local) = self.split_range(range);
        let mut blocks = Vec::new();
        if let (Some(remote), Some(fork)) = (remote, &self.fork) {
            for number in remote {
                blocks.extend(fork.recovered_block(number.into())?);
            }
        }
        if !local.is_empty() {
            blocks.extend(self.inner.recovered_block_range(local)?);
        }
        Ok(blocks)
    }

    fn block_by_transaction_id(&self, id: TxNumber) -> ProviderResult<Option<BlockNumber>> {
        if let Some((block, _)) = decode_remote_tx_number(id) {
            return Ok(Some(block));
        }
        self.inner.block_by_transaction_id(id)
    }
}

impl<N: AnvilNodeTypes> TransactionsProvider for AnvilProvider<N> {
    type Transaction = TxTy<N>;

    fn transaction_id(&self, tx_hash: TxHash) -> ProviderResult<Option<TxNumber>> {
        if let Some(id) = self.inner.transaction_id(tx_hash)? {
            return Ok(Some(id));
        }
        match &self.fork {
            Some(fork) => Ok(fork
                .transaction_by_hash_with_meta(tx_hash)?
                .map(|(_, meta)| remote_tx_number(meta.block_number, meta.index))),
            None => Ok(None),
        }
    }

    fn transaction_by_id(&self, id: TxNumber) -> ProviderResult<Option<Self::Transaction>> {
        match (decode_remote_tx_number(id), &self.fork) {
            (Some((block, index)), Some(fork)) => Ok(fork
                .block_by_number(block)?
                .and_then(|block| block.body().transactions().get(index as usize).cloned())),
            _ => self.inner.transaction_by_id(id),
        }
    }

    fn transaction_by_id_unhashed(
        &self,
        id: TxNumber,
    ) -> ProviderResult<Option<Self::Transaction>> {
        if decode_remote_tx_number(id).is_some() {
            return self.transaction_by_id(id);
        }
        self.inner.transaction_by_id_unhashed(id)
    }

    fn transaction_by_hash(&self, hash: TxHash) -> ProviderResult<Option<Self::Transaction>> {
        if let Some(tx) = self.inner.transaction_by_hash(hash)? {
            return Ok(Some(tx));
        }
        match &self.fork {
            Some(fork) => Ok(fork.transaction_by_hash_with_meta(hash)?.map(|(tx, _)| tx)),
            None => Ok(None),
        }
    }

    fn transaction_by_hash_with_meta(
        &self,
        tx_hash: TxHash,
    ) -> ProviderResult<Option<(Self::Transaction, TransactionMeta)>> {
        if let Some(tx) = self.inner.transaction_by_hash_with_meta(tx_hash)? {
            return Ok(Some(tx));
        }
        match &self.fork {
            Some(fork) => fork.transaction_by_hash_with_meta(tx_hash),
            None => Ok(None),
        }
    }

    fn transactions_by_block(
        &self,
        id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<Self::Transaction>>> {
        match self.remote_for_id(id)? {
            Some(fork) => Ok(fork.block(id)?.map(|block| block.body().transactions().to_vec())),
            None => self.inner.transactions_by_block(id),
        }
    }

    fn transactions_by_block_range(
        &self,
        range: impl RangeBounds<BlockNumber>,
    ) -> ProviderResult<Vec<Vec<Self::Transaction>>> {
        let (remote, local) = self.split_range(range);
        let mut txs = Vec::new();
        if let (Some(remote), Some(fork)) = (remote, &self.fork) {
            for number in remote {
                txs.extend(
                    fork.block_by_number(number)?.map(|block| block.body().transactions().to_vec()),
                );
            }
        }
        if !local.is_empty() {
            txs.extend(self.inner.transactions_by_block_range(local)?);
        }
        Ok(txs)
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
        match (decode_remote_tx_number(id), &self.fork) {
            (Some((block, index)), Some(fork)) => Ok(fork
                .recovered_block(block.into())?
                .and_then(|block| block.senders().get(index as usize).copied())),
            _ => self.inner.transaction_sender(id),
        }
    }
}

impl<N: AnvilNodeTypes> ReceiptProvider for AnvilProvider<N> {
    type Receipt = ReceiptTy<N>;

    fn receipt(&self, id: TxNumber) -> ProviderResult<Option<Self::Receipt>> {
        match (decode_remote_tx_number(id), &self.fork) {
            (Some((block, index)), Some(fork)) => {
                let Some(hash) = fork.block_hash_by_number(block)? else { return Ok(None) };
                Ok(fork
                    .receipts_by_block(hash)?
                    .and_then(|receipts| receipts.get(index as usize).cloned()))
            }
            _ => self.inner.receipt(id),
        }
    }

    fn receipt_by_hash(&self, hash: TxHash) -> ProviderResult<Option<Self::Receipt>> {
        if let Some(receipt) = self.inner.receipt_by_hash(hash)? {
            return Ok(Some(receipt));
        }
        match &self.fork {
            Some(fork) => fork.receipt_by_hash(hash),
            None => Ok(None),
        }
    }

    fn receipts_by_block(
        &self,
        block: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<Self::Receipt>>> {
        match self.remote_for_id(block)? {
            Some(fork) => {
                let Some(hash) = fork.block(block)?.map(|block| block.hash()) else {
                    return Ok(None);
                };
                Ok(fork.receipts_by_block(hash)?.map(|receipts| (*receipts).clone()))
            }
            None => self.inner.receipts_by_block(block),
        }
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
        let (remote, local) = self.split_range(block_range);
        let mut receipts = Vec::new();
        if let Some(remote) = remote {
            for number in remote {
                receipts.extend(self.receipts_by_block(number.into())?);
            }
        }
        if !local.is_empty() {
            receipts.extend(self.inner.receipts_by_block_range(local)?);
        }
        Ok(receipts)
    }
}

impl<N: AnvilNodeTypes> ReceiptProviderIdExt for AnvilProvider<N> {
    fn receipts_by_block_id(&self, block: BlockId) -> ProviderResult<Option<Vec<Self::Receipt>>> {
        match block {
            BlockId::Hash(hash) => self.receipts_by_block(hash.block_hash.into()),
            BlockId::Number(number) => match self.convert_block_number(number)? {
                Some(number) => self.receipts_by_block(number.into()),
                None => Ok(None),
            },
        }
    }
}

impl<N: AnvilNodeTypes> BlockBodyIndicesProvider for AnvilProvider<N> {
    fn block_body_indices(
        &self,
        number: BlockNumber,
    ) -> ProviderResult<Option<StoredBlockBodyIndices>> {
        match self.remote_for(number) {
            Some(fork) => Ok(fork.block_by_number(number)?.map(|block| StoredBlockBodyIndices {
                first_tx_num: remote_tx_number(number, 0),
                tx_count: block.body().transactions().len() as u64,
            })),
            None => self.inner.block_body_indices(number),
        }
    }

    fn block_body_indices_range(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> ProviderResult<Vec<StoredBlockBodyIndices>> {
        let (remote, local) = self.split_range(range);
        let mut indices = Vec::new();
        if let Some(remote) = remote {
            for number in remote {
                indices.extend(self.block_body_indices(number)?);
            }
        }
        if !local.is_empty() {
            indices.extend(self.inner.block_body_indices_range(local)?);
        }
        Ok(indices)
    }
}

impl<N: AnvilNodeTypes> StageCheckpointReader for AnvilProvider<N> {
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

impl<N: AnvilNodeTypes> PruneCheckpointReader for AnvilProvider<N> {
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

impl<N: AnvilNodeTypes> ChainSpecProvider for AnvilProvider<N> {
    type ChainSpec = N::ChainSpec;

    fn chain_spec(&self) -> Arc<N::ChainSpec> {
        self.inner.chain_spec()
    }
}

impl<N: AnvilNodeTypes> StateProviderFactory for AnvilProvider<N> {
    type Primitives = N::Primitives;

    fn latest(&self) -> ProviderResult<StateProviderBox> {
        let head = self.inner.best_block_number()?;
        Ok(self.overlay(self.with_fork(self.inner.latest()?, head)?))
    }

    fn state_with_block_appended(
        &self,
        parent_hash: BlockHash,
        block: ExecutedBlock<N::Primitives>,
    ) -> ProviderResult<StateProviderBox> {
        let number = block.recovered_block().number();
        let provider = self.inner.state_with_block_appended(parent_hash, block)?;
        self.with_fork(provider, number)
    }

    fn state_by_block_number_or_tag(
        &self,
        number_or_tag: BlockNumberOrTag,
    ) -> ProviderResult<StateProviderBox> {
        match number_or_tag {
            BlockNumberOrTag::Latest => self.latest(),
            BlockNumberOrTag::Pending => self.pending(),
            BlockNumberOrTag::Number(number) => self.history_by_block_number(number),
            BlockNumberOrTag::Earliest => {
                self.history_by_block_number(self.earliest_block_number()?)
            }
            BlockNumberOrTag::Finalized | BlockNumberOrTag::Safe => {
                let number = self
                    .convert_block_number(number_or_tag)?
                    .ok_or(ProviderError::FinalizedBlockNotFound)?;
                self.history_by_block_number(number)
            }
        }
    }

    fn history_by_block_number(&self, block: BlockNumber) -> ProviderResult<StateProviderBox> {
        if let Some(fork) = self.remote_for(block) {
            return self.remote_state(fork, block);
        }
        let provider = self.with_fork(self.inner.history_by_block_number(block)?, block)?;
        self.overlay_for_block(block, provider)
    }

    fn history_by_block_hash(&self, block: BlockHash) -> ProviderResult<StateProviderBox> {
        if let Some(fork) = self.remote_for_hash(block)? {
            let number =
                fork.block_number_by_hash(block)?.ok_or(ProviderError::BlockHashNotFound(block))?;
            return self.remote_state(fork, number);
        }
        let number =
            self.inner.block_number(block)?.ok_or(ProviderError::BlockHashNotFound(block))?;
        let provider = self.with_fork(self.inner.history_by_block_hash(block)?, number)?;
        self.overlay_for_block(number, provider)
    }

    fn state_by_block_hash(&self, block: BlockHash) -> ProviderResult<StateProviderBox> {
        if let Some(fork) = self.remote_for_hash(block)? {
            let number =
                fork.block_number_by_hash(block)?.ok_or(ProviderError::BlockHashNotFound(block))?;
            return self.remote_state(fork, number);
        }
        // The engine loads an unwind target with the state at its parent, and hashes the
        // target's writes against it. Genesis has no parent, and no writes: its own state does.
        let chain_spec = self.inner.chain_spec();
        let genesis = chain_spec.genesis_header();
        if self.fork.is_none() && block == genesis.parent_hash() {
            return self.history_by_block_number(genesis.number());
        }
        let number =
            self.inner.block_number(block)?.ok_or(ProviderError::BlockHashNotFound(block))?;
        self.with_fork(self.inner.state_by_block_hash(block)?, number)
    }

    fn pending(&self) -> ProviderResult<StateProviderBox> {
        let pending = self.inner.best_block_number()? + 1;
        Ok(self.overlay(self.with_fork(self.inner.pending()?, pending)?))
    }

    fn pending_state_by_hash(&self, block_hash: B256) -> ProviderResult<Option<StateProviderBox>> {
        let Some(provider) = self.inner.pending_state_by_hash(block_hash)? else { return Ok(None) };
        let pending = self.inner.best_block_number()? + 1;
        Ok(Some(self.with_fork(provider, pending)?))
    }

    fn maybe_pending(&self) -> ProviderResult<Option<StateProviderBox>> {
        let Some(provider) = self.inner.maybe_pending()? else { return Ok(None) };
        let pending = self.inner.best_block_number()? + 1;
        Ok(Some(self.overlay(self.with_fork(provider, pending)?)))
    }
}

impl<N: AnvilNodeTypes> CanonChainTracker for AnvilProvider<N> {
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

impl<N: AnvilNodeTypes> BlockReaderIdExt for AnvilProvider<N> {
    fn block_by_id(&self, id: BlockId) -> ProviderResult<Option<Self::Block>> {
        match id {
            BlockId::Hash(hash) => self.block(hash.block_hash.into()),
            BlockId::Number(number) => match self.convert_block_number(number)? {
                Some(number) => self.block(number.into()),
                None => Ok(None),
            },
        }
    }

    fn header_by_number_or_tag(
        &self,
        id: BlockNumberOrTag,
    ) -> ProviderResult<Option<Self::Header>> {
        Ok(self.sealed_header_by_number_or_tag(id)?.map(|header| header.into_header()))
    }

    fn sealed_header_by_number_or_tag(
        &self,
        id: BlockNumberOrTag,
    ) -> ProviderResult<Option<SealedHeader<Self::Header>>> {
        match id {
            BlockNumberOrTag::Number(number) if self.remote_for(number).is_some() => {
                self.sealed_header(number)
            }
            BlockNumberOrTag::Earliest if self.fork.is_some() => self.sealed_header(0),
            _ => self.inner.sealed_header_by_number_or_tag(id),
        }
    }

    fn sealed_header_by_id(
        &self,
        id: BlockId,
    ) -> ProviderResult<Option<SealedHeader<Self::Header>>> {
        match id {
            BlockId::Hash(hash) => {
                let hash = hash.block_hash;
                if let Some(header) = self.inner.sealed_header_by_id(id)? {
                    return Ok(Some(header));
                }
                match &self.fork {
                    Some(fork) => {
                        Ok(fork.block_by_hash(hash)?.map(|block| block.sealed_header().clone()))
                    }
                    None => Ok(None),
                }
            }
            BlockId::Number(number) => self.sealed_header_by_number_or_tag(number),
        }
    }

    fn header_by_id(&self, id: BlockId) -> ProviderResult<Option<Self::Header>> {
        Ok(self.sealed_header_by_id(id)?.map(|header| header.into_header()))
    }
}

impl<N: AnvilNodeTypes> CanonStateSubscriptions for AnvilProvider<N> {
    type Primitives = N::Primitives;

    fn subscribe_to_canonical_state(&self) -> CanonStateNotifications<Self::Primitives> {
        self.inner.subscribe_to_canonical_state()
    }
}

impl<N: AnvilNodeTypes> ForkChoiceSubscriptions for AnvilProvider<N> {
    type Header = HeaderTy<N>;

    fn subscribe_safe_block(&self) -> ForkChoiceNotifications<Self::Header> {
        self.inner.subscribe_safe_block()
    }

    fn subscribe_finalized_block(&self) -> ForkChoiceNotifications<Self::Header> {
        self.inner.subscribe_finalized_block()
    }
}

impl<N: AnvilNodeTypes> PersistedBlockSubscriptions for AnvilProvider<N> {
    fn subscribe_persisted_block(&self) -> PersistedBlockNotifications {
        self.inner.subscribe_persisted_block()
    }
}

impl<N: AnvilNodeTypes> StorageChangeSetReader for AnvilProvider<N> {
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

impl<N: AnvilNodeTypes> ChangeSetReader for AnvilProvider<N> {
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

impl<N: AnvilNodeTypes> StateReader for AnvilProvider<N> {
    type Receipt = ReceiptTy<N>;

    fn get_state(
        &self,
        block: BlockNumber,
    ) -> ProviderResult<Option<ExecutionOutcome<Self::Receipt>>> {
        if let Some(state) = self.inner.get_state(block)? {
            return Ok(Some(state));
        }
        // Genesis has no execution outcome. The engine loads the outcome of a canonical ancestor
        // when it unwinds onto it, so genesis gets an empty one.
        let genesis = self.inner.chain_spec().genesis_header().number();
        Ok((block == genesis)
            .then(|| ExecutionOutcome { first_block: block, ..Default::default() }))
    }
}

/// Answers which state keys the local chain wrote, from reth's history index for persisted blocks
/// and the bundle states of the in-memory blocks.
struct LocalWriteIndex<DB, N: reth_ethereum::primitives::NodePrimitives> {
    db: DB,
    in_memory: CanonicalInMemoryState<N>,
}

impl<DB: DBProvider, N: reth_ethereum::primitives::NodePrimitives> LocalWriteIndex<DB, N> {
    /// Returns the in-memory blocks at or below `block`, including the pending block.
    fn bundles(&self, block: u64) -> Vec<Arc<revm::database::BundleState>> {
        let mut bundles: Vec<_> = self
            .in_memory
            .canonical_chain()
            .filter(|state| state.number() <= block)
            .map(|state| Arc::new(state.block_ref().execution_output.state.clone()))
            .collect();
        if let Some(pending) = self.in_memory.pending_state()
            && pending.number() <= block
        {
            bundles.push(Arc::new(pending.block_ref().execution_output.state.clone()));
        }
        bundles
    }

    /// Returns the first block in which the persisted history saw `address` change.
    fn first_account_change(&self, address: &Address) -> ProviderResult<Option<u64>> {
        let mut cursor = self.db.tx_ref().cursor_read::<tables::AccountsHistory>()?;
        Ok(cursor
            .seek(ShardedKey::new(*address, 0))?
            .filter(|(key, _)| key.key == *address)
            .and_then(|(_, blocks)| blocks.0.min()))
    }

    /// Returns the first block in which the persisted history saw `slot` of `address` change.
    fn first_slot_change(
        &self,
        address: &Address,
        slot: &StorageKey,
    ) -> ProviderResult<Option<u64>> {
        let mut cursor = self.db.tx_ref().cursor_read::<tables::StoragesHistory>()?;
        Ok(cursor
            .seek(StorageShardedKey::new(*address, *slot, 0))?
            .filter(|(key, _)| key.address == *address && key.sharded_key.key == *slot)
            .and_then(|(_, blocks)| blocks.0.min()))
    }
}

impl<DB: DBProvider + Send + Sync, N: reth_ethereum::primitives::NodePrimitives> LocalWrites
    for LocalWriteIndex<DB, N>
{
    fn account_is_local(&self, address: &Address, block: u64) -> ProviderResult<bool> {
        if self.first_account_change(address)?.is_some_and(|first| first <= block) {
            return Ok(true);
        }
        Ok(self.bundles(block).iter().any(|bundle| {
            bundle
                .state
                .get(address)
                .is_some_and(|account| account.is_info_changed() || account.was_destroyed())
        }))
    }

    fn slot_is_local(
        &self,
        address: &Address,
        slot: &StorageKey,
        block: u64,
    ) -> ProviderResult<bool> {
        if self.first_slot_change(address, slot)?.is_some_and(|first| first <= block) {
            return Ok(true);
        }
        let slot = alloy_primitives::U256::from_be_bytes(slot.0);
        Ok(self.bundles(block).iter().any(|bundle| {
            bundle.state.get(address).is_some_and(|account| {
                account.was_destroyed()
                    || account.storage.get(&slot).is_some_and(|value| value.is_changed())
            })
        }))
    }
}

impl<N: AnvilNodeTypes> AccountDump for AnvilProvider<N> {
    fn dump_accounts(&self) -> ProviderResult<BTreeMap<Address, SerializableAccountRecord>> {
        let provider = self.inner.database_provider_ro()?;
        let tx = provider.tx_ref();
        let mut accounts = BTreeMap::new();

        // The persisted plain state.
        let mut cursor = tx.cursor_read::<tables::PlainAccountState>()?;
        for entry in cursor.walk(None)? {
            let (address, account) = entry?;
            let code = match account.bytecode_hash {
                Some(hash) => tx
                    .get::<tables::Bytecodes>(hash)?
                    .map(|code| code.original_bytes())
                    .unwrap_or_default(),
                None => Bytes::new(),
            };
            accounts.insert(
                address,
                SerializableAccountRecord {
                    nonce: account.nonce,
                    balance: account.balance,
                    code,
                    storage: BTreeMap::new(),
                },
            );
        }
        let mut cursor = tx.cursor_dup_read::<tables::PlainStorageState>()?;
        for entry in cursor.walk(None)? {
            let (address, slot) = entry?;
            if slot.value.is_zero() {
                continue;
            }
            if let Some(account) = accounts.get_mut(&address) {
                account.storage.insert(slot.key, slot.value.into());
            }
        }

        // The in-memory blocks, oldest first.
        let mut states: Vec<_> = self.inner.canonical_in_memory_state().canonical_chain().collect();
        states.reverse();
        for state in states {
            let bundle = &state.block_ref().execution_output.state;
            for (address, account) in &bundle.state {
                let Some(info) = &account.info else {
                    accounts.remove(address);
                    continue;
                };
                let record = accounts.entry(*address).or_default();
                record.nonce = info.nonce;
                record.balance = info.balance;
                if let Some(code) = info.code.as_ref().filter(|code| !code.is_empty()) {
                    record.code = code.original_bytes();
                } else if let Some(code) = bundle.bytecode(&info.code_hash) {
                    record.code = code.original_bytes();
                } else if info.is_empty_code_hash() {
                    record.code = Bytes::new();
                }
                if account.was_destroyed() {
                    record.storage.clear();
                }
                for (slot, value) in &account.storage {
                    let key = B256::from(*slot);
                    if value.present_value.is_zero() {
                        record.storage.remove(&key);
                    } else {
                        record.storage.insert(key, value.present_value.into());
                    }
                }
            }
        }
        Ok(accounts)
    }
}

impl<N: AnvilNodeTypes> RewindHooks for AnvilProvider<N>
where
    HeaderTy<N>: HeaderMut,
{
    /// The engine loads an unwind target with the state at its parent and hashes the target's
    /// writes against it. Genesis has no parent state, so a pending block under the parent hash,
    /// anchored on genesis, stands in; the unwind's chain update keeps it, and `settle` clears it.
    fn prepare(&self, number: u64) -> Result<()> {
        // The engine's unwind tells nobody about the removed blocks; the reorg notification
        // in `settle` does, and needs them before the persistence task removes them.
        let best = self.inner.best_block_number()?;
        let removed = (number + 1..=best)
            .map(|number| self.executed_block(number))
            .collect::<Result<Vec<_>, _>>()?;
        *self.rewinding.write() = removed;

        let genesis = self.genesis_header()?;
        if number != genesis.number() {
            return Ok(());
        }
        let (block, senders) = self
            .inner
            .recovered_block(genesis.hash().into(), TransactionVariant::WithHash)?
            .ok_or(ProviderError::BlockHashNotFound(genesis.hash()))?
            .split_sealed();
        let (header, body) = block.split_sealed_header_body();
        let mut header = header.into_header();
        header.set_parent_hash(genesis.hash());
        header.set_number(genesis.number() + 1);
        let header = SealedHeader::new(header, genesis.parent_hash());
        let stand_in =
            RecoveredBlock::new_sealed(SealedBlock::from_sealed_parts(header, body), senders);
        let output = BlockExecutionOutput {
            state: Default::default(),
            result: BlockExecutionResult {
                receipts: Vec::new(),
                requests: Default::default(),
                gas_used: 0,
                blob_gas_used: 0,
            },
        };
        self.inner.canonical_in_memory_state().set_pending_block(ExecutedBlock::new(
            Arc::new(stand_in),
            Arc::new(output),
            ComputedTrieData::default(),
        ));
        Ok(())
    }

    /// A rewind to genesis leaves genesis as an in-memory block anchored on its missing parent
    /// state, and the stand-in pending block; both go, and genesis is served from the database
    /// again.
    fn settle(&self, number: u64) -> Result<bool> {
        // The database, not the in-memory head, which the unwind moved already.
        if self.inner.database_provider_ro()?.last_block_number()? > number {
            return Ok(false);
        }
        let in_memory = self.inner.canonical_in_memory_state();
        let genesis = self.genesis_header()?;
        if number == genesis.number() {
            in_memory.clear_state();
            in_memory.set_canonical_head(genesis);
        }
        // Subscribers such as the RPC caches, the pool, and the anvil state learn about the
        // rewind the same way they learn about a reorg.
        let old = std::mem::take(&mut *self.rewinding.write());
        if !old.is_empty() {
            let target = self.executed_block(number)?;
            let reorg = NewCanonicalChain::Reorg { new: vec![target], old };
            in_memory.notify_canon_state(reorg.to_chain_notification());
        }
        Ok(true)
    }
}
