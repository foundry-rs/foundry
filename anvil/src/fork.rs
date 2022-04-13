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
use parking_lot::RwLock;

#[derive(Debug, Clone)]
pub struct ClientFork {
    pub eth_rpc_url: String,
    pub block_number: u64,
    pub block_hash: H256,
    pub storage: Arc<RwLock<ForkedStorage>>,
    // TODO make provider agnostic
    pub provider: Arc<Provider<Http>>,
    pub chain_id: u64,
}

// === impl ClientFork ===

impl ClientFork {
    /// Returns true whether the block predates the fork
    pub fn predates_fork(&self, block: u64) -> bool {
        block <= self.block_number
    }

    pub async fn storage_at(
        &self,
        address: Address,
        index: U256,
        number: Option<BlockNumber>,
    ) -> Result<H256, ProviderError> {
        let index = u256_to_h256_le(index);
        self.provider.get_storage_at(address, index, number.map(Into::into)).await
    }

    pub async fn logs(&self, filter: &Filter) -> Result<Vec<Log>, ProviderError> {
        self.provider.get_logs(filter).await
    }

    pub async fn get_code(
        &self,
        address: Address,
        blocknumber: u64,
    ) -> Result<Bytes, ProviderError> {
        if let Some(code) = self.storage.read().code_at.get(&(address, blocknumber)).cloned() {
            return Ok(code)
        }

        let code = self.provider.get_code(address, Some(blocknumber.into())).await?;
        let mut storage = self.storage.write();
        storage.code_at.insert((address, blocknumber), code.clone());

        Ok(code)
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
        if let tx @ Some(_) = self.storage.read().transactions.get(&hash).cloned() {
            return Ok(tx)
        }

        if let Some(tx) = self.provider.get_transaction(hash).await? {
            let mut storage = self.storage.write();
            storage.transactions.insert(hash, tx.clone());
            return Ok(Some(tx))
        }
        Ok(None)
    }

    pub async fn transaction_receipt(
        &self,
        hash: H256,
    ) -> Result<Option<TransactionReceipt>, ProviderError> {
        if let Some(receipt) = self.storage.read().transaction_receipts.get(&hash).cloned() {
            return Ok(Some(receipt))
        }

        if let Some(receipt) = self.provider.get_transaction_receipt(hash).await? {
            let mut storage = self.storage.write();
            storage.transaction_receipts.insert(hash, receipt.clone());
            return Ok(Some(receipt))
        }

        Ok(None)
    }

    pub async fn block_by_hash(&self, hash: H256) -> Result<Option<Block<TxHash>>, ProviderError> {
        if let Some(block) = self.storage.read().blocks.get(&hash).cloned() {
            return Ok(Some(block))
        }

        if let Some(block) = self.provider.get_block(hash).await? {
            let number = block.number.unwrap().as_u64();
            let mut storage = self.storage.write();
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
            .storage
            .read()
            .hashes
            .get(&number)
            .copied()
            .and_then(|hash| self.storage.read().blocks.get(&hash).cloned())
        {
            return Ok(Some(block))
        }

        if let Some(block) = self.provider.get_block(number).await? {
            let hash = block.hash.unwrap();
            let mut storage = self.storage.write();
            storage.hashes.insert(number, hash);
            storage.blocks.insert(hash, block.clone());
            return Ok(Some(block))
        }

        Ok(None)
    }
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
