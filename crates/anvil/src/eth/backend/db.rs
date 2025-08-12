//! Helper types for working with [revm](foundry_evm::revm)

use std::{
    collections::BTreeMap,
    fmt::{self, Debug},
    path::Path,
};

use alloy_consensus::Header;
use alloy_primitives::{Address, B256, Bytes, U256, keccak256, map::HashMap};
use alloy_rpc_types::BlockId;
use anvil_core::eth::{
    block::Block,
    transaction::{MaybeImpersonatedTransaction, TransactionInfo, TypedReceipt, TypedTransaction},
};
use foundry_common::errors::FsPathError;
use foundry_evm::backend::{
    BlockchainDb, DatabaseError, DatabaseResult, MemDb, RevertStateSnapshotAction, StateSnapshot,
};
use revm::{
    Database, DatabaseCommit,
    bytecode::Bytecode,
    context::BlockEnv,
    context_interface::block::BlobExcessGasAndPrice,
    database::{CacheDB, DatabaseRef, DbAccount},
    primitives::{KECCAK_EMPTY, eip4844::BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE},
    state::AccountInfo,
};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{Error as DeError, MapAccess, Visitor},
};
use serde_json::Value;

use crate::mem::storage::MinedTransaction;

/// Helper trait get access to the full state data of the database
pub trait MaybeFullDatabase: DatabaseRef<Error = DatabaseError> + Debug {
    /// Returns a reference to the database as a `dyn DatabaseRef`.
    // TODO: Required until trait upcasting is stabilized: <https://github.com/rust-lang/rust/issues/65991>
    fn as_dyn(&self) -> &dyn DatabaseRef<Error = DatabaseError>;

    fn maybe_as_full_db(&self) -> Option<&HashMap<Address, DbAccount>> {
        None
    }

    /// Clear the state and move it into a new `StateSnapshot`.
    fn clear_into_state_snapshot(&mut self) -> StateSnapshot;

    /// Read the state snapshot.
    ///
    /// This clones all the states and returns a new `StateSnapshot`.
    fn read_as_state_snapshot(&self) -> StateSnapshot;

    /// Clears the entire database
    fn clear(&mut self);

    /// Reverses `clear_into_snapshot` by initializing the db's state with the state snapshot.
    fn init_from_state_snapshot(&mut self, state_snapshot: StateSnapshot);
}

impl<'a, T: 'a + MaybeFullDatabase + ?Sized> MaybeFullDatabase for &'a T
where
    &'a T: DatabaseRef<Error = DatabaseError>,
{
    fn as_dyn(&self) -> &dyn DatabaseRef<Error = DatabaseError> {
        T::as_dyn(self)
    }

    fn maybe_as_full_db(&self) -> Option<&HashMap<Address, DbAccount>> {
        T::maybe_as_full_db(self)
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        unreachable!("never called for DatabaseRef")
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        unreachable!("never called for DatabaseRef")
    }

    fn clear(&mut self) {}

    fn init_from_state_snapshot(&mut self, _state_snapshot: StateSnapshot) {}
}

/// Helper trait to reset the DB if it's forked
pub trait MaybeForkedDatabase {
    fn maybe_reset(&mut self, _url: Option<String>, block_number: BlockId) -> Result<(), String>;

    fn maybe_flush_cache(&self) -> Result<(), String>;

    fn maybe_inner(&self) -> Result<&BlockchainDb, String>;
}

/// This bundles all required revm traits
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

    /// Sets the code of the given address
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

    /// Sets the storage value at the given slot for the address
    fn set_storage_at(&mut self, address: Address, slot: B256, val: B256) -> DatabaseResult<()>;

    /// inserts a blockhash for the given number
    fn insert_block_hash(&mut self, number: U256, hash: B256);

    /// Write all chain data to serialized bytes buffer
    fn dump_state(
        &self,
        at: BlockEnv,
        best_number: u64,
        blocks: Vec<SerializableBlock>,
        transactions: Vec<SerializableTransaction>,
        historical_states: Option<SerializableHistoricalStates>,
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

    /// Creates a new state snapshot.
    fn snapshot_state(&mut self) -> U256;

    /// Reverts a state snapshot.
    ///
    /// Returns `true` if the state snapshot was reverted.
    fn revert_state(&mut self, state_snapshot: U256, action: RevertStateSnapshotAction) -> bool;

    /// Returns the state root if possible to compute
    fn maybe_state_root(&self) -> Option<B256> {
        None
    }

    /// Returns the current, standalone state of the Db
    fn current_state(&self) -> StateDb;
}

impl dyn Db {
    // TODO: Required until trait upcasting is stabilized: <https://github.com/rust-lang/rust/issues/65991>
    pub fn as_dbref(&self) -> &dyn DatabaseRef<Error = DatabaseError> {
        self.as_dyn()
    }
}

/// Convenience impl only used to use any `Db` on the fly as the db layer for revm's CacheDB
/// This is useful to create blocks without actually writing to the `Db`, but rather in the cache of
/// the `CacheDB` see also
/// [Backend::pending_block()](crate::eth::backend::mem::Backend::pending_block())
impl<T: DatabaseRef<Error = DatabaseError> + Send + Sync + Clone + fmt::Debug> Db for CacheDB<T> {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.insert_account_info(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: B256, val: B256) -> DatabaseResult<()> {
        self.insert_account_storage(address, slot.into(), val.into())
    }

    fn insert_block_hash(&mut self, number: U256, hash: B256) {
        self.cache.block_hashes.insert(number, hash);
    }

    fn dump_state(
        &self,
        _at: BlockEnv,
        _best_number: u64,
        _blocks: Vec<SerializableBlock>,
        _transaction: Vec<SerializableTransaction>,
        _historical_states: Option<SerializableHistoricalStates>,
    ) -> DatabaseResult<Option<SerializableState>> {
        Ok(None)
    }

    fn snapshot_state(&mut self) -> U256 {
        U256::ZERO
    }

    fn revert_state(&mut self, _state_snapshot: U256, _action: RevertStateSnapshotAction) -> bool {
        false
    }

    fn current_state(&self) -> StateDb {
        StateDb::new(MemDb::default())
    }
}

impl<T: DatabaseRef<Error = DatabaseError> + Debug> MaybeFullDatabase for CacheDB<T> {
    fn as_dyn(&self) -> &dyn DatabaseRef<Error = DatabaseError> {
        self
    }

    fn maybe_as_full_db(&self) -> Option<&HashMap<Address, DbAccount>> {
        Some(&self.cache.accounts)
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        let db_accounts = std::mem::take(&mut self.cache.accounts);
        let mut accounts = HashMap::default();
        let mut account_storage = HashMap::default();

        for (addr, mut acc) in db_accounts {
            account_storage.insert(addr, std::mem::take(&mut acc.storage));
            let mut info = acc.info;
            info.code = self.cache.contracts.remove(&info.code_hash);
            accounts.insert(addr, info);
        }
        let block_hashes = std::mem::take(&mut self.cache.block_hashes);
        StateSnapshot { accounts, storage: account_storage, block_hashes }
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        let db_accounts = self.cache.accounts.clone();
        let mut accounts = HashMap::default();
        let mut account_storage = HashMap::default();

        for (addr, acc) in db_accounts {
            account_storage.insert(addr, acc.storage.clone());
            let mut info = acc.info;
            info.code = self.cache.contracts.get(&info.code_hash).cloned();
            accounts.insert(addr, info);
        }

        let block_hashes = self.cache.block_hashes.clone();
        StateSnapshot { accounts, storage: account_storage, block_hashes }
    }

    fn clear(&mut self) {
        self.clear_into_state_snapshot();
    }

    fn init_from_state_snapshot(&mut self, state_snapshot: StateSnapshot) {
        let StateSnapshot { accounts, mut storage, block_hashes } = state_snapshot;

        for (addr, mut acc) in accounts {
            if let Some(code) = acc.code.take() {
                self.cache.contracts.insert(acc.code_hash, code);
            }
            self.cache.accounts.insert(
                addr,
                DbAccount {
                    info: acc,
                    storage: storage.remove(&addr).unwrap_or_default(),
                    ..Default::default()
                },
            );
        }
        self.cache.block_hashes = block_hashes;
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
#[derive(Debug)]
pub struct StateDb(pub(crate) Box<dyn MaybeFullDatabase + Send + Sync>);

impl StateDb {
    pub fn new(db: impl MaybeFullDatabase + Send + Sync + 'static) -> Self {
        Self(Box::new(db))
    }

    pub fn serialize_state(&mut self) -> StateSnapshot {
        // Using read_as_snapshot makes sures we don't clear the historical state from the current
        // instance.
        self.read_as_state_snapshot()
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
    fn as_dyn(&self) -> &dyn DatabaseRef<Error = DatabaseError> {
        self.0.as_dyn()
    }

    fn maybe_as_full_db(&self) -> Option<&HashMap<Address, DbAccount>> {
        self.0.maybe_as_full_db()
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        self.0.clear_into_state_snapshot()
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        self.0.read_as_state_snapshot()
    }

    fn clear(&mut self) {
        self.0.clear()
    }

    fn init_from_state_snapshot(&mut self, state_snapshot: StateSnapshot) {
        self.0.init_from_state_snapshot(state_snapshot)
    }
}

/// Legacy block environment from before v1.3.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LegacyBlockEnv {
    pub number: Option<StringOrU64>,
    #[serde(alias = "coinbase")]
    pub beneficiary: Option<Address>,
    pub timestamp: Option<StringOrU64>,
    pub gas_limit: Option<StringOrU64>,
    pub basefee: Option<StringOrU64>,
    pub difficulty: Option<StringOrU64>,
    pub prevrandao: Option<B256>,
    pub blob_excess_gas_and_price: Option<LegacyBlobExcessGasAndPrice>,
}

/// Legacy blob excess gas and price structure from before v1.3.
#[derive(Debug, Deserialize)]
pub struct LegacyBlobExcessGasAndPrice {
    pub excess_blob_gas: u64,
    pub blob_gasprice: u64,
}

/// Legacy string or u64 type from before v1.3.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum StringOrU64 {
    Hex(String),
    Dec(u64),
}

impl StringOrU64 {
    pub fn to_u64(&self) -> Option<u64> {
        match self {
            Self::Dec(n) => Some(*n),
            Self::Hex(s) => s.strip_prefix("0x").and_then(|s| u64::from_str_radix(s, 16).ok()),
        }
    }

    pub fn to_u256(&self) -> Option<U256> {
        match self {
            Self::Dec(n) => Some(U256::from(*n)),
            Self::Hex(s) => s.strip_prefix("0x").and_then(|s| U256::from_str_radix(s, 16).ok()),
        }
    }
}

/// Converts a `LegacyBlockEnv` to a `BlockEnv`, handling the conversion of legacy fields.
impl TryFrom<LegacyBlockEnv> for BlockEnv {
    type Error = &'static str;

    fn try_from(legacy: LegacyBlockEnv) -> Result<Self, Self::Error> {
        Ok(Self {
            number: legacy.number.and_then(|v| v.to_u256()).unwrap_or(U256::ZERO),
            beneficiary: legacy.beneficiary.unwrap_or(Address::ZERO),
            timestamp: legacy.timestamp.and_then(|v| v.to_u256()).unwrap_or(U256::ONE),
            gas_limit: legacy.gas_limit.and_then(|v| v.to_u64()).unwrap_or(u64::MAX),
            basefee: legacy.basefee.and_then(|v| v.to_u64()).unwrap_or(0),
            difficulty: legacy.difficulty.and_then(|v| v.to_u256()).unwrap_or(U256::ZERO),
            prevrandao: legacy.prevrandao.or(Some(B256::ZERO)),
            blob_excess_gas_and_price: legacy
                .blob_excess_gas_and_price
                .map(|v| BlobExcessGasAndPrice::new(v.excess_blob_gas, v.blob_gasprice))
                .or_else(|| {
                    Some(BlobExcessGasAndPrice::new(0, BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE))
                }),
        })
    }
}

/// Custom deserializer for `BlockEnv` that handles both v1.2 and v1.3+ formats.
fn deserialize_block_env_compat<'de, D>(deserializer: D) -> Result<Option<BlockEnv>, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Option<Value> = Option::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };

    if let Ok(env) = BlockEnv::deserialize(&value) {
        return Ok(Some(env));
    }

    let legacy: LegacyBlockEnv = serde_json::from_value(value).map_err(|e| {
        D::Error::custom(format!("Legacy deserialization of `BlockEnv` failed: {e}"))
    })?;

    Ok(Some(BlockEnv::try_from(legacy).map_err(D::Error::custom)?))
}

/// Custom deserializer for `best_block_number` that handles both v1.2 and v1.3+ formats.
fn deserialize_best_block_number_compat<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Option<Value> = Option::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };

    let number = match value {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => {
            if let Some(s) = s.strip_prefix("0x") {
                u64::from_str_radix(s, 16).ok()
            } else {
                s.parse().ok()
            }
        }
        _ => None,
    };

    Ok(number)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SerializableState {
    /// The block number of the state
    ///
    /// Note: This is an Option for backwards compatibility: <https://github.com/foundry-rs/foundry/issues/5460>
    #[serde(deserialize_with = "deserialize_block_env_compat")]
    pub block: Option<BlockEnv>,
    pub accounts: BTreeMap<Address, SerializableAccountRecord>,
    /// The best block number of the state, can be different from block number (Arbitrum chain).
    #[serde(deserialize_with = "deserialize_best_block_number_compat")]
    pub best_block_number: Option<u64>,
    #[serde(default)]
    pub blocks: Vec<SerializableBlock>,
    #[serde(default)]
    pub transactions: Vec<SerializableTransaction>,
    /// Historical states of accounts and storage at particular block hashes.
    ///
    /// Note: This is an Option for backwards compatibility.
    #[serde(default)]
    pub historical_states: Option<SerializableHistoricalStates>,
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
    #[allow(dead_code)]
    pub(crate) fn parse(path: &str) -> Result<Self, String> {
        Self::load(path).map_err(|err| err.to_string())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SerializableAccountRecord {
    pub nonce: u64,
    pub balance: U256,
    pub code: Bytes,

    #[serde(deserialize_with = "deserialize_btree")]
    pub storage: BTreeMap<B256, B256>,
}

fn deserialize_btree<'de, D>(deserializer: D) -> Result<BTreeMap<B256, B256>, D::Error>
where
    D: Deserializer<'de>,
{
    struct BTreeVisitor;

    impl<'de> Visitor<'de> for BTreeVisitor {
        type Value = BTreeMap<B256, B256>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a mapping of hex encoded storage slots to hex encoded state data")
        }

        fn visit_map<M>(self, mut mapping: M) -> Result<BTreeMap<B256, B256>, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut btree = BTreeMap::new();
            while let Some((key, value)) = mapping.next_entry::<U256, U256>()? {
                btree.insert(B256::from(key), B256::from(value));
            }

            Ok(btree)
        }
    }

    deserializer.deserialize_map(BTreeVisitor)
}

/// Defines a backwards-compatible enum for transactions.
/// This is essential for maintaining compatibility with state dumps
/// created before the changes introduced in PR #8411.
///
/// The enum can represent either a `TypedTransaction` or a `MaybeImpersonatedTransaction`,
/// depending on the data being deserialized. This flexibility ensures that older state
/// dumps can still be loaded correctly, even after the changes in #8411.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SerializableTransactionType {
    TypedTransaction(TypedTransaction),
    MaybeImpersonatedTransaction(MaybeImpersonatedTransaction),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SerializableBlock {
    pub header: Header,
    pub transactions: Vec<SerializableTransactionType>,
    pub ommers: Vec<Header>,
}

impl From<Block> for SerializableBlock {
    fn from(block: Block) -> Self {
        Self {
            header: block.header,
            transactions: block.transactions.into_iter().map(Into::into).collect(),
            ommers: block.ommers.into_iter().collect(),
        }
    }
}

impl From<SerializableBlock> for Block {
    fn from(block: SerializableBlock) -> Self {
        Self {
            header: block.header,
            transactions: block.transactions.into_iter().map(Into::into).collect(),
            ommers: block.ommers.into_iter().collect(),
        }
    }
}

impl From<MaybeImpersonatedTransaction> for SerializableTransactionType {
    fn from(transaction: MaybeImpersonatedTransaction) -> Self {
        Self::MaybeImpersonatedTransaction(transaction)
    }
}

impl From<SerializableTransactionType> for MaybeImpersonatedTransaction {
    fn from(transaction: SerializableTransactionType) -> Self {
        match transaction {
            SerializableTransactionType::TypedTransaction(tx) => Self::new(tx),
            SerializableTransactionType::MaybeImpersonatedTransaction(tx) => tx,
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

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SerializableHistoricalStates(Vec<(B256, StateSnapshot)>);

impl SerializableHistoricalStates {
    pub const fn new(states: Vec<(B256, StateSnapshot)>) -> Self {
        Self(states)
    }
}

impl IntoIterator for SerializableHistoricalStates {
    type Item = (B256, StateSnapshot);
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_deser_block() {
        let block = r#"{
            "header": {
                "parentHash": "0xceb0fe420d6f14a8eeec4319515b89acbb0bb4861cad9983d529ab4b1e4af929",
                "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
                "miner": "0x0000000000000000000000000000000000000000",
                "stateRoot": "0xe1423fd180478ab4fd05a7103277d64496b15eb914ecafe71eeec871b552efd1",
                "transactionsRoot": "0x2b5598ef261e5f88e4303bb2b3986b3d5c0ebf4cd9977daebccae82a6469b988",
                "receiptsRoot": "0xf78dfb743fbd92ade140711c8bbc542b5e307f0ab7984eff35d751969fe57efa",
                "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
                "difficulty": "0x0",
                "number": "0x2",
                "gasLimit": "0x1c9c380",
                "gasUsed": "0x5208",
                "timestamp": "0x66cdc823",
                "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                "nonce": "0x0000000000000000",
                "baseFeePerGas": "0x342a1c58",
                "blobGasUsed": "0x0",
                "excessBlobGas": "0x0",
                "extraData": "0x"
            },
            "transactions": [
                {
                    "EIP1559": {
                        "chainId": "0x7a69",
                        "nonce": "0x0",
                        "gas": "0x5209",
                        "maxFeePerGas": "0x77359401",
                        "maxPriorityFeePerGas": "0x1",
                        "to": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
                        "value": "0x0",
                        "accessList": [],
                        "input": "0x",
                        "r": "0x85c2794a580da137e24ccc823b45ae5cea99371ae23ee13860fcc6935f8305b0",
                        "s": "0x41de7fa4121dab284af4453d30928241208bafa90cdb701fe9bc7054759fe3cd",
                        "yParity": "0x0",
                        "hash": "0x8c9b68e8947ace33028dba167354fde369ed7bbe34911b772d09b3c64b861515"
                    }
                }
            ],
            "ommers": []
        }
        "#;

        let _block: SerializableBlock = serde_json::from_str(block).unwrap();
    }
}
