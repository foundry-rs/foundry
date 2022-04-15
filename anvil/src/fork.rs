//! Support for forking off another client

use ethers::{
    prelude::{Http, Provider},
    types::H256,
};
use std::{collections::HashMap, sync::Arc};

use ethers::{
    prelude::BlockNumber,
    providers::{Middleware, ProviderError},
    types::{Address, Block, Bytes, Filter, Log, Transaction, TransactionReceipt, TxHash, U256},
};
use foundry_evm::utils::u256_to_h256_le;
use parking_lot::{
    lock_api::{RwLockReadGuard, RwLockWriteGuard},
    RawRwLock, RwLock,
};

#[derive(Debug, Clone)]
pub struct ClientFork {
    /// Contains the cached data
    pub storage: Arc<RwLock<ForkedStorage>>,
    /// contains the info how the fork is configured
    // Wrapping this in a lock, ensures we can update this on the fly via additional custom RPC
    // endpoints
    pub config: Arc<RwLock<ClientForkConfig>>,
}

// === impl ClientFork ===

impl ClientFork {
    /// Returns true whether the block predates the fork
    pub fn predates_fork(&self, block: u64) -> bool {
        block <= self.block_number()
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

    pub async fn storage_at(
        &self,
        address: Address,
        index: U256,
        number: Option<BlockNumber>,
    ) -> Result<H256, ProviderError> {
        let index = u256_to_h256_le(index);
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
        // if let Some(code) = self.storage_read()..get(&(address, blocknumber)).cloned() {
        //     return Ok(code)
        // }
        //
        // let code = self.provider().get_code(address, Some(blocknumber.into())).await?;
        // let mut storage = self.storage_write();
        // storage.code_at.insert((address, blocknumber), code.clone());

        self.provider().get_balance(address, Some(blocknumber.into())).await
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

        if let Some(block) = self.provider().get_block(hash).await? {
            let number = block.number.unwrap().as_u64();
            let mut storage = self.storage_write();
            storage.hashes.insert(number, hash);
            storage.blocks.insert(hash, block.clone());
            return Ok(Some(block))
        }
        Ok(None)
    }

    pub async fn block_by_number(
        &self,
        number: u64,
    ) -> Result<Option<Block<TxHash>>, ProviderError> {
        if let Some(block) = self
            .storage_read()
            .hashes
            .get(&number)
            .copied()
            .and_then(|hash| self.storage_read().blocks.get(&hash).cloned())
        {
            return Ok(Some(block))
        }

        if let Some(block) = self.provider().get_block(number).await? {
            let hash = block.hash.unwrap();
            let mut storage = self.storage_write();
            storage.hashes.insert(number, hash);
            storage.blocks.insert(hash, block.clone());
            return Ok(Some(block))
        }

        Ok(None)
    }
}

#[derive(Debug, Clone)]
pub struct ClientForkConfig {
    pub eth_rpc_url: String,
    pub block_number: u64,
    pub block_hash: H256,
    // TODO make provider agnostic
    pub provider: Arc<Provider<Http>>,
    pub chain_id: u64,
}

/// Contains cached state fetched to serve EthApi requests
#[derive(Debug, Clone, Default)]
pub struct ForkedStorage {
    pub blocks: HashMap<H256, Block<TxHash>>,
    pub hashes: HashMap<u64, H256>,
    pub transactions: HashMap<H256, Transaction>,
    pub transaction_receipts: HashMap<H256, TransactionReceipt>,
    pub code_at: HashMap<(Address, u64), Bytes>,
}
