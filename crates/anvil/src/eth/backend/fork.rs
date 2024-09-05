//! Support for forking off another client

use crate::eth::{backend::db::Db, error::BlockchainError, pool::transactions::PoolTransaction};
use alloy_consensus::Account;
use alloy_eips::eip2930::AccessListResult;
use alloy_primitives::{Address, Bytes, StorageValue, B256, U256};
use alloy_provider::{
    ext::{DebugApi, TraceApi},
    Provider,
};
use alloy_rpc_types::{
    request::TransactionRequest,
    trace::{
        geth::{GethDebugTracingOptions, GethTrace},
        parity::LocalizedTransactionTrace as Trace,
    },
    Block, BlockId, BlockNumberOrTag as BlockNumber, BlockTransactions,
    EIP1186AccountProofResponse, FeeHistory, Filter, Log, Transaction,
};
use alloy_serde::WithOtherFields;
use alloy_transport::TransportError;
use anvil_core::eth::transaction::{convert_to_anvil_receipt, ReceiptResponse};
use foundry_common::provider::{ProviderBuilder, RetryProvider};
use parking_lot::{
    lock_api::{RwLockReadGuard, RwLockWriteGuard},
    RawRwLock, RwLock,
};
use revm::primitives::BlobExcessGasAndPrice;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::RwLock as AsyncRwLock;

/// Represents a fork of a remote client
///
/// This type contains a subset of the [`EthApi`](crate::eth::EthApi) functions but will exclusively
/// fetch the requested data from the remote client, if it wasn't already fetched.
#[derive(Clone, Debug)]
pub struct ClientFork {
    /// Contains the cached data
    pub storage: Arc<RwLock<ForkedStorage>>,
    /// contains the info how the fork is configured
    // Wrapping this in a lock, ensures we can update this on the fly via additional custom RPC
    // endpoints
    pub config: Arc<RwLock<ClientForkConfig>>,
    /// This also holds a handle to the underlying database
    pub database: Arc<AsyncRwLock<Box<dyn Db>>>,
}

impl ClientFork {
    /// Creates a new instance of the fork
    pub fn new(config: ClientForkConfig, database: Arc<AsyncRwLock<Box<dyn Db>>>) -> Self {
        Self { storage: Default::default(), config: Arc::new(RwLock::new(config)), database }
    }

    /// Reset the fork to a fresh forked state, and optionally update the fork config
    pub async fn reset(
        &self,
        url: Option<String>,
        block_number: impl Into<BlockId>,
    ) -> Result<(), BlockchainError> {
        let block_number = block_number.into();
        {
            self.database
                .write()
                .await
                .maybe_reset(url.clone(), block_number)
                .map_err(BlockchainError::Internal)?;
        }

        if let Some(url) = url {
            self.config.write().update_url(url)?;
            let override_chain_id = self.config.read().override_chain_id;
            let chain_id = if let Some(chain_id) = override_chain_id {
                chain_id
            } else {
                self.provider().get_chain_id().await?
            };
            self.config.write().chain_id = chain_id;
        }

        let provider = self.provider();
        let block = provider
            .get_block(block_number, false.into())
            .await?
            .ok_or(BlockchainError::BlockNotFound)?;
        let block_hash = block.header.hash.ok_or(BlockchainError::BlockNotFound)?;
        let timestamp = block.header.timestamp;
        let base_fee = block.header.base_fee_per_gas;
        let total_difficulty = block.header.total_difficulty.unwrap_or_default();

        let number = block.header.number.ok_or(BlockchainError::BlockNotFound)?;
        self.config.write().update_block(number, block_hash, timestamp, base_fee, total_difficulty);

        self.clear_cached_storage();

        self.database.write().await.insert_block_hash(U256::from(number), block_hash);

        Ok(())
    }

    /// Removes all data cached from previous responses
    pub fn clear_cached_storage(&self) {
        self.storage.write().clear()
    }

    /// Returns true whether the block predates the fork
    pub fn predates_fork(&self, block: u64) -> bool {
        block < self.block_number()
    }

    /// Returns true whether the block predates the fork _or_ is the same block as the fork
    pub fn predates_fork_inclusive(&self, block: u64) -> bool {
        block <= self.block_number()
    }

    pub fn timestamp(&self) -> u64 {
        self.config.read().timestamp
    }

    pub fn block_number(&self) -> u64 {
        self.config.read().block_number
    }

    /// Returns the transaction hash we forked off of, if any.
    pub fn transaction_hash(&self) -> Option<B256> {
        self.config.read().transaction_hash
    }

    pub fn total_difficulty(&self) -> U256 {
        self.config.read().total_difficulty
    }

    pub fn base_fee(&self) -> Option<u128> {
        self.config.read().base_fee
    }

    pub fn block_hash(&self) -> B256 {
        self.config.read().block_hash
    }

    pub fn eth_rpc_url(&self) -> String {
        self.config.read().eth_rpc_url.clone()
    }

    pub fn chain_id(&self) -> u64 {
        self.config.read().chain_id
    }

    fn provider(&self) -> Arc<RetryProvider> {
        self.config.read().provider.clone()
    }

    fn storage_read(&self) -> RwLockReadGuard<'_, RawRwLock, ForkedStorage> {
        self.storage.read()
    }

    fn storage_write(&self) -> RwLockWriteGuard<'_, RawRwLock, ForkedStorage> {
        self.storage.write()
    }

    /// Returns the fee history  `eth_feeHistory`
    pub async fn fee_history(
        &self,
        block_count: u64,
        newest_block: BlockNumber,
        reward_percentiles: &[f64],
    ) -> Result<FeeHistory, TransportError> {
        self.provider().get_fee_history(block_count, newest_block, reward_percentiles).await
    }

    /// Sends `eth_getProof`
    pub async fn get_proof(
        &self,
        address: Address,
        keys: Vec<B256>,
        block_number: Option<BlockId>,
    ) -> Result<EIP1186AccountProofResponse, TransportError> {
        self.provider().get_proof(address, keys).block_id(block_number.unwrap_or_default()).await
    }

    /// Sends `eth_call`
    pub async fn call(
        &self,
        request: &WithOtherFields<TransactionRequest>,
        block: Option<BlockNumber>,
    ) -> Result<Bytes, TransportError> {
        let block = block.unwrap_or(BlockNumber::Latest);
        let res = self.provider().call(request).block(block.into()).await?;

        Ok(res)
    }

    /// Sends `eth_call`
    pub async fn estimate_gas(
        &self,
        request: &WithOtherFields<TransactionRequest>,
        block: Option<BlockNumber>,
    ) -> Result<u128, TransportError> {
        let block = block.unwrap_or_default();
        let res = self.provider().estimate_gas(request).block(block.into()).await?;

        Ok(res)
    }

    /// Sends `eth_createAccessList`
    pub async fn create_access_list(
        &self,
        request: &WithOtherFields<TransactionRequest>,
        block: Option<BlockNumber>,
    ) -> Result<AccessListResult, TransportError> {
        self.provider().create_access_list(request).block_id(block.unwrap_or_default().into()).await
    }

    pub async fn storage_at(
        &self,
        address: Address,
        index: U256,
        number: Option<BlockNumber>,
    ) -> Result<StorageValue, TransportError> {
        self.provider()
            .get_storage_at(address, index)
            .block_id(number.unwrap_or_default().into())
            .await
    }

    pub async fn logs(&self, filter: &Filter) -> Result<Vec<Log>, TransportError> {
        if let Some(logs) = self.storage_read().logs.get(filter).cloned() {
            return Ok(logs);
        }

        let logs = self.provider().get_logs(filter).await?;

        let mut storage = self.storage_write();
        storage.logs.insert(filter.clone(), logs.clone());
        Ok(logs)
    }

    pub async fn get_code(
        &self,
        address: Address,
        blocknumber: u64,
    ) -> Result<Bytes, TransportError> {
        trace!(target: "backend::fork", "get_code={:?}", address);
        if let Some(code) = self.storage_read().code_at.get(&(address, blocknumber)).cloned() {
            return Ok(code);
        }

        let block_id = BlockId::number(blocknumber);

        let code = self.provider().get_code_at(address).block_id(block_id).await?;

        let mut storage = self.storage_write();
        storage.code_at.insert((address, blocknumber), code.clone().0.into());

        Ok(code)
    }

    pub async fn get_balance(
        &self,
        address: Address,
        blocknumber: u64,
    ) -> Result<U256, TransportError> {
        trace!(target: "backend::fork", "get_balance={:?}", address);
        self.provider().get_balance(address).block_id(blocknumber.into()).await
    }

    pub async fn get_nonce(&self, address: Address, block: u64) -> Result<u64, TransportError> {
        trace!(target: "backend::fork", "get_nonce={:?}", address);
        self.provider().get_transaction_count(address).block_id(block.into()).await
    }

    pub async fn get_account(
        &self,
        address: Address,
        blocknumber: u64,
    ) -> Result<Account, TransportError> {
        trace!(target: "backend::fork", "get_account={:?}", address);
        self.provider().get_account(address).block_id(blocknumber.into()).await
    }

    pub async fn transaction_by_block_number_and_index(
        &self,
        number: u64,
        index: usize,
    ) -> Result<Option<WithOtherFields<Transaction>>, TransportError> {
        if let Some(block) = self.block_by_number(number).await? {
            match block.transactions {
                BlockTransactions::Full(txs) => {
                    if let Some(tx) = txs.get(index) {
                        return Ok(Some(WithOtherFields::new(tx.clone())));
                    }
                }
                BlockTransactions::Hashes(hashes) => {
                    if let Some(tx_hash) = hashes.get(index) {
                        return self.transaction_by_hash(*tx_hash).await;
                    }
                }
                // TODO(evalir): Is it possible to reach this case? Should we support it
                BlockTransactions::Uncle => panic!("Uncles not supported"),
            }
        }
        Ok(None)
    }

    pub async fn transaction_by_block_hash_and_index(
        &self,
        hash: B256,
        index: usize,
    ) -> Result<Option<WithOtherFields<Transaction>>, TransportError> {
        if let Some(block) = self.block_by_hash(hash).await? {
            match block.transactions {
                BlockTransactions::Full(txs) => {
                    if let Some(tx) = txs.get(index) {
                        return Ok(Some(WithOtherFields::new(tx.clone())));
                    }
                }
                BlockTransactions::Hashes(hashes) => {
                    if let Some(tx_hash) = hashes.get(index) {
                        return self.transaction_by_hash(*tx_hash).await;
                    }
                }
                // TODO(evalir): Is it possible to reach this case? Should we support it
                BlockTransactions::Uncle => panic!("Uncles not supported"),
            }
        }
        Ok(None)
    }

    pub async fn transaction_by_hash(
        &self,
        hash: B256,
    ) -> Result<Option<WithOtherFields<Transaction>>, TransportError> {
        trace!(target: "backend::fork", "transaction_by_hash={:?}", hash);
        if let tx @ Some(_) = self.storage_read().transactions.get(&hash).cloned() {
            return Ok(tx);
        }

        let tx = self.provider().get_transaction_by_hash(hash).await?;
        if let Some(tx) = tx.clone() {
            let mut storage = self.storage_write();
            storage.transactions.insert(hash, tx);
        }
        Ok(tx)
    }

    pub async fn trace_transaction(&self, hash: B256) -> Result<Vec<Trace>, TransportError> {
        if let Some(traces) = self.storage_read().transaction_traces.get(&hash).cloned() {
            return Ok(traces);
        }

        let traces = self.provider().trace_transaction(hash).await?.into_iter().collect::<Vec<_>>();

        let mut storage = self.storage_write();
        storage.transaction_traces.insert(hash, traces.clone());

        Ok(traces)
    }

    pub async fn debug_trace_transaction(
        &self,
        hash: B256,
        opts: GethDebugTracingOptions,
    ) -> Result<GethTrace, TransportError> {
        if let Some(traces) = self.storage_read().geth_transaction_traces.get(&hash).cloned() {
            return Ok(traces);
        }

        let trace = self.provider().debug_trace_transaction(hash, opts).await?;

        let mut storage = self.storage_write();
        storage.geth_transaction_traces.insert(hash, trace.clone());

        Ok(trace)
    }

    pub async fn trace_block(&self, number: u64) -> Result<Vec<Trace>, TransportError> {
        if let Some(traces) = self.storage_read().block_traces.get(&number).cloned() {
            return Ok(traces);
        }

        let traces =
            self.provider().trace_block(number.into()).await?.into_iter().collect::<Vec<_>>();

        let mut storage = self.storage_write();
        storage.block_traces.insert(number, traces.clone());

        Ok(traces)
    }

    pub async fn transaction_receipt(
        &self,
        hash: B256,
    ) -> Result<Option<ReceiptResponse>, BlockchainError> {
        if let Some(receipt) = self.storage_read().transaction_receipts.get(&hash).cloned() {
            return Ok(Some(receipt));
        }

        if let Some(receipt) = self.provider().get_transaction_receipt(hash).await? {
            let receipt =
                convert_to_anvil_receipt(receipt).ok_or(BlockchainError::FailedToDecodeReceipt)?;
            let mut storage = self.storage_write();
            storage.transaction_receipts.insert(hash, receipt.clone());
            return Ok(Some(receipt));
        }

        Ok(None)
    }

    pub async fn block_receipts(
        &self,
        number: u64,
    ) -> Result<Option<Vec<ReceiptResponse>>, BlockchainError> {
        if let receipts @ Some(_) = self.storage_read().block_receipts.get(&number).cloned() {
            return Ok(receipts);
        }

        // TODO Needs to be removed.
        // Since alloy doesn't indicate in the result whether the block exists,
        // this is being temporarily implemented in anvil.
        if self.predates_fork_inclusive(number) {
            let receipts = self.provider().get_block_receipts(BlockId::from(number)).await?;
            let receipts = receipts
                .map(|r| {
                    r.into_iter()
                        .map(|r| {
                            convert_to_anvil_receipt(r)
                                .ok_or(BlockchainError::FailedToDecodeReceipt)
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?;

            if let Some(receipts) = receipts.clone() {
                let mut storage = self.storage_write();
                storage.block_receipts.insert(number, receipts);
            }

            return Ok(receipts);
        }

        Ok(None)
    }

    pub async fn block_by_hash(&self, hash: B256) -> Result<Option<Block>, TransportError> {
        if let Some(mut block) = self.storage_read().blocks.get(&hash).cloned() {
            block.transactions.convert_to_hashes();
            return Ok(Some(block));
        }

        Ok(self.fetch_full_block(hash).await?.map(|mut b| {
            b.transactions.convert_to_hashes();
            b
        }))
    }

    pub async fn block_by_hash_full(&self, hash: B256) -> Result<Option<Block>, TransportError> {
        if let Some(block) = self.storage_read().blocks.get(&hash).cloned() {
            return Ok(Some(self.convert_to_full_block(block)));
        }
        self.fetch_full_block(hash).await
    }

    pub async fn block_by_number(
        &self,
        block_number: u64,
    ) -> Result<Option<Block>, TransportError> {
        if let Some(mut block) = self
            .storage_read()
            .hashes
            .get(&block_number)
            .and_then(|hash| self.storage_read().blocks.get(hash).cloned())
        {
            block.transactions.convert_to_hashes();
            return Ok(Some(block));
        }

        let mut block = self.fetch_full_block(block_number).await?;
        if let Some(block) = &mut block {
            block.transactions.convert_to_hashes();
        }
        Ok(block)
    }

    pub async fn block_by_number_full(
        &self,
        block_number: u64,
    ) -> Result<Option<Block>, TransportError> {
        if let Some(block) = self
            .storage_read()
            .hashes
            .get(&block_number)
            .copied()
            .and_then(|hash| self.storage_read().blocks.get(&hash).cloned())
        {
            return Ok(Some(self.convert_to_full_block(block)));
        }

        self.fetch_full_block(block_number).await
    }

    async fn fetch_full_block(
        &self,
        block_id: impl Into<BlockId>,
    ) -> Result<Option<Block>, TransportError> {
        if let Some(block) = self.provider().get_block(block_id.into(), true.into()).await? {
            let hash = block.header.hash.unwrap();
            let block_number = block.header.number.unwrap();
            let mut storage = self.storage_write();
            // also insert all transactions
            let block_txs = match block.clone().transactions {
                BlockTransactions::Full(txs) => txs,
                _ => vec![],
            };
            storage
                .transactions
                .extend(block_txs.iter().map(|tx| (tx.hash, WithOtherFields::new(tx.clone()))));
            storage.hashes.insert(block_number, hash);
            storage.blocks.insert(hash, block.clone());
            return Ok(Some(block));
        }

        Ok(None)
    }

    pub async fn uncle_by_block_hash_and_index(
        &self,
        hash: B256,
        index: usize,
    ) -> Result<Option<Block>, TransportError> {
        if let Some(block) = self.block_by_hash(hash).await? {
            return self.uncles_by_block_and_index(block, index).await;
        }
        Ok(None)
    }

    pub async fn uncle_by_block_number_and_index(
        &self,
        number: u64,
        index: usize,
    ) -> Result<Option<Block>, TransportError> {
        if let Some(block) = self.block_by_number(number).await? {
            return self.uncles_by_block_and_index(block, index).await;
        }
        Ok(None)
    }

    async fn uncles_by_block_and_index(
        &self,
        block: Block,
        index: usize,
    ) -> Result<Option<Block>, TransportError> {
        let block_hash = block.header.hash.expect("Missing block hash");
        let block_number = block.header.number.expect("Missing block number");
        if let Some(uncles) = self.storage_read().uncles.get(&block_hash) {
            return Ok(uncles.get(index).cloned());
        }

        let mut uncles = Vec::with_capacity(block.uncles.len());
        for (uncle_idx, _) in block.uncles.iter().enumerate() {
            let uncle =
                match self.provider().get_uncle(block_number.into(), uncle_idx as u64).await? {
                    Some(u) => u,
                    None => return Ok(None),
                };
            uncles.push(uncle);
        }
        self.storage_write().uncles.insert(block_hash, uncles.clone());
        Ok(uncles.get(index).cloned())
    }

    /// Converts a block of hashes into a full block
    fn convert_to_full_block(&self, block: Block) -> Block {
        let storage = self.storage.read();
        let block_txs_len = match block.transactions {
            BlockTransactions::Full(ref txs) => txs.len(),
            BlockTransactions::Hashes(ref hashes) => hashes.len(),
            // TODO: Should this be supported at all?
            BlockTransactions::Uncle => 0,
        };
        let mut transactions = Vec::with_capacity(block_txs_len);
        for tx in block.transactions.hashes() {
            if let Some(tx) = storage.transactions.get(&tx).cloned() {
                transactions.push(tx.inner);
            }
        }
        // TODO: fix once blocks have generic transactions
        block.into_full_block(transactions)
    }
}

/// Contains all fork metadata
#[derive(Clone, Debug)]
pub struct ClientForkConfig {
    pub eth_rpc_url: String,
    /// The block number of the forked block
    pub block_number: u64,
    /// The hash of the forked block
    pub block_hash: B256,
    /// The transaction hash we forked off of, if any.
    pub transaction_hash: Option<B256>,
    // TODO make provider agnostic
    pub provider: Arc<RetryProvider>,
    pub chain_id: u64,
    pub override_chain_id: Option<u64>,
    /// The timestamp for the forked block
    pub timestamp: u64,
    /// The basefee of the forked block
    pub base_fee: Option<u128>,
    /// Blob gas used of the forked block
    pub blob_gas_used: Option<u128>,
    /// Blob excess gas and price of the forked block
    pub blob_excess_gas_and_price: Option<BlobExcessGasAndPrice>,
    /// request timeout
    pub timeout: Duration,
    /// request retries for spurious networks
    pub retries: u32,
    /// request retries for spurious networks
    pub backoff: Duration,
    /// available CUPS
    pub compute_units_per_second: u64,
    /// total difficulty of the chain until this block
    pub total_difficulty: U256,
    /// Transactions to force include in the forked chain
    pub force_transactions: Option<Vec<PoolTransaction>>,
}

impl ClientForkConfig {
    /// Updates the provider URL
    ///
    /// # Errors
    ///
    /// This will fail if no new provider could be established (erroneous URL)
    fn update_url(&mut self, url: String) -> Result<(), BlockchainError> {
        // let interval = self.provider.get_interval();
        self.provider = Arc::new(
            ProviderBuilder::new(url.as_str())
                .timeout(self.timeout)
                // .timeout_retry(self.retries)
                .max_retry(self.retries)
                .initial_backoff(self.backoff.as_millis() as u64)
                .compute_units_per_second(self.compute_units_per_second)
                .build()
                .map_err(|_| BlockchainError::InvalidUrl(url.clone()))?, // .interval(interval),
        );
        trace!(target: "fork", "Updated rpc url  {}", url);
        self.eth_rpc_url = url;
        Ok(())
    }
    /// Updates the block forked off `(block number, block hash, timestamp)`
    pub fn update_block(
        &mut self,
        block_number: u64,
        block_hash: B256,
        timestamp: u64,
        base_fee: Option<u128>,
        total_difficulty: U256,
    ) {
        self.block_number = block_number;
        self.block_hash = block_hash;
        self.timestamp = timestamp;
        self.base_fee = base_fee;
        self.total_difficulty = total_difficulty;
        trace!(target: "fork", "Updated block number={} hash={:?}", block_number, block_hash);
    }
}

/// Contains cached state fetched to serve EthApi requests
///
/// This is used as a cache so repeated requests to the same data are not sent to the remote client
#[derive(Clone, Debug, Default)]
pub struct ForkedStorage {
    pub uncles: HashMap<B256, Vec<Block>>,
    pub blocks: HashMap<B256, Block>,
    pub hashes: HashMap<u64, B256>,
    pub transactions: HashMap<B256, WithOtherFields<Transaction>>,
    pub transaction_receipts: HashMap<B256, ReceiptResponse>,
    pub transaction_traces: HashMap<B256, Vec<Trace>>,
    pub logs: HashMap<Filter, Vec<Log>>,
    pub geth_transaction_traces: HashMap<B256, GethTrace>,
    pub block_traces: HashMap<u64, Vec<Trace>>,
    pub block_receipts: HashMap<u64, Vec<ReceiptResponse>>,
    pub code_at: HashMap<(Address, u64), Bytes>,
}

impl ForkedStorage {
    /// Clears all data
    pub fn clear(&mut self) {
        // simply replace with a completely new, empty instance
        *self = Self::default()
    }
}
