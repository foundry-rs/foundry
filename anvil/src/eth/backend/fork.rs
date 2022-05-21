//! Support for forking off another client

use crate::eth::{backend::mem::fork_db::ForkedDatabase, error::BlockchainError};
use anvil_core::eth::call::CallRequest;
use ethers::{
    prelude::{BlockNumber, Http, Provider},
    providers::{Middleware, ProviderError},
    types::{
        transaction::eip2930::AccessListWithGasUsed, Address, Block, BlockId, Bytes, Filter, Log,
        Trace, Transaction, TransactionReceipt, TxHash, H256, U256,
    },
};
use foundry_evm::utils::u256_to_h256_be;
use parking_lot::{
    lock_api::{RwLockReadGuard, RwLockWriteGuard},
    RawRwLock, RwLock,
};
use std::{collections::HashMap, sync::Arc};
use tracing::trace;

/// Represents a fork of a remote client
///
/// This type contains a subset of the [`EthApi`](crate::eth::EthApi) functions but will exclusively
/// fetch the requested data from the remote client, if it wasn't already fetched.
#[derive(Debug, Clone)]
pub struct ClientFork {
    /// Contains the cached data
    pub storage: Arc<RwLock<ForkedStorage>>,
    /// contains the info how the fork is configured
    // Wrapping this in a lock, ensures we can update this on the fly via additional custom RPC
    // endpoints
    pub config: Arc<RwLock<ClientForkConfig>>,
    /// This also holds a handle to the underlying database
    pub database: Arc<RwLock<ForkedDatabase>>,
}

// === impl ClientFork ===

impl ClientFork {
    /// Creates a new instance of the fork
    pub fn new(config: ClientForkConfig, database: Arc<RwLock<ForkedDatabase>>) -> Self {
        Self { storage: Default::default(), config: Arc::new(RwLock::new(config)), database }
    }

    /// Reset the fork to a fresh forked state, and optionally update the fork config
    pub async fn reset(
        &self,
        url: Option<String>,
        block_number: Option<u64>,
    ) -> Result<(), BlockchainError> {
        {
            self.database.write().reset(url.clone(), block_number)?;
        }

        if let Some(url) = url {
            self.config.write().update_url(url)?;
            let chain_id = self.provider().get_chainid().await?;
            self.config.write().chain_id = chain_id.as_u64();
        }

        let block = if let Some(block_number) = block_number {
            let provider = self.provider();
            let block =
                provider.get_block(block_number).await?.ok_or(BlockchainError::BlockNotFound)?;
            let block_hash = block.hash.ok_or(BlockchainError::BlockNotFound)?;
            let timestamp = block.timestamp.as_u64();

            Some((block_number, block_hash, timestamp))
        } else {
            None
        };

        self.config.write().update_block(block);
        self.clear_cached_storage();
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

    pub fn timestamp(&self) -> u64 {
        self.config.read().timestamp
    }

    pub fn block_number(&self) -> u64 {
        self.config.read().block_number
    }

    pub fn block_hash(&self) -> H256 {
        self.config.read().block_hash
    }

    pub fn eth_rpc_url(&self) -> String {
        self.config.read().eth_rpc_url.clone()
    }

    pub fn chain_id(&self) -> u64 {
        self.config.read().chain_id
    }

    fn provider(&self) -> Arc<Provider<Http>> {
        self.config.read().provider.clone()
    }

    fn storage_read(&self) -> RwLockReadGuard<'_, RawRwLock, ForkedStorage> {
        self.storage.read()
    }

    fn storage_write(&self) -> RwLockWriteGuard<'_, RawRwLock, ForkedStorage> {
        self.storage.write()
    }

    /// Sends `eth_call`
    pub async fn call(
        &self,
        request: &CallRequest,
        block: Option<BlockNumber>,
    ) -> Result<Bytes, ProviderError> {
        let tx = ethers::utils::serialize(request);
        let block = ethers::utils::serialize(&block.unwrap_or(BlockNumber::Latest));
        self.provider().request("eth_call", [tx, block]).await
    }

    /// Sends `eth_call`
    pub async fn estimate_gas(
        &self,
        request: &CallRequest,
        block: Option<BlockNumber>,
    ) -> Result<U256, ProviderError> {
        let tx = ethers::utils::serialize(request);
        let block = ethers::utils::serialize(&block.unwrap_or(BlockNumber::Latest));
        self.provider().request("eth_estimateGas", [tx, block]).await
    }

    /// Sends `eth_call`
    pub async fn create_access_list(
        &self,
        request: &CallRequest,
        block: Option<BlockNumber>,
    ) -> Result<AccessListWithGasUsed, ProviderError> {
        let tx = ethers::utils::serialize(request);
        let block = ethers::utils::serialize(&block.unwrap_or(BlockNumber::Latest));
        self.provider().request("eth_createAccessList", [tx, block]).await
    }

    pub async fn storage_at(
        &self,
        address: Address,
        index: U256,
        number: Option<BlockNumber>,
    ) -> Result<H256, ProviderError> {
        let index = u256_to_h256_be(index);
        self.provider().get_storage_at(address, index, number.map(Into::into)).await
    }

    pub async fn logs(&self, filter: &Filter) -> Result<Vec<Log>, ProviderError> {
        self.provider().get_logs(filter).await
    }

    pub async fn get_code(
        &self,
        address: Address,
        blocknumber: u64,
    ) -> Result<Bytes, ProviderError> {
        trace!(target: "backend::fork", "get_code={:?}", address);
        if let Some(code) = self.storage_read().code_at.get(&(address, blocknumber)).cloned() {
            return Ok(code)
        }

        let code = self.provider().get_code(address, Some(blocknumber.into())).await?;
        let mut storage = self.storage_write();
        storage.code_at.insert((address, blocknumber), code.clone());

        Ok(code)
    }

    pub async fn get_balance(
        &self,
        address: Address,
        blocknumber: u64,
    ) -> Result<U256, ProviderError> {
        trace!(target: "backend::fork", "get_balance={:?}", address);
        self.provider().get_balance(address, Some(blocknumber.into())).await
    }

    pub async fn get_nonce(
        &self,
        address: Address,
        blocknumber: u64,
    ) -> Result<U256, ProviderError> {
        trace!(target: "backend::fork", "get_nonce={:?}", address);
        self.provider().get_transaction_count(address, Some(blocknumber.into())).await
    }

    pub async fn transaction_by_block_number_and_index(
        &self,
        number: u64,
        index: usize,
    ) -> Result<Option<Transaction>, ProviderError> {
        if let Some(block) = self.block_by_number(number).await? {
            if let Some(tx_hash) = block.transactions.get(index) {
                return self.transaction_by_hash(*tx_hash).await
            }
        }
        Ok(None)
    }

    pub async fn transaction_by_block_hash_and_index(
        &self,
        hash: H256,
        index: usize,
    ) -> Result<Option<Transaction>, ProviderError> {
        if let Some(block) = self.block_by_hash(hash).await? {
            if let Some(tx_hash) = block.transactions.get(index) {
                return self.transaction_by_hash(*tx_hash).await
            }
        }
        Ok(None)
    }

    pub async fn transaction_by_hash(
        &self,
        hash: H256,
    ) -> Result<Option<Transaction>, ProviderError> {
        trace!(target: "backend::fork", "transaction_by_hash={:?}", hash);
        if let tx @ Some(_) = self.storage_read().transactions.get(&hash).cloned() {
            return Ok(tx)
        }

        if let Some(tx) = self.provider().get_transaction(hash).await? {
            let mut storage = self.storage_write();
            storage.transactions.insert(hash, tx.clone());
            return Ok(Some(tx))
        }
        Ok(None)
    }

    pub async fn trace_transaction(&self, hash: H256) -> Result<Vec<Trace>, ProviderError> {
        if let Some(traces) = self.storage_read().transaction_traces.get(&hash).cloned() {
            return Ok(traces)
        }

        let traces = self.provider().trace_transaction(hash).await?;
        let mut storage = self.storage_write();
        storage.transaction_traces.insert(hash, traces.clone());

        Ok(traces)
    }

    pub async fn trace_block(&self, number: u64) -> Result<Vec<Trace>, ProviderError> {
        if let Some(traces) = self.storage_read().block_traces.get(&number).cloned() {
            return Ok(traces)
        }

        let traces = self.provider().trace_block(number.into()).await?;
        let mut storage = self.storage_write();
        storage.block_traces.insert(number, traces.clone());

        Ok(traces)
    }

    pub async fn transaction_receipt(
        &self,
        hash: H256,
    ) -> Result<Option<TransactionReceipt>, ProviderError> {
        if let Some(receipt) = self.storage_read().transaction_receipts.get(&hash).cloned() {
            return Ok(Some(receipt))
        }

        if let Some(receipt) = self.provider().get_transaction_receipt(hash).await? {
            let mut storage = self.storage_write();
            storage.transaction_receipts.insert(hash, receipt.clone());
            return Ok(Some(receipt))
        }

        Ok(None)
    }

    pub async fn block_by_hash(&self, hash: H256) -> Result<Option<Block<TxHash>>, ProviderError> {
        if let Some(block) = self.storage_read().blocks.get(&hash).cloned() {
            return Ok(Some(block))
        }
        let block = self.fetch_full_block(hash).await?.map(Into::into);
        Ok(block)
    }

    pub async fn block_by_hash_full(
        &self,
        hash: H256,
    ) -> Result<Option<Block<Transaction>>, ProviderError> {
        if let Some(block) = self.storage_read().blocks.get(&hash).cloned() {
            return Ok(Some(self.convert_to_full_block(block)))
        }
        self.fetch_full_block(hash).await
    }

    pub async fn block_by_number(
        &self,
        block_number: u64,
    ) -> Result<Option<Block<TxHash>>, ProviderError> {
        if let Some(block) = self
            .storage_read()
            .hashes
            .get(&block_number)
            .copied()
            .and_then(|hash| self.storage_read().blocks.get(&hash).cloned())
        {
            return Ok(Some(block))
        }

        let block = self.fetch_full_block(block_number).await?.map(Into::into);
        Ok(block)
    }

    pub async fn block_by_number_full(
        &self,
        block_number: u64,
    ) -> Result<Option<Block<Transaction>>, ProviderError> {
        if let Some(block) = self
            .storage_read()
            .hashes
            .get(&block_number)
            .copied()
            .and_then(|hash| self.storage_read().blocks.get(&hash).cloned())
        {
            return Ok(Some(self.convert_to_full_block(block)))
        }

        self.fetch_full_block(block_number).await
    }

    async fn fetch_full_block(
        &self,
        block_id: impl Into<BlockId>,
    ) -> Result<Option<Block<Transaction>>, ProviderError> {
        if let Some(block) = self.provider().get_block_with_txs(block_id.into()).await? {
            let hash = block.hash.unwrap();
            let block_number = block.number.unwrap().as_u64();
            let mut storage = self.storage_write();
            // also insert all transactions
            storage.transactions.extend(block.transactions.iter().map(|tx| (tx.hash, tx.clone())));
            storage.hashes.insert(block_number, hash);
            storage.blocks.insert(hash, block.clone().into());
            return Ok(Some(block))
        }

        Ok(None)
    }

    /// Converts a block of hashes into a full block
    fn convert_to_full_block(&self, block: Block<TxHash>) -> Block<Transaction> {
        let storage = self.storage.read();
        let mut transactions = Vec::with_capacity(block.transactions.len());
        for tx in block.transactions.iter() {
            if let Some(tx) = storage.transactions.get(tx).cloned() {
                transactions.push(tx);
            }
        }
        block.into_full_block(transactions)
    }
}

/// Contains all fork metadata
#[derive(Debug, Clone)]
pub struct ClientForkConfig {
    pub eth_rpc_url: String,
    pub block_number: u64,
    pub block_hash: H256,
    // TODO make provider agnostic
    pub provider: Arc<Provider<Http>>,
    pub chain_id: u64,
    /// The timestamp for the forked block
    pub timestamp: u64,
}

// === impl ClientForkConfig ===

impl ClientForkConfig {
    /// Updates the provider URL
    ///
    /// # Errors
    ///
    /// This will fail if no new provider could be established (erroneous URL)
    fn update_url(&mut self, url: String) -> Result<(), BlockchainError> {
        self.provider = Arc::new(
            Provider::try_from(&url).map_err(|_| BlockchainError::InvalidUrl(url.clone()))?,
        );
        trace!(target: "fork", "Updated rpc url  {}", url);
        self.eth_rpc_url = url;
        Ok(())
    }
    /// Updates the block forked off `(block number, block hash, timestamp)`
    pub fn update_block(&mut self, block: Option<(u64, H256, u64)>) {
        if let Some((block_number, block_hash, timestamp)) = block {
            self.block_number = block_number;
            self.block_hash = block_hash;
            self.timestamp = timestamp;
            trace!(target: "fork", "Updated block number={} hash={:?}", block_number, block_hash);
        }
    }
}

/// Contains cached state fetched to serve EthApi requests
#[derive(Debug, Clone, Default)]
pub struct ForkedStorage {
    pub blocks: HashMap<H256, Block<TxHash>>,
    pub hashes: HashMap<u64, H256>,
    pub transactions: HashMap<H256, Transaction>,
    pub transaction_receipts: HashMap<H256, TransactionReceipt>,
    pub transaction_traces: HashMap<H256, Vec<Trace>>,
    pub block_traces: HashMap<u64, Vec<Trace>>,
    pub code_at: HashMap<(Address, u64), Bytes>,
}

// === impl ForkedStorage ===

impl ForkedStorage {
    /// Clears all data
    pub fn clear(&mut self) {
        // simply replace with a completely new, empty instance
        *self = Self::default()
    }
}
