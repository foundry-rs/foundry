//! In memory blockchain backend

use crate::eth::{
    backend::{db::Db, executor::TransactionExecutor},
    pool::transactions::PoolTransaction,
};
use ethers::{
    prelude::{BlockNumber, TxHash, H256, U256, U64},
    types::BlockId,
};

use crate::{
    eth::{backend::duration_since_unix_epoch, error::InvalidTransactionError, fees::FeeDetails},
    revm::db::DatabaseRef,
};
use ethers::{
    types::{Address, Block as EthersBlock, Log, Transaction, TransactionReceipt},
    utils::{keccak256, rlp},
};
use foundry_evm::{
    revm,
    revm::{db::CacheDB, CreateScheme, Env, Return, TransactOut, TransactTo, TxEnv},
};
use foundry_node_core::eth::{
    block::{Block, BlockInfo, Header, PartialHeader},
    call::CallRequest,
    receipt::{EIP658Receipt, TypedReceipt},
    transaction::{PendingTransaction, TransactionInfo, TypedTransaction},
    utils::to_access_list,
};
use parking_lot::RwLock;
use std::{collections::HashMap, sync::Arc};
use tracing::trace;

/// Stores the blockchain data (blocks, transactions)
#[derive(Clone)]
struct BlockchainStorage {
    /// all stored blocks (block hash -> block)
    blocks: HashMap<H256, Block>,
    /// mapping from block number -> block hash
    hashes: HashMap<U64, H256>,
    /// The current best hash
    best_hash: H256,
    /// The current best block number
    best_number: U64,
    /// genesis hash of the chain
    genesis_hash: H256,
    /// Mapping from the transaction hash to a tuple containing the transaction as well as the
    /// transaction receipt
    transactions: HashMap<TxHash, MinedTransaction>,
}

impl BlockchainStorage {
    #[allow(unused)]
    pub fn empty() -> Self {
        Self {
            blocks: Default::default(),
            hashes: Default::default(),
            best_hash: Default::default(),
            best_number: Default::default(),
            genesis_hash: Default::default(),
            transactions: Default::default(),
        }
    }
}

impl Default for BlockchainStorage {
    fn default() -> Self {
        // create a dummy genesis block
        let partial_header = PartialHeader {
            timestamp: duration_since_unix_epoch().as_secs(),
            ..Default::default()
        };
        let block = Block::new(partial_header, vec![], vec![]);
        let genesis_hash = block.header.hash();
        let best_hash = genesis_hash;
        let best_number: U64 = 0u64.into();

        Self {
            blocks: HashMap::from([(genesis_hash, block)]),
            hashes: HashMap::from([(best_number, genesis_hash)]),
            best_hash,
            best_number,
            genesis_hash,
            transactions: Default::default(),
        }
    }
}

impl BlockchainStorage {
    /// Returns the hash for [BlockNumber]
    pub fn hash(&self, number: BlockNumber) -> Option<H256> {
        match number {
            BlockNumber::Latest => Some(self.best_hash),
            BlockNumber::Earliest => Some(self.genesis_hash),
            BlockNumber::Pending => None,
            BlockNumber::Number(num) => self.hashes.get(&num).copied(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MinedTransaction {
    pub info: TransactionInfo,
    pub receipt: TypedReceipt,
    pub block_hash: H256,
}

/// A simple in-memory blockchain
#[derive(Clone, Default)]
pub struct Blockchain {
    /// underlying storage that supports concurrent reads
    storage: Arc<RwLock<BlockchainStorage>>,
}

impl Blockchain {
    /// returns the header hash of given block
    pub fn hash(&self, id: BlockId) -> Option<H256> {
        match id {
            BlockId::Hash(h) => Some(h),
            BlockId::Number(num) => self.storage.read().hash(num),
        }
    }

    /// Returns the total number of blocks
    pub fn blocks_count(&self) -> usize {
        self.storage.read().blocks.len()
    }
}

/// Gives access to the [revm::Database]
#[derive(Clone)]
pub struct Backend {
    /// access to revm's database related operations
    /// This stores the actual state of the blockchain
    /// Supports concurrent reads
    db: Arc<RwLock<dyn Db>>,
    /// stores all block related data in memory
    blockchain: Blockchain,
    /// env data of the chain
    env: Arc<RwLock<Env>>,

    /// Default gas price for all transactions
    gas_price: U256,
}

impl Backend {
    /// Create a new instance of in-mem backend.
    pub fn new(db: Arc<RwLock<dyn Db>>, env: Arc<RwLock<Env>>, gas_price: U256) -> Self {
        Self { db, blockchain: Blockchain::default(), env, gas_price }
    }

    /// Creates a new empty blockchain backend
    pub fn empty(env: Arc<RwLock<Env>>, gas_price: U256) -> Self {
        let db = CacheDB::default();
        Self::new(Arc::new(RwLock::new(db)), env, gas_price)
    }

    /// Initialises the balance of the given accounts
    pub fn with_genesis_balance(
        env: Arc<RwLock<Env>>,
        balance: U256,
        accounts: impl IntoIterator<Item = Address>,
        gas_price: U256,
    ) -> Self {
        let mut db = CacheDB::default();
        for account in accounts {
            let mut info = db.basic(account);
            info.balance = balance;
            db.insert_cache(account, info);
        }
        Self::new(Arc::new(RwLock::new(db)), env, gas_price)
    }

    /// The env data of the blockchain
    pub fn env(&self) -> &Arc<RwLock<Env>> {
        &self.env
    }

    /// Returns the current best hash of the chain
    pub fn best_hash(&self) -> H256 {
        self.blockchain.storage.read().best_hash
    }

    /// Returns the current best number of the chain
    pub fn best_number(&self) -> U64 {
        self.blockchain.storage.read().best_number
    }

    /// Returns the client coinbase address.
    pub fn coinbase(&self) -> Address {
        self.env.read().block.coinbase
    }

    /// Returns the client coinbase address.
    pub fn chain_id(&self) -> U256 {
        self.env.read().cfg.chain_id
    }

    /// Returns balance of the given account.
    pub fn current_balance(&self, address: Address) -> U256 {
        self.db.read().basic(address).balance
    }

    /// Returns balance of the given account.
    pub fn current_nonce(&self, address: Address) -> U256 {
        self.db.read().basic(address).nonce.into()
    }

    pub fn gas_limit(&self) -> U256 {
        // TODO make this a separate value?
        self.env().read().block.gas_limit
    }

    pub fn base_fee(&self) -> U256 {
        self.env().read().block.basefee
    }

    pub fn gas_price(&self) -> U256 {
        self.gas_price
    }

    /// Validates the transaction's validity when it comes to nonce, payment
    ///
    /// This is intended to be checked before the transaction makes it into the pool and whether it
    /// should rather be outright rejected if the sender has insufficient funds.
    pub fn validate_transaction(
        &self,
        tx: &PendingTransaction,
    ) -> Result<(), InvalidTransactionError> {
        let sender = *tx.sender();
        let tx = &tx.transaction;
        let account = self.db.read().basic(sender);

        // check nonce
        if tx.nonce().as_u64() < account.nonce {
            return Err(InvalidTransactionError::Payment)
        }

        let max_cost = tx.max_cost();
        let value = tx.value();
        // check sufficient funds: `gas * price + value`
        let req_funds =
            max_cost.checked_add(value).ok_or_else(|| InvalidTransactionError::Payment)?;

        if account.balance < req_funds {
            return Err(InvalidTransactionError::Payment)
        }

        Ok(())
    }

    /// Mines a new block and stores it.
    ///
    /// this will execute all transaction in the order they come in and return all the markers they
    /// provide.
    ///
    /// TODO(mattsse): currently we're assuming all transactions are valid:
    ///  needs an additional validation step: gas limit, fee
    pub fn mine_block(&self, pool_transactions: Vec<Arc<PoolTransaction>>) -> U64 {
        // acquire all locks
        let mut env = self.env.write();
        let mut db = self.db.write();
        let mut storage = self.blockchain.storage.write();

        // increase block number for this block
        env.block.number = env.block.number.saturating_add(U256::one());

        let executor = TransactionExecutor {
            db: &mut *db,
            pending: pool_transactions.into_iter(),
            block_env: env.block.clone(),
            cfg_env: env.cfg.clone(),
            parent_hash: storage.best_hash,
        };

        let BlockInfo { block, transactions, receipts } = executor.create_block();

        let block_hash = block.header.hash();
        let block_number: U64 = env.block.number.as_u64().into();

        trace!(target: "backend", "Created block {} with {} tx: [{:?}]", block_number, transactions.len(), block_hash);

        // update block metadata
        storage.best_number = block_number;
        storage.best_hash = block_hash;

        storage.blocks.insert(block_hash, block);
        storage.hashes.insert(block_number, block_hash);

        // insert all transactions
        for (info, receipt) in transactions.into_iter().zip(receipts) {
            let mined_tx = MinedTransaction { info, receipt, block_hash };
            storage.transactions.insert(mined_tx.info.transaction_hash, mined_tx);
        }

        block_number
    }

    /// Executes the `CallRequest` without writing to the DB
    pub fn call(
        &self,
        request: CallRequest,
        fee_details: FeeDetails,
    ) -> (Return, TransactOut, u64) {
        let CallRequest { from, to, gas, value, data, nonce, access_list, .. } = request;

        let FeeDetails { gas_price, max_fee_per_gas, max_priority_fee_per_gas } = fee_details;

        let gas_limit = gas.unwrap_or_else(|| self.gas_limit());
        let mut env = self.env.read().clone();

        env.tx = TxEnv {
            caller: from.unwrap_or_default(),
            gas_limit: gas_limit.as_u64(),
            gas_price: gas_price.or(max_fee_per_gas).unwrap_or(self.gas_price),
            gas_priority_fee: max_priority_fee_per_gas,
            transact_to: match to {
                Some(addr) => TransactTo::Call(addr),
                None => TransactTo::Create(CreateScheme::Create),
            },
            value: value.unwrap_or_default(),
            data: data.unwrap_or_else(|| vec![].into()).to_vec().into(),
            chain_id: None,
            nonce: nonce.map(|n| n.as_u64()),
            access_list: to_access_list(access_list.unwrap_or_default().0),
        };

        trace!(target: "backend", "calling with {:?}", env.tx);

        let db = self.db.read();
        let mut evm = revm::EVM::new();
        evm.env = env;
        evm.database(&*db);

        let (exit, out, gas, _, _) = evm.transact_ref();
        trace!(target: "backend", "call return {:?} out: {:?} gas {}", exit, out, gas);

        (exit, out, gas)
    }

    /// returns all receipts for the given transactions
    fn get_receipts(&self, tx_hashes: impl IntoIterator<Item = TxHash>) -> Vec<TypedReceipt> {
        let storage = self.blockchain.storage.read();
        let mut receipts = vec![];

        for hash in tx_hashes {
            if let Some(tx) = storage.transactions.get(&hash) {
                receipts.push(tx.receipt.clone());
            }
        }

        receipts
    }

    pub fn block_by_number(&self, number: BlockNumber) -> Option<EthersBlock<TxHash>> {
        let block = {
            let storage = self.blockchain.storage.read();
            let hash = match number {
                BlockNumber::Latest => storage.best_hash,
                BlockNumber::Earliest => storage.genesis_hash,
                BlockNumber::Pending => return None,
                BlockNumber::Number(num) => *storage.hashes.get(&num)?,
            };
            storage.blocks.get(&hash)?.clone()
        };
        let size = U256::from(rlp::encode(&block).len() as u32);

        let Block { header, transactions, .. } = block;

        let hash = header.hash();
        let Header {
            parent_hash,
            ommers_hash,
            beneficiary,
            state_root,
            transactions_root,
            receipts_root,
            logs_bloom,
            difficulty,
            number,
            gas_limit,
            gas_used,
            timestamp,
            extra_data,
            mix_hash,
            nonce,
        } = header;

        let block = EthersBlock {
            hash: Some(hash),
            parent_hash,
            uncles_hash: ommers_hash,
            author: beneficiary,
            state_root,
            transactions_root,
            receipts_root,
            number: Some(number.as_u64().into()),
            gas_used,
            gas_limit,
            extra_data,
            logs_bloom: Some(logs_bloom),
            timestamp: timestamp.into(),
            difficulty,
            total_difficulty: None,
            seal_fields: {
                let mut arr = [0u8; 8];
                nonce.to_big_endian(&mut arr);
                vec![mix_hash.as_bytes().to_vec().into(), arr.to_vec().into()]
            },
            uncles: vec![],
            transactions: transactions.into_iter().map(|tx| tx.hash()).collect(),
            size: Some(size),
            mix_hash: Some(mix_hash),
            nonce: Some(nonce),
            // TODO check
            base_fee_per_gas: Some(self.base_fee()),
        };

        Some(block)
    }

    /// Returns the transaction receipt for the given hash
    pub fn transaction_receipt(&self, hash: H256) -> Option<TransactionReceipt> {
        let MinedTransaction { info, receipt, block_hash, .. } =
            self.blockchain.storage.read().transactions.get(&hash)?.clone();

        let EIP658Receipt { status_code, gas_used, logs_bloom, logs } = receipt.into();

        let index = info.transaction_index as usize;

        let block = self.blockchain.storage.read().blocks.get(&block_hash).cloned()?;

        // TODO store cumulative gas used in receipt instead
        let receipts = self.get_receipts(block.transactions.iter().map(|tx| tx.hash()));

        let mut cumulative_gas_used = U256::zero();
        for receipt in receipts.iter().take(index) {
            cumulative_gas_used = cumulative_gas_used.saturating_add(receipt.gas_used());
        }

        // cumulative_gas_used = cumulative_gas_used.saturating_sub(gas_used);

        let mut cumulative_receipts = receipts;
        cumulative_receipts.truncate(index + 1);

        let transaction = block.transactions[index].clone();

        let effective_gas_price = match transaction {
            TypedTransaction::Legacy(t) => t.gas_price,
            TypedTransaction::EIP2930(t) => t.gas_price,
            TypedTransaction::EIP1559(t) => self
                .base_fee()
                .checked_add(t.max_priority_fee_per_gas)
                .unwrap_or_else(U256::max_value),
        };

        Some(TransactionReceipt {
            transaction_hash: info.transaction_hash,
            transaction_index: info.transaction_index.into(),
            block_hash: Some(block_hash),
            block_number: Some(block.header.number.as_u64().into()),
            cumulative_gas_used,
            gas_used: Some(gas_used),
            contract_address: info.contract_address,
            logs: {
                let mut pre_receipts_log_index = None;
                if !cumulative_receipts.is_empty() {
                    cumulative_receipts.truncate(cumulative_receipts.len() - 1);
                    pre_receipts_log_index =
                        Some(cumulative_receipts.iter().map(|_r| logs.len() as u32).sum::<u32>());
                }
                logs.iter()
                    .enumerate()
                    .map(|(i, log)| Log {
                        address: log.address,
                        topics: log.topics.clone(),
                        data: log.data.clone(),
                        block_hash: Some(block_hash),
                        block_number: Some(block.header.number.as_u64().into()),
                        transaction_hash: Some(info.transaction_hash),
                        transaction_index: Some(info.transaction_index.into()),
                        log_index: Some(U256::from(
                            (pre_receipts_log_index.unwrap_or(0)) + i as u32,
                        )),
                        transaction_log_index: Some(U256::from(i)),
                        log_type: None,
                        removed: None,
                    })
                    .collect()
            },
            status: Some(status_code.into()),
            root: None,
            logs_bloom,
            transaction_type: None,
            effective_gas_price: Some(effective_gas_price),
        })
    }

    pub fn transaction_by_hash(&self, hash: H256) -> Option<Transaction> {
        let MinedTransaction { info, block_hash, .. } =
            self.blockchain.storage.read().transactions.get(&hash)?.clone();

        let block = self.blockchain.storage.read().blocks.get(&block_hash).cloned()?;

        let tx = block.transactions.get(info.transaction_index as usize)?.clone();

        Some(transaction_build(tx, Some(block), Some(info), true, Some(self.base_fee())))
    }
}

pub fn transaction_build(
    eth_transaction: TypedTransaction,
    block: Option<Block>,
    info: Option<TransactionInfo>,
    is_eip1559: bool,
    base_fee: Option<U256>,
) -> Transaction {
    let mut transaction: Transaction = eth_transaction.clone().into();

    if let TypedTransaction::EIP1559(_) = eth_transaction {
        if block.is_none() && info.is_none() {
            // transaction is not mined yet, gas price is considered just `max_fee_per_gas`
            transaction.gas_price = transaction.max_fee_per_gas;
        } else {
            // if transaction is already mined, gas price is considered base fee + priority fee: the
            // effective gas price.
            let base_fee = base_fee.unwrap_or(U256::zero());
            let max_priority_fee_per_gas =
                transaction.max_priority_fee_per_gas.unwrap_or(U256::zero());
            transaction.gas_price = Some(
                base_fee.checked_add(max_priority_fee_per_gas).unwrap_or_else(U256::max_value),
            );
        }
    } else if !is_eip1559 {
        transaction.max_fee_per_gas = None;
        transaction.max_priority_fee_per_gas = None;
        transaction.transaction_type = None;
    }

    transaction.block_hash =
        block.as_ref().map(|block| H256::from(keccak256(&rlp::encode(&block.header))));

    transaction.block_number = block.as_ref().map(|block| block.header.number.as_u64().into());

    transaction.transaction_index = info.as_ref().map(|status| status.transaction_index.into());

    transaction.from = eth_transaction.recover().unwrap();

    transaction.to = info.as_ref().map_or(eth_transaction.to().cloned(), |status| status.to);

    transaction
}
