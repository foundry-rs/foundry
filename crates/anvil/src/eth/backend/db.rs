//! Helper types for working with [revm](foundry_evm::revm)

use crate::{mem::storage::MinedTransaction, revm::primitives::AccountInfo};
use alloy_consensus::Header;
use alloy_primitives::{keccak256, Address, Bytes, B256, U256, U64};
use alloy_rpc_types::BlockId;
use anvil_core::eth::{
    block::Block,
    transaction::{MaybeImpersonatedTransaction, TransactionInfo, TypedReceipt},
};
use foundry_common::errors::FsPathError;
use foundry_evm::{
    backend::{
        BlockchainDb, DatabaseError, DatabaseResult, MemDb, RevertSnapshotAction, StateSnapshot,
    },
    revm::{
        db::{CacheDB, DatabaseRef, DbAccount},
        primitives::{BlockEnv, Bytecode, HashMap, KECCAK_EMPTY},
        Database, DatabaseCommit,
    },
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, path::Path};

/// Helper trait get access to the full state data of the database
#[auto_impl::auto_impl(Box)]
pub trait MaybeFullDatabase: DatabaseRef<Error = DatabaseError> {
    fn maybe_as_full_db(&self) -> Option<&HashMap<Address, DbAccount>> {
        None
    }

    /// Clear the state and move it into a new `StateSnapshot`
    fn clear_into_snapshot(&mut self) -> StateSnapshot;

    /// Clears the entire database
    fn clear(&mut self);

    /// Reverses `clear_into_snapshot` by initializing the db's state with the snapshot
    fn init_from_snapshot(&mut self, snapshot: StateSnapshot);
}

impl<'a, T: 'a + MaybeFullDatabase + ?Sized> MaybeFullDatabase for &'a T
where
    &'a T: DatabaseRef<Error = DatabaseError>,
{
    fn maybe_as_full_db(&self) -> Option<&HashMap<Address, DbAccount>> {
        T::maybe_as_full_db(self)
    }

    fn clear_into_snapshot(&mut self) -> StateSnapshot {
        unreachable!("never called for DatabaseRef")
    }

    fn clear(&mut self) {}

    fn init_from_snapshot(&mut self, _snapshot: StateSnapshot) {}
}

/// Helper trait to reset the DB if it's forked
#[auto_impl::auto_impl(Box)]
pub trait MaybeForkedDatabase {
    fn maybe_reset(&mut self, _url: Option<String>, block_number: BlockId) -> Result<(), String>;

    fn maybe_flush_cache(&self) -> Result<(), String>;

    fn maybe_inner(&self) -> Result<&BlockchainDb, String>;
}

/// This bundles all required revm traits
#[auto_impl::auto_impl(Box)]
pub trait Db:
    DatabaseRef<Error = DatabaseError>
    + Database<Error = DatabaseError>
    + DatabaseCommit
    + MaybeFullDatabase
    + MaybeForkedDatabase
    + fmt::Debug
    + Send
    + Sync
{
    /// Inserts an account
    fn insert_account(&mut self, address: Address, account: AccountInfo);

    /// Sets the nonce of the given address
    fn set_nonce(&mut self, address: Address, nonce: u64) -> DatabaseResult<()> {
        let mut info = self.basic(address)?.unwrap_or_default();
        info.nonce = nonce;
        self.insert_account(address, info);
        Ok(())
    }

    /// Sets the balance of the given address
    fn set_balance(&mut self, address: Address, balance: U256) -> DatabaseResult<()> {
        let mut info = self.basic(address)?.unwrap_or_default();
        info.balance = balance;
        self.insert_account(address, info);
        Ok(())
    }

    /// Sets the balance of the given address
    fn set_code(&mut self, address: Address, code: Bytes) -> DatabaseResult<()> {
        let mut info = self.basic(address)?.unwrap_or_default();
        let code_hash = if code.as_ref().is_empty() {
            KECCAK_EMPTY
        } else {
            B256::from_slice(&keccak256(code.as_ref())[..])
        };
        info.code_hash = code_hash;
        info.code = Some(Bytecode::new_raw(alloy_primitives::Bytes(code.0)));
        self.insert_account(address, info);
        Ok(())
    }

    /// Sets the balance of the given address
    fn set_storage_at(&mut self, address: Address, slot: U256, val: U256) -> DatabaseResult<()>;

    /// inserts a blockhash for the given number
    fn insert_block_hash(&mut self, number: U256, hash: B256);

    /// Write all chain data to serialized bytes buffer
    fn dump_state(
        &self,
        at: BlockEnv,
        best_number: U64,
        blocks: Vec<SerializableBlock>,
        transactions: Vec<SerializableTransaction>,
    ) -> DatabaseResult<Option<SerializableState>>;

    /// Deserialize and add all chain data to the backend storage
    fn load_state(&mut self, state: SerializableState) -> DatabaseResult<bool> {
        for (addr, account) in state.accounts.into_iter() {
            let old_account_nonce = DatabaseRef::basic_ref(self, addr)
                .ok()
                .and_then(|acc| acc.map(|acc| acc.nonce))
                .unwrap_or_default();
            // use max nonce in case account is imported multiple times with difference
            // nonces to prevent collisions
            let nonce = std::cmp::max(old_account_nonce, account.nonce);

            self.insert_account(
                addr,
                AccountInfo {
                    balance: account.balance,
                    code_hash: KECCAK_EMPTY, // will be set automatically
                    code: if account.code.0.is_empty() {
                        None
                    } else {
                        Some(Bytecode::new_raw(alloy_primitives::Bytes(account.code.0)))
                    },
                    nonce,
                },
            );

            for (k, v) in account.storage.into_iter() {
                self.set_storage_at(addr, k, v)?;
            }
        }
        Ok(true)
    }

    /// Creates a new snapshot
    fn snapshot(&mut self) -> U256;

    /// Reverts a snapshot
    ///
    /// Returns `true` if the snapshot was reverted
    fn revert(&mut self, snapshot: U256, action: RevertSnapshotAction) -> bool;

    /// Returns the state root if possible to compute
    fn maybe_state_root(&self) -> Option<B256> {
        None
    }

    /// Returns the current, standalone state of the Db
    fn current_state(&self) -> StateDb;
}

/// Convenience impl only used to use any `Db` on the fly as the db layer for revm's CacheDB
/// This is useful to create blocks without actually writing to the `Db`, but rather in the cache of
/// the `CacheDB` see also
/// [Backend::pending_block()](crate::eth::backend::mem::Backend::pending_block())
impl<T: DatabaseRef<Error = DatabaseError> + Send + Sync + Clone + fmt::Debug> Db for CacheDB<T> {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.insert_account_info(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: U256, val: U256) -> DatabaseResult<()> {
        self.insert_account_storage(address, slot, val)
    }

    fn insert_block_hash(&mut self, number: U256, hash: B256) {
        self.block_hashes.insert(number, hash);
    }

    fn dump_state(
        &self,
        _at: BlockEnv,
        _best_number: U64,
        _blocks: Vec<SerializableBlock>,
        _transaction: Vec<SerializableTransaction>,
    ) -> DatabaseResult<Option<SerializableState>> {
        Ok(None)
    }

    fn snapshot(&mut self) -> U256 {
        U256::ZERO
    }

    fn revert(&mut self, _snapshot: U256, _action: RevertSnapshotAction) -> bool {
        false
    }

    fn current_state(&self) -> StateDb {
        StateDb::new(MemDb::default())
    }
}

impl<T: DatabaseRef<Error = DatabaseError>> MaybeFullDatabase for CacheDB<T> {
    fn maybe_as_full_db(&self) -> Option<&HashMap<Address, DbAccount>> {
        Some(&self.accounts)
    }

    fn clear_into_snapshot(&mut self) -> StateSnapshot {
        let db_accounts = std::mem::take(&mut self.accounts);
        let mut accounts = HashMap::new();
        let mut account_storage = HashMap::new();

        for (addr, mut acc) in db_accounts {
            account_storage.insert(addr, std::mem::take(&mut acc.storage));
            let mut info = acc.info;
            info.code = self.contracts.remove(&info.code_hash);
            accounts.insert(addr, info);
        }
        let block_hashes = std::mem::take(&mut self.block_hashes);
        StateSnapshot { accounts, storage: account_storage, block_hashes }
    }

    fn clear(&mut self) {
        self.clear_into_snapshot();
    }

    fn init_from_snapshot(&mut self, snapshot: StateSnapshot) {
        let StateSnapshot { accounts, mut storage, block_hashes } = snapshot;

        for (addr, mut acc) in accounts {
            if let Some(code) = acc.code.take() {
                self.contracts.insert(acc.code_hash, code);
            }
            self.accounts.insert(
                addr,
                DbAccount {
                    info: acc,
                    storage: storage.remove(&addr).unwrap_or_default(),
                    ..Default::default()
                },
            );
        }
        self.block_hashes = block_hashes;
    }
}

impl<T: DatabaseRef<Error = DatabaseError>> MaybeForkedDatabase for CacheDB<T> {
    fn maybe_reset(&mut self, _url: Option<String>, _block_number: BlockId) -> Result<(), String> {
        Err("not supported".to_string())
    }

    fn maybe_flush_cache(&self) -> Result<(), String> {
        Err("not supported".to_string())
    }

    fn maybe_inner(&self) -> Result<&BlockchainDb, String> {
        Err("not supported".to_string())
    }
}

/// Represents a state at certain point
pub struct StateDb(pub(crate) Box<dyn MaybeFullDatabase + Send + Sync>);

impl StateDb {
    pub fn new(db: impl MaybeFullDatabase + Send + Sync + 'static) -> Self {
        Self(Box::new(db))
    }
}

impl DatabaseRef for StateDb {
    type Error = DatabaseError;
    fn basic_ref(&self, address: Address) -> DatabaseResult<Option<AccountInfo>> {
        self.0.basic_ref(address)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> DatabaseResult<Bytecode> {
        self.0.code_by_hash_ref(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> DatabaseResult<U256> {
        self.0.storage_ref(address, index)
    }

    fn block_hash_ref(&self, number: u64) -> DatabaseResult<B256> {
        self.0.block_hash_ref(number)
    }
}

impl MaybeFullDatabase for StateDb {
    fn maybe_as_full_db(&self) -> Option<&HashMap<Address, DbAccount>> {
        self.0.maybe_as_full_db()
    }

    fn clear_into_snapshot(&mut self) -> StateSnapshot {
        self.0.clear_into_snapshot()
    }

    fn clear(&mut self) {
        self.0.clear()
    }

    fn init_from_snapshot(&mut self, snapshot: StateSnapshot) {
        self.0.init_from_snapshot(snapshot)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SerializableState {
    /// The block number of the state
    ///
    /// Note: This is an Option for backwards compatibility: <https://github.com/foundry-rs/foundry/issues/5460>
    pub block: Option<BlockEnv>,
    pub accounts: BTreeMap<Address, SerializableAccountRecord>,
    /// The best block number of the state, can be different from block number (Arbitrum chain).
    pub best_block_number: Option<U64>,
    #[serde(default)]
    pub blocks: Vec<SerializableBlock>,
    #[serde(default)]
    pub transactions: Vec<SerializableTransaction>,
}

impl SerializableState {
    /// Loads the `Genesis` object from the given json file path
    pub fn load(path: impl AsRef<Path>) -> Result<Self, FsPathError> {
        let path = path.as_ref();
        if path.is_dir() {
            foundry_common::fs::read_json_file(&path.join("state.json"))
        } else {
            foundry_common::fs::read_json_file(path)
        }
    }

    /// This is used as the clap `value_parser` implementation
    pub(crate) fn parse(path: &str) -> Result<Self, String> {
        Self::load(path).map_err(|err| err.to_string())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SerializableAccountRecord {
    pub nonce: u64,
    pub balance: U256,
    pub code: Bytes,
    pub storage: BTreeMap<U256, U256>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SerializableBlock {
    pub header: Header,
    pub transactions: Vec<MaybeImpersonatedTransaction>,
    pub ommers: Vec<Header>,
}

impl From<Block> for SerializableBlock {
    fn from(block: Block) -> Self {
        Self {
            header: block.header,
            transactions: block.transactions.into_iter().map(Into::into).collect(),
            ommers: block.ommers.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<SerializableBlock> for Block {
    fn from(block: SerializableBlock) -> Self {
        Self {
            header: block.header,
            transactions: block.transactions.into_iter().map(Into::into).collect(),
            ommers: block.ommers.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SerializableTransaction {
    pub info: TransactionInfo,
    pub receipt: TypedReceipt,
    pub block_hash: B256,
    pub block_number: u64,
}

impl From<MinedTransaction> for SerializableTransaction {
    fn from(transaction: MinedTransaction) -> Self {
        Self {
            info: transaction.info,
            receipt: transaction.receipt,
            block_hash: transaction.block_hash,
            block_number: transaction.block_number,
        }
    }
}

impl From<SerializableTransaction> for MinedTransaction {
    fn from(transaction: SerializableTransaction) -> Self {
        Self {
            info: transaction.info,
            receipt: transaction.receipt,
            block_hash: transaction.block_hash,
            block_number: transaction.block_number,
        }
    }
}
