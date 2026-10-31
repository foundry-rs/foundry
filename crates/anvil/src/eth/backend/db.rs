//! Anvil's state databases.
//!
//! Anvil stores state in its own account model ([`DbAccount`]
//! in a [`Cache`]) over an optional remote fork. Execution reads it through [`EvmDb`], an evm2
//! [`DynDatabase`] adapter, and writes back the accumulated changes with [`Db::commit`].

use crate::mem::storage::MinedTransaction;
use alloy_consensus::BlockBody;
use alloy_eips::{eip4895::Withdrawals, eip7691::BLOB_GASPRICE_UPDATE_FRACTION_PECTRA};
use alloy_network::Network;
use alloy_primitives::{
    Address, B256, Bytes, U256, keccak256,
    map::{AddressMap, B256Map, U256Map},
};
use alloy_rpc_types::{BlockId, BlockOverrides, state::AccountOverride};
use anvil_core::eth::{
    block::Block,
    transaction::{MaybeImpersonatedTransaction, TransactionInfo},
};
use evm2::evm::{BlockStateAccumulator, DbResult, DynDatabase};
use foundry_common::errors::FsPathError;
use foundry_evm_core::evm::{FoundryDatabase, existing_account};
use foundry_primitives::{FoundryHeader, FoundryReceiptEnvelope, FoundryTxEnvelope};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as DeError, MapAccess, Visitor},
    ser::SerializeStruct,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fmt::{self, Debug},
    fs::File,
    io::BufReader,
    path::Path,
};

pub use super::env::{BlobExcessGasAndPrice, BlockEnv, SpecId};
pub use alloy_primitives::KECCAK256_EMPTY as KECCAK_EMPTY;
pub use evm2::{AccountInfo, bytecode::Bytecode};
pub use foundry_evm_core::{backend::RevertStateSnapshotAction, state_snapshot::StateSnapshots};
pub use foundry_fork_db::{DatabaseError, DatabaseResult};

/// The fork cache, keyed by Anvil's block environment.
pub type BlockchainDb = foundry_fork_db::BlockchainDb<BlockEnv>;

/// Metadata of the fork cache.
pub type BlockchainDbMeta = foundry_fork_db::cache::BlockchainDbMeta<BlockEnv>;

/// The RPC-backed fork cache handle.
pub type SharedBackend<N> = foundry_fork_db::SharedBackend<N, BlockEnv>;

/// Number of preceding block hashes available to the EVM's `BLOCKHASH` opcode.
pub(crate) const BLOCKHASH_HISTORY: u64 = 256;

/// Execution inputs needed to replay a locally stored Monad block faithfully.
#[cfg(feature = "monad")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonadBlockReplayProfile {
    /// Chain ID active when the block was executed.
    pub execution_chain_id: u64,
    /// Monad hardfork active when the block was executed.
    #[serde(with = "alloy_serde::displayfromstr")]
    pub hardfork: foundry_evm_hardforks::MonadHardfork,
}

/// Inserts a block hash, discards entries outside the EVM-visible cache, and returns its head.
pub(crate) fn cache_block_hash(block_hashes: &mut U256Map<B256>, number: U256, hash: B256) -> U256 {
    let head = block_hashes.keys().copied().max().map_or(number, |head| head.max(number));
    let min_number = head.saturating_sub(U256::from(BLOCKHASH_HISTORY));
    block_hashes.retain(|cached, _| *cached >= min_number && *cached <= head);
    if number >= min_number {
        block_hashes.insert(number, hash);
    }
    head
}

/// Lifecycle of a cached account.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccountState {
    /// The account is known not to exist: reads must not fall through to the backing database.
    NotExisting,
    /// The account was changed.
    Touched,
    /// The account's storage was cleared: slots missing from the cache are zero.
    StorageCleared,
    /// The account was loaded or inserted without changes.
    #[default]
    None,
}

impl AccountState {
    /// Returns whether slots missing from the cache are known to be zero.
    pub const fn is_storage_cleared(&self) -> bool {
        matches!(self, Self::StorageCleared | Self::NotExisting)
    }
}

/// A cached account with its storage.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DbAccount {
    /// Account info. The bytecode lives in [`Cache::contracts`].
    pub info: AccountInfo,
    /// Account lifecycle.
    pub account_state: AccountState,
    /// Cached storage slots.
    pub storage: U256Map<U256>,
}

impl DbAccount {
    /// Returns an account that is known not to exist.
    pub fn new_not_existing() -> Self {
        Self { account_state: AccountState::NotExisting, ..Default::default() }
    }

    /// Returns the account info, or `None` if the account does not exist.
    pub fn info(&self) -> Option<AccountInfo> {
        (self.account_state != AccountState::NotExisting).then(|| self.info.clone())
    }
}

impl From<AccountInfo> for DbAccount {
    fn from(info: AccountInfo) -> Self {
        Self { info, account_state: AccountState::None, storage: Default::default() }
    }
}

/// A minimal abstraction of a state at a certain point in time.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct StateSnapshot {
    /// Accounts with their bytecode.
    #[serde(deserialize_with = "deserialize_snapshot_accounts")]
    pub accounts: AddressMap<AccountInfo>,
    /// Storage by account.
    pub storage: AddressMap<U256Map<U256>>,
    /// Cached block hashes.
    pub block_hashes: U256Map<B256>,
    /// Accounts that are [`AccountState::NotExisting`] or [`AccountState::StorageCleared`], whose
    /// storage missing from `storage` is zero rather than read from the underlying database.
    ///
    /// `None` if the snapshot does not track account states, such as one saved by an older
    /// version.
    #[serde(default)]
    pub account_states: Option<AddressMap<AccountState>>,
}

impl StateSnapshot {
    /// Drops the account states, and with them the accounts known not to exist, for a state
    /// that nothing backs.
    pub fn into_standalone(mut self) -> Self {
        for (address, state) in self.account_states.take().unwrap_or_default() {
            if state == AccountState::NotExisting {
                self.accounts.remove(&address);
                self.storage.remove(&address);
            }
        }
        self
    }
}

impl Serialize for StateSnapshot {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let accounts = self
            .accounts
            .iter()
            .map(|(address, info)| (address, SnapshotAccount::from(info)))
            .collect::<BTreeMap<_, _>>();
        let storage = self
            .storage
            .iter()
            .map(|(address, storage)| (address, storage.iter().collect::<BTreeMap<_, _>>()))
            .collect::<BTreeMap<_, _>>();
        let block_hashes = self.block_hashes.iter().collect::<BTreeMap<_, _>>();
        let account_states = self
            .account_states
            .as_ref()
            .map(|account_states| account_states.iter().collect::<BTreeMap<_, _>>());

        let len = if account_states.is_some() { 4 } else { 3 };
        let mut state = serializer.serialize_struct("StateSnapshot", len)?;
        state.serialize_field("accounts", &accounts)?;
        state.serialize_field("storage", &storage)?;
        state.serialize_field("block_hashes", &block_hashes)?;
        if let Some(account_states) = account_states {
            state.serialize_field("account_states", &account_states)?;
        }
        state.end()
    }
}

/// Serialized account of a [`StateSnapshot`].
#[derive(Serialize, Deserialize)]
struct SnapshotAccount {
    balance: U256,
    nonce: u64,
    code_hash: B256,
    #[serde(default, deserialize_with = "deserialize_snapshot_code")]
    code: Option<Bytes>,
}

impl From<&AccountInfo> for SnapshotAccount {
    fn from(info: &AccountInfo) -> Self {
        Self {
            balance: info.balance,
            nonce: info.nonce,
            code_hash: info.code_hash,
            code: info.code.as_ref().map(Bytecode::original_bytes),
        }
    }
}

impl From<SnapshotAccount> for AccountInfo {
    fn from(account: SnapshotAccount) -> Self {
        let code = account.code.filter(|code| !code.is_empty()).map(Bytecode::new_raw);
        Self::new(account.balance, account.nonce, account.code_hash, code)
    }
}

fn deserialize_snapshot_accounts<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<AddressMap<AccountInfo>, D::Error> {
    let accounts = AddressMap::<SnapshotAccount>::deserialize(deserializer)?;
    Ok(accounts.into_iter().map(|(address, account)| (address, account.into())).collect())
}

/// Deserializes bytecode written as raw bytes, or in the tagged form of dumps written before the
/// evm2 migration.
fn deserialize_snapshot_code<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Bytes>, D::Error> {
    #[derive(Deserialize)]
    struct LegacyAnalyzed {
        bytecode: Bytes,
        original_len: usize,
    }

    #[derive(Deserialize)]
    struct Eip7702 {
        raw: Bytes,
    }

    #[derive(Deserialize)]
    enum Tagged {
        LegacyAnalyzed(LegacyAnalyzed),
        LegacyRaw(Bytes),
        Eip7702(Eip7702),
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Code {
        Raw(Bytes),
        Tagged(Tagged),
    }

    Ok(Option::<Code>::deserialize(deserializer)?.map(|code| match code {
        Code::Raw(bytes) | Code::Tagged(Tagged::LegacyRaw(bytes)) => bytes,
        Code::Tagged(Tagged::LegacyAnalyzed(code)) => {
            code.bytecode.slice(..code.original_len.min(code.bytecode.len()))
        }
        Code::Tagged(Tagged::Eip7702(code)) => code.raw,
    }))
}

/// Read access to Anvil state.
///
/// Like REVM's `DatabaseRef`, a missing account of a local database is reported as an existing
/// empty account; [`EvmDb`] maps it to evm2's absent account where execution requires it.
#[auto_impl::auto_impl(&, Box)]
pub trait DatabaseRef: Debug {
    /// Returns the account at `address`, with its bytecode when available.
    fn basic_ref(&self, address: Address) -> DatabaseResult<Option<AccountInfo>>;

    /// Returns the bytecode with `code_hash`.
    fn code_by_hash_ref(&self, code_hash: B256) -> DatabaseResult<Bytecode>;

    /// Returns the storage slot `index` of `address`.
    fn storage_ref(&self, address: Address, index: U256) -> DatabaseResult<U256>;

    /// Returns the hash of block `number`.
    fn block_hash_ref(&self, number: u64) -> DatabaseResult<B256>;
}

/// The empty backing database of local state: every account exists and is empty.
#[derive(Clone, Copy, Debug, Default)]
pub struct EmptyDb;

impl DatabaseRef for EmptyDb {
    fn basic_ref(&self, _address: Address) -> DatabaseResult<Option<AccountInfo>> {
        Ok(Some(AccountInfo::default()))
    }

    fn code_by_hash_ref(&self, _code_hash: B256) -> DatabaseResult<Bytecode> {
        Ok(Bytecode::default())
    }

    fn storage_ref(&self, _address: Address, _index: U256) -> DatabaseResult<U256> {
        Ok(U256::ZERO)
    }

    fn block_hash_ref(&self, number: u64) -> DatabaseResult<B256> {
        Ok(keccak256(number.to_string().as_bytes()))
    }
}

/// Cached state of a [`CacheDB`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cache {
    /// Accounts by address.
    pub accounts: AddressMap<DbAccount>,
    /// Bytecode by code hash.
    pub contracts: B256Map<Bytecode>,
    /// Block hashes by number.
    pub block_hashes: U256Map<B256>,
}

impl Default for Cache {
    fn default() -> Self {
        let mut contracts = B256Map::default();
        contracts.insert(KECCAK_EMPTY, Bytecode::default());
        contracts.insert(B256::ZERO, Bytecode::default());
        Self { accounts: Default::default(), contracts, block_hashes: Default::default() }
    }
}

/// State cached over a read-only backing database.
#[derive(Clone, Debug, Default)]
pub struct CacheDB<ExtDB> {
    /// Cached state, authoritative over `db`.
    pub cache: Cache,
    /// Backing database.
    pub db: ExtDB,
}

impl<ExtDB> CacheDB<ExtDB> {
    /// Creates an empty cache over `db`.
    pub fn new(db: ExtDB) -> Self {
        Self { cache: Cache::default(), db }
    }

    /// Stores the bytecode of `info` and normalizes its code hash.
    pub fn insert_contract(&mut self, info: &mut AccountInfo) {
        if let Some(code) = &info.code
            && !code.is_empty()
        {
            if info.code_hash == KECCAK_EMPTY {
                info.code_hash = code.hash_slow();
            }
            self.cache.contracts.entry(info.code_hash).or_insert_with(|| code.clone());
        }
        if info.code_hash.is_zero() {
            info.code_hash = KECCAK_EMPTY;
        }
    }

    /// Inserts account info, keeping the account's cached storage.
    pub fn insert_account_info(&mut self, address: Address, mut info: AccountInfo) {
        self.insert_contract(&mut info);
        let account = self.cache.accounts.entry(address).or_default();
        account.info = info;
        if account.account_state == AccountState::NotExisting {
            account.account_state = AccountState::StorageCleared;
        }
    }
}

impl<ExtDB: DatabaseRef> CacheDB<ExtDB> {
    /// Loads `address` into the cache and returns it.
    pub fn load_account(&mut self, address: Address) -> DatabaseResult<&mut DbAccount> {
        if !self.cache.accounts.contains_key(&address) {
            let account = match self.db.basic_ref(address)? {
                Some(mut info) => {
                    self.insert_contract(&mut info);
                    DbAccount::from(info)
                }
                None => DbAccount::new_not_existing(),
            };
            self.cache.accounts.insert(address, account);
        }
        Ok(self.cache.accounts.get_mut(&address).expect("account was just loaded"))
    }

    /// Inserts a storage slot, loading the account first.
    pub fn insert_account_storage(
        &mut self,
        address: Address,
        slot: U256,
        value: U256,
    ) -> DatabaseResult<()> {
        let account = self.load_account(address)?;
        if account.account_state == AccountState::NotExisting {
            account.account_state = AccountState::StorageCleared;
        }
        account.storage.insert(slot, value);
        Ok(())
    }

    /// Applies state changes accumulated by evm2 to the cache.
    ///
    /// Changed accounts are loaded from the backing database first, so an account whose storage
    /// alone changed keeps its info.
    pub fn commit(&mut self, changes: &BlockStateAccumulator) {
        let addresses = changes
            .accounts()
            .map(|(address, _)| address)
            .chain(changes.storage().map(|(key, _)| key.address()))
            .chain(changes.storage_wipes())
            .collect::<Vec<_>>();
        for address in addresses {
            // A failed load already failed the execution that produced the change.
            let _ = self.load_account(address);
        }
        commit_changes(&mut self.cache, changes);
    }
}

/// Applies `eth_call` state overrides to `db`.
///
/// `state` replaces the account's storage and `stateDiff` updates individual slots. Precompile
/// moves are applied by the EVM, not here.
pub fn apply_state_overrides<ExtDB: DatabaseRef>(
    overrides: impl IntoIterator<Item = (Address, AccountOverride)>,
    db: &mut CacheDB<ExtDB>,
) -> Result<(), crate::eth::error::BlockchainError> {
    use crate::eth::error::BlockchainError;

    for (address, account_override) in overrides {
        let mut info = db.load_account(address)?.info.clone();
        if let Some(nonce) = account_override.nonce {
            info.nonce = nonce;
        }
        if let Some(code) = account_override.code {
            let code = Bytecode::new_raw_checked(code)
                .map_err(|err| BlockchainError::StateOverrideError(err.to_string()))?;
            info.code_hash = code.hash_slow();
            info.code = Some(code);
        }
        if let Some(balance) = account_override.balance {
            info.balance = balance;
        }
        db.insert_account_info(address, info);
        match (account_override.state, account_override.state_diff) {
            (Some(_), Some(_)) => {
                return Err(BlockchainError::StateOverrideError(format!(
                    "state and state_diff can't be used together for account {address}"
                )));
            }
            (Some(state), None) => {
                let account = db.load_account(address)?;
                account.account_state = AccountState::StorageCleared;
                account.storage = state
                    .into_iter()
                    .map(|(slot, value)| {
                        (U256::from_be_bytes(slot.0), U256::from_be_bytes(value.0))
                    })
                    .collect();
            }
            (None, Some(state_diff)) => {
                for (slot, value) in state_diff {
                    db.insert_account_storage(
                        address,
                        U256::from_be_bytes(slot.0),
                        U256::from_be_bytes(value.0),
                    )?;
                }
            }
            (None, None) => {}
        }
    }
    Ok(())
}

/// Applies `eth_call` block overrides to `block_env` and overrides block hashes in `db`.
pub fn apply_block_overrides<ExtDB>(
    overrides: BlockOverrides,
    db: &mut CacheDB<ExtDB>,
    block_env: &mut BlockEnv,
) {
    let BlockOverrides {
        number,
        difficulty,
        time,
        gas_limit,
        coinbase,
        random,
        base_fee,
        blob_base_fee,
        block_hash,
        ..
    } = overrides;
    if let Some(block_hashes) = block_hash {
        db.cache
            .block_hashes
            .extend(block_hashes.into_iter().map(|(number, hash)| (U256::from(number), hash)));
    }
    if let Some(number) = number {
        block_env.number = number;
    }
    if let Some(difficulty) = difficulty {
        block_env.difficulty = difficulty;
    }
    if let Some(time) = time {
        block_env.timestamp = U256::from(time);
    }
    if let Some(gas_limit) = gas_limit {
        block_env.gas_limit = gas_limit;
    }
    if let Some(coinbase) = coinbase {
        block_env.beneficiary = coinbase;
    }
    if let Some(random) = random {
        block_env.prevrandao = Some(random);
    }
    if let Some(base_fee) = base_fee {
        block_env.basefee = base_fee.saturating_to();
    }
    if let Some(blob_base_fee) = blob_base_fee {
        let excess_blob_gas = block_env.blob_excess_gas().unwrap_or_default();
        block_env.blob_excess_gas_and_price = Some(BlobExcessGasAndPrice {
            excess_blob_gas,
            blob_gasprice: blob_base_fee.saturating_to(),
        });
    }
}

/// Applies evm2 state changes to a [`Cache`].
///
/// A created or deleted account starts from empty storage: evm2 drops the storage wipe marker of
/// accounts that did not exist before the block, so it is applied here before storage writes.
pub(crate) fn commit_changes(cache: &mut Cache, changes: &BlockStateAccumulator) {
    fn wipe(account: &mut DbAccount) {
        account.storage.clear();
        account.account_state = AccountState::StorageCleared;
    }

    for (code_hash, code) in changes.code() {
        cache.contracts.entry(*code_hash).or_insert_with(|| code.clone());
    }
    for (address, delta) in changes.accounts() {
        if delta.original.is_none() || delta.current.is_none() {
            wipe(cache.accounts.entry(address).or_default());
        }
    }
    for address in changes.storage_wipes() {
        wipe(cache.accounts.entry(address).or_default());
    }
    for (key, delta) in changes.storage() {
        let account = cache.accounts.entry(key.address()).or_default();
        account.storage.insert(key.key(), delta.current);
        if account.account_state == AccountState::None {
            account.account_state = AccountState::Touched;
        }
    }
    for (address, delta) in changes.accounts() {
        let account = cache.accounts.entry(address).or_default();
        match &delta.current {
            Some(info) => {
                let mut info = info.clone();
                if let Some(code) = info.code.take()
                    && !code.is_empty()
                {
                    cache.contracts.entry(info.code_hash).or_insert(code);
                }
                account.info = info;
                if account.account_state == AccountState::None {
                    account.account_state = AccountState::Touched;
                }
            }
            None => *account = DbAccount::new_not_existing(),
        }
    }
}

impl<ExtDB: DatabaseRef> DatabaseRef for CacheDB<ExtDB> {
    fn basic_ref(&self, address: Address) -> DatabaseResult<Option<AccountInfo>> {
        match self.cache.accounts.get(&address) {
            Some(account) => Ok(account.info().map(|mut info| {
                if info.code.is_none() {
                    info.code = self.cache.contracts.get(&info.code_hash).cloned();
                }
                info
            })),
            None => self.db.basic_ref(address),
        }
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> DatabaseResult<Bytecode> {
        match self.cache.contracts.get(&code_hash) {
            Some(code) => Ok(code.clone()),
            None => self.db.code_by_hash_ref(code_hash),
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> DatabaseResult<U256> {
        match self.cache.accounts.get(&address) {
            Some(account) => match account.storage.get(&index) {
                Some(value) => Ok(*value),
                None if account.account_state.is_storage_cleared() => Ok(U256::ZERO),
                None => self.db.storage_ref(address, index),
            },
            None => self.db.storage_ref(address, index),
        }
    }

    fn block_hash_ref(&self, number: u64) -> DatabaseResult<B256> {
        match self.cache.block_hashes.get(&U256::from(number)) {
            Some(hash) => Ok(*hash),
            None => self.db.block_hash_ref(number),
        }
    }
}

/// Reads Anvil state as an evm2 backing database.
///
/// Empty accounts are loaded as absent from Spurious Dragon on, see [`existing_account`].
#[derive(Debug)]
pub struct EvmDb<'a> {
    db: &'a dyn DatabaseRef,
    spec: SpecId,
    forked: Option<u64>,
}

impl<'a> EvmDb<'a> {
    /// Reads `db` for execution at `spec`, as a fork of `forked` if set.
    pub fn new(db: &'a dyn DatabaseRef, spec: SpecId, forked: Option<u64>) -> Self {
        Self { db, spec, forked }
    }
}

/// Converts an Anvil database error into an evm2 database error.
pub fn evm_db_err(err: DatabaseError) -> evm2::DatabaseError {
    evm2::DatabaseError::new(err, true)
}

impl DynDatabase for EvmDb<'_> {
    fn get_account(&mut self, address: &Address) -> DbResult<Option<AccountInfo>> {
        let account = self.db.basic_ref(*address).map_err(evm_db_err)?;
        // Anvil's databases, like forked state, report an account they do not hold as an existing
        // empty account.
        Ok(existing_account(self.spec >= SpecId::SPURIOUS_DRAGON, account))
    }

    fn get_code_by_hash(&mut self, code_hash: &B256) -> DbResult<Bytecode> {
        self.db.code_by_hash_ref(*code_hash).map_err(evm_db_err)
    }

    fn get_storage(&mut self, address: &Address, key: &U256) -> DbResult<U256> {
        self.db.storage_ref(*address, *key).map_err(evm_db_err)
    }

    fn get_block_hash(&mut self, number: &U256) -> DbResult<B256> {
        self.db.block_hash_ref(number.saturating_to()).map_err(evm_db_err)
    }
}

impl FoundryDatabase for EvmDb<'_> {
    fn is_forked_mode(&self) -> bool {
        self.forked.is_some()
    }

    fn active_fork_source_chain_id(&self) -> Option<u64> {
        self.forked
    }
}

/// Helper trait get access to the full state data of the database
pub trait MaybeFullDatabase: DatabaseRef {
    fn maybe_as_full_db(&self) -> Option<&AddressMap<DbAccount>> {
        None
    }

    fn maybe_as_full_db_mut(&mut self) -> Option<&mut AddressMap<DbAccount>> {
        None
    }

    /// Returns an owned, recursively merged view of all available accounts.
    fn maybe_full_db(&self) -> Option<AddressMap<DbAccount>> {
        self.maybe_as_full_db().cloned()
    }

    /// Returns whether snapshots of this database use structural sharing.
    fn is_persistent(&self) -> bool {
        false
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

impl<'a, T: 'a + MaybeFullDatabase + ?Sized> MaybeFullDatabase for &'a T {
    fn maybe_as_full_db(&self) -> Option<&AddressMap<DbAccount>> {
        T::maybe_as_full_db(self)
    }

    fn maybe_full_db(&self) -> Option<AddressMap<DbAccount>> {
        T::maybe_full_db(self)
    }

    fn is_persistent(&self) -> bool {
        T::is_persistent(self)
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        unreachable!("never called for DatabaseRef")
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        T::read_as_state_snapshot(self)
    }

    fn clear(&mut self) {}

    fn init_from_state_snapshot(&mut self, _state_snapshot: StateSnapshot) {}
}

impl<T: MaybeFullDatabase + ?Sized> MaybeFullDatabase for Box<T> {
    fn maybe_as_full_db(&self) -> Option<&AddressMap<DbAccount>> {
        T::maybe_as_full_db(self)
    }

    fn maybe_as_full_db_mut(&mut self) -> Option<&mut AddressMap<DbAccount>> {
        T::maybe_as_full_db_mut(self)
    }

    fn maybe_full_db(&self) -> Option<AddressMap<DbAccount>> {
        T::maybe_full_db(self)
    }

    fn is_persistent(&self) -> bool {
        T::is_persistent(self)
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        T::clear_into_state_snapshot(self)
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        T::read_as_state_snapshot(self)
    }

    fn clear(&mut self) {
        T::clear(self)
    }

    fn init_from_state_snapshot(&mut self, state_snapshot: StateSnapshot) {
        T::init_from_state_snapshot(self, state_snapshot)
    }
}

/// Helper trait to reset the DB if it's forked
pub trait MaybeForkedDatabase {
    fn maybe_reset(&mut self, _urls: Vec<String>, block_number: BlockId) -> Result<(), String>;

    fn maybe_flush_cache(&self) -> Result<(), String>;

    fn maybe_inner(&self) -> Result<&BlockchainDb, String>;
}

/// Anvil's mutable state database.
pub trait Db: MaybeFullDatabase + MaybeForkedDatabase + Send + Sync {
    /// Inserts an account
    fn insert_account(&mut self, address: Address, account: AccountInfo);

    /// Sets the nonce of the given address
    fn set_nonce(&mut self, address: Address, nonce: u64) -> DatabaseResult<()> {
        let mut info = self.basic_ref(address)?.unwrap_or_default();
        info.nonce = nonce;
        self.insert_account(address, info);
        Ok(())
    }

    /// Sets the balance of the given address
    fn set_balance(&mut self, address: Address, balance: U256) -> DatabaseResult<()> {
        let mut info = self.basic_ref(address)?.unwrap_or_default();
        info.balance = balance;
        self.insert_account(address, info);
        Ok(())
    }

    /// Sets the code of the given address
    fn set_code(&mut self, address: Address, code: Bytes) -> DatabaseResult<()> {
        let mut info = self.basic_ref(address)?.unwrap_or_default();
        if code.is_empty() {
            info.code_hash = KECCAK_EMPTY;
            info.code = None;
        } else {
            info.code_hash = keccak256(&code);
            info.code = Some(Bytecode::new_raw(code));
        }
        self.insert_account(address, info);
        Ok(())
    }

    /// Sets the storage value at the given slot for the address
    fn set_storage_at(&mut self, address: Address, slot: B256, val: B256) -> DatabaseResult<()>;

    /// inserts a blockhash for the given number
    fn insert_block_hash(&mut self, number: U256, hash: B256);

    /// Replaces all cached block hashes.
    fn set_block_hashes(&mut self, block_hashes: Vec<(U256, B256)>);

    /// Applies state changes accumulated by evm2.
    fn commit(&mut self, changes: &BlockStateAccumulator);

    /// Write all chain data to serialized bytes buffer
    fn dump_state(
        &self,
        at: BlockEnv,
        best_number: u64,
        blocks: Vec<SerializableBlock>,
        transactions: Vec<SerializableTransaction>,
        historical_states: Option<SerializableHistoricalStates>,
    ) -> DatabaseResult<Option<SerializableState>>;

    /// Deserialize and add all accounts to the backend storage.
    fn load_state(&mut self, state: SerializableState) -> DatabaseResult<bool> {
        for (addr, account) in state.accounts {
            let old_account_nonce = self.basic_ref(addr)?.map(|acc| acc.nonce).unwrap_or_default();
            // use max nonce in case account is imported multiple times with difference
            // nonces to prevent collisions
            let nonce = std::cmp::max(old_account_nonce, account.nonce);

            self.insert_account(
                addr,
                AccountInfo::new(
                    account.balance,
                    nonce,
                    // set automatically from the code
                    KECCAK_EMPTY,
                    (!account.code.is_empty()).then(|| Bytecode::new_raw(account.code)),
                ),
            );

            for (k, v) in account.storage {
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

    /// Deletes a state snapshot without reverting it.
    fn delete_state_snapshot(&mut self, state_snapshot: U256) -> bool;

    /// Returns the state root if possible to compute
    fn maybe_state_root(&self) -> Option<B256> {
        None
    }

    /// Returns the current, standalone state of the Db
    fn current_state(&self) -> StateDb;

    /// Rebuilds a historical state from a serialized state snapshot.
    fn historical_state(&self, state_snapshot: StateSnapshot) -> StateDb {
        let mut state = StateDb::new(crate::mem::in_memory_db::MemDb::default());
        state.init_from_state_snapshot(state_snapshot);
        state
    }
}

/// Convenience impl only used to use any `Db` on the fly as the db layer of a [`CacheDB`].
///
/// This is useful to create blocks without actually writing to the `Db`, but rather in the cache of
/// the `CacheDB` see also
/// [Backend::pending_block()](crate::eth::backend::mem::Backend::pending_block())
impl<T> Db for CacheDB<T>
where
    T: MaybeFullDatabase + Send + Sync + Clone,
{
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.insert_account_info(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: B256, val: B256) -> DatabaseResult<()> {
        self.insert_account_storage(address, slot.into(), val.into())
    }

    fn insert_block_hash(&mut self, number: U256, hash: B256) {
        cache_block_hash(&mut self.cache.block_hashes, number, hash);
    }

    fn set_block_hashes(&mut self, block_hashes: Vec<(U256, B256)>) {
        self.cache.block_hashes = block_hashes.into_iter().collect();
    }

    fn commit(&mut self, changes: &BlockStateAccumulator) {
        Self::commit(self, changes)
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

    fn delete_state_snapshot(&mut self, _state_snapshot: U256) -> bool {
        false
    }

    fn maybe_state_root(&self) -> Option<B256> {
        self.maybe_full_db().map(|accounts| crate::mem::state::state_root(&accounts))
    }

    fn current_state(&self) -> StateDb {
        StateDb::new(crate::mem::in_memory_db::MemDb::default())
    }
}

impl<T: MaybeFullDatabase> MaybeFullDatabase for CacheDB<T> {
    fn maybe_as_full_db(&self) -> Option<&AddressMap<DbAccount>> {
        Some(&self.cache.accounts)
    }

    fn maybe_as_full_db_mut(&mut self) -> Option<&mut AddressMap<DbAccount>> {
        Some(&mut self.cache.accounts)
    }

    fn maybe_full_db(&self) -> Option<AddressMap<DbAccount>> {
        let mut accounts = self.db.maybe_full_db()?;
        for (address, overlay) in &self.cache.accounts {
            if overlay.account_state == AccountState::NotExisting {
                accounts.remove(address);
                continue;
            }
            if overlay.account_state.is_storage_cleared() {
                accounts.insert(*address, overlay.clone());
                continue;
            }

            let mut account = accounts.remove(address).unwrap_or_default();
            account.info = overlay.info.clone();
            account.account_state = overlay.account_state.clone();
            account.storage.extend(overlay.storage.clone());
            accounts.insert(*address, account);
        }
        Some(accounts)
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        let state_snapshot = self.read_as_state_snapshot();
        self.cache = Cache::default();
        state_snapshot
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        let mut accounts = AddressMap::default();
        let mut account_storage = AddressMap::default();
        let mut account_states = AddressMap::default();

        for (addr, acc) in &self.cache.accounts {
            account_storage.insert(*addr, acc.storage.clone());
            if matches!(acc.account_state, AccountState::NotExisting | AccountState::StorageCleared)
            {
                account_states.insert(*addr, acc.account_state.clone());
            }
            let mut info = acc.info.clone();
            info.code = self.cache.contracts.get(&info.code_hash).cloned();
            accounts.insert(*addr, info);
        }

        let block_hashes = self.cache.block_hashes.clone();
        StateSnapshot {
            accounts,
            storage: account_storage,
            block_hashes,
            account_states: Some(account_states),
        }
    }

    fn clear(&mut self) {
        self.cache = Cache::default();
    }

    fn init_from_state_snapshot(&mut self, state_snapshot: StateSnapshot) {
        let StateSnapshot { accounts, mut storage, block_hashes, account_states } = state_snapshot;
        let mut account_states = account_states.unwrap_or_default();

        for (addr, mut acc) in accounts {
            if let Some(code) = acc.code.take() {
                self.cache.contracts.insert(acc.code_hash, code);
            }
            self.cache.accounts.insert(
                addr,
                DbAccount {
                    info: acc,
                    account_state: account_states.remove(&addr).unwrap_or_default(),
                    storage: storage.remove(&addr).unwrap_or_default(),
                },
            );
        }
        self.cache.block_hashes = block_hashes;
    }
}

impl MaybeFullDatabase for EmptyDb {
    fn maybe_full_db(&self) -> Option<AddressMap<DbAccount>> {
        Some(AddressMap::default())
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        StateSnapshot::default()
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        StateSnapshot::default()
    }

    fn clear(&mut self) {}

    fn init_from_state_snapshot(&mut self, _state_snapshot: StateSnapshot) {}
}

impl<T> MaybeForkedDatabase for CacheDB<T> {
    fn maybe_reset(&mut self, _urls: Vec<String>, _block_number: BlockId) -> Result<(), String> {
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
    fn maybe_as_full_db(&self) -> Option<&AddressMap<DbAccount>> {
        self.0.maybe_as_full_db()
    }

    fn maybe_full_db(&self) -> Option<AddressMap<DbAccount>> {
        self.0.maybe_full_db()
    }

    fn is_persistent(&self) -> bool {
        self.0.is_persistent()
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
    pub blob_excess_gas_and_price: Option<BlobExcessGasAndPrice>,
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
            slot_num: 0,
            blob_excess_gas_and_price: legacy.blob_excess_gas_and_price.or_else(|| {
                Some(BlobExcessGasAndPrice::new(0, BLOB_GASPRICE_UPDATE_FRACTION_PECTRA as u64))
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
    /// Authoritative Monad senders and EIP-7702 authorities for locally stored blocks.
    ///
    /// This metadata can differ from transaction-body recovery when signature impersonation was
    /// used, so it is preserved even while the corresponding transaction bodies are retained.
    #[cfg(feature = "monad")]
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub monad_block_participants: BTreeMap<B256, std::collections::BTreeSet<Address>>,
    /// Execution profile used for each locally stored Monad block.
    #[cfg(feature = "monad")]
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub monad_block_replay_profiles: BTreeMap<B256, MonadBlockReplayProfile>,
    /// Historical states of accounts and storage at particular block hashes.
    ///
    /// Note: This is an Option for backwards compatibility.
    #[serde(default)]
    pub historical_states: Option<SerializableHistoricalStates>,
}

impl SerializableState {
    /// Loads the `Genesis` object from the given json file path
    pub fn load(path: impl AsRef<Path>) -> Result<Self, FsPathError> {
        let mut path = path.as_ref().to_path_buf();
        if path.is_dir() {
            path = path.join("state.json");
        }

        let file = File::open(&path).map_err(|err| FsPathError::read(err, &path))?;
        serde_json::from_reader(BufReader::new(file)).map_err(|err| {
            if err.is_io() {
                FsPathError::read(err.into(), &path)
            } else {
                FsPathError::ReadJson { source: err, path }
            }
        })
    }

    /// This is used as the clap `value_parser` implementation
    #[cfg(feature = "cmd")]
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
    TypedTransaction(FoundryTxEnvelope),
    MaybeImpersonatedTransaction(MaybeImpersonatedTransaction<FoundryTxEnvelope>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SerializableBlock {
    pub header: FoundryHeader,
    pub transactions: Vec<SerializableTransactionType>,
    pub ommers: Vec<FoundryHeader>,
    #[serde(default)]
    pub withdrawals: Option<Withdrawals>,
}

impl From<Block> for SerializableBlock {
    fn from(block: Block) -> Self {
        Self {
            header: block.header,
            transactions: block.body.transactions.into_iter().map(Into::into).collect(),
            ommers: block.body.ommers.into_iter().collect(),
            withdrawals: block.body.withdrawals,
        }
    }
}

impl From<SerializableBlock> for Block {
    fn from(block: SerializableBlock) -> Self {
        let transactions = block.transactions.into_iter().map(Into::into).collect();
        let ommers = block.ommers;
        let body = BlockBody { transactions, ommers, withdrawals: block.withdrawals };
        Self::new(block.header, body)
    }
}

impl From<MaybeImpersonatedTransaction<FoundryTxEnvelope>> for SerializableTransactionType {
    fn from(transaction: MaybeImpersonatedTransaction<FoundryTxEnvelope>) -> Self {
        Self::MaybeImpersonatedTransaction(transaction)
    }
}

impl From<SerializableTransactionType> for MaybeImpersonatedTransaction<FoundryTxEnvelope> {
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
    pub receipt: FoundryReceiptEnvelope,
    pub block_hash: B256,
    pub block_number: u64,
}

impl<N: Network<ReceiptEnvelope = FoundryReceiptEnvelope>> From<MinedTransaction<N>>
    for SerializableTransaction
{
    fn from(transaction: MinedTransaction<N>) -> Self {
        Self {
            info: transaction.info,
            receipt: transaction.receipt,
            block_hash: transaction.block_hash,
            block_number: transaction.block_number,
        }
    }
}

impl<N: Network<ReceiptEnvelope = FoundryReceiptEnvelope>> From<SerializableTransaction>
    for MinedTransaction<N>
{
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
    use crate::eth::backend::mem::{fork_db::ForkedDatabase, in_memory_db::MemDb};
    use alloy_consensus::Header;
    use alloy_network::AnyNetwork;
    use alloy_provider::ProviderBuilder;
    use alloy_transport::mock::Asserter;
    use std::fs;

    #[test]
    fn loads_state_from_file_or_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("state.json");
        fs::write(&state_path, serde_json::to_vec(&SerializableState::default()).unwrap()).unwrap();

        assert!(SerializableState::load(&state_path).unwrap().accounts.is_empty());
        assert!(SerializableState::load(tmp.path()).unwrap().accounts.is_empty());

        fs::write(&state_path, b"not json").unwrap();
        let Err(FsPathError::ReadJson { path, .. }) = SerializableState::load(tmp.path()) else {
            panic!("expected invalid JSON error")
        };
        assert_eq!(path, state_path);

        let missing_path = tmp.path().join("missing.json");
        let Err(FsPathError::Read { path, .. }) = SerializableState::load(&missing_path) else {
            panic!("expected file read error")
        };
        assert_eq!(path, missing_path);
    }

    #[test]
    fn cache_db_full_state_merges_base_and_overlay() {
        let preserved = Address::with_last_byte(1);
        let updated = Address::with_last_byte(2);
        let deleted = Address::with_last_byte(3);
        let cleared = Address::with_last_byte(4);
        let deleted_slot = U256::ONE;
        let updated_slot = U256::from(2);

        let mut base = MemDb::default();
        base.insert_account(preserved, AccountInfo::default().with_balance(U256::ONE));
        base.insert_account(updated, AccountInfo::default().with_balance(U256::from(2)));
        base.set_storage_at(updated, deleted_slot.into(), B256::with_last_byte(10)).unwrap();
        base.set_storage_at(updated, updated_slot.into(), B256::with_last_byte(11)).unwrap();
        base.insert_account(deleted, AccountInfo::default().with_balance(U256::from(3)));
        base.insert_account(cleared, AccountInfo::default().with_balance(U256::from(4)));
        base.set_storage_at(cleared, deleted_slot.into(), B256::with_last_byte(11)).unwrap();

        let mut cache = CacheDB::new(base);
        cache.insert_account_info(updated, AccountInfo::default().with_balance(U256::from(20)));
        cache.insert_account_storage(updated, deleted_slot, U256::ZERO).unwrap();
        cache.insert_account_storage(updated, updated_slot, U256::from(12)).unwrap();
        cache.cache.accounts.insert(deleted, DbAccount::new_not_existing());
        cache.cache.accounts.insert(
            cleared,
            DbAccount {
                info: AccountInfo::default().with_balance(U256::from(40)),
                account_state: AccountState::StorageCleared,
                ..Default::default()
            },
        );

        let accounts = cache.maybe_full_db().unwrap();
        assert_eq!(accounts[&preserved].info.balance, U256::ONE);
        assert_eq!(accounts[&updated].info.balance, U256::from(20));
        assert_eq!(accounts[&updated].storage[&deleted_slot], U256::ZERO);
        assert_eq!(accounts[&updated].storage[&updated_slot], U256::from(12));
        assert!(!accounts.contains_key(&deleted));
        assert!(accounts[&cleared].storage.is_empty());
        let mut expected = accounts;
        expected.get_mut(&updated).unwrap().storage.remove(&deleted_slot);
        assert_eq!(
            cache.maybe_full_db().map(|accounts| crate::mem::state::state_root(&accounts)),
            Some(crate::mem::state::state_root(&expected))
        );
    }

    #[test]
    fn cache_db_state_snapshot_preserves_account_states() {
        let touched = Address::with_last_byte(1);
        let deleted = Address::with_last_byte(2);
        let cleared = Address::with_last_byte(3);
        let (kept_slot, missing_slot) = (U256::ONE, U256::from(2));
        // The base stands in for a fork whose storage must not leak into cleared accounts.
        let base = || {
            let mut base = MemDb::default();
            for address in [touched, deleted, cleared] {
                base.insert_account(address, AccountInfo::default().with_balance(U256::ONE));
                base.set_storage_at(address, missing_slot.into(), B256::with_last_byte(9)).unwrap();
            }
            base
        };

        let mut cache = CacheDB::new(base());
        cache.insert_account_storage(touched, kept_slot, U256::from(5)).unwrap();
        cache.cache.accounts.insert(deleted, DbAccount::new_not_existing());
        let mut cleared_account = DbAccount {
            info: AccountInfo::default().with_balance(U256::from(40)),
            account_state: AccountState::StorageCleared,
            ..Default::default()
        };
        cleared_account.storage.insert(kept_slot, U256::from(6));
        cache.cache.accounts.insert(cleared, cleared_account);

        let snapshot = cache.read_as_state_snapshot();
        let mut restored = CacheDB::new(base());
        restored.init_from_state_snapshot(
            serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap(),
        );

        assert_eq!(restored.storage_ref(touched, kept_slot).unwrap(), U256::from(5));
        assert_eq!(restored.storage_ref(touched, missing_slot).unwrap(), U256::from(9));
        assert_eq!(restored.basic_ref(deleted).unwrap(), None);
        assert_eq!(restored.storage_ref(deleted, missing_slot).unwrap(), U256::ZERO);
        assert_eq!(restored.basic_ref(cleared).unwrap().unwrap().balance, U256::from(40));
        assert_eq!(restored.storage_ref(cleared, kept_slot).unwrap(), U256::from(6));
        assert_eq!(restored.storage_ref(cleared, missing_slot).unwrap(), U256::ZERO);
        assert_eq!(restored.clear_into_state_snapshot().account_states, snapshot.account_states);
    }

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
                    "type": "0x2",
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
            ],
            "ommers": []
        }
        "#;

        let _block: SerializableBlock = serde_json::from_str(block).unwrap();
    }

    #[test]
    fn test_block_withdrawals_preserved() {
        use alloy_eips::eip4895::Withdrawal;

        // create a block with withdrawals (like post-Shanghai blocks)
        let withdrawal = Withdrawal {
            index: 42,
            validator_index: 123,
            address: Address::repeat_byte(1),
            amount: 1000,
        };

        let header = Header::default();
        let body = BlockBody {
            transactions: vec![],
            ommers: vec![],
            withdrawals: Some(vec![withdrawal].into()),
        };
        let block = Block::new(header.into(), body);

        // convert to SerializableBlock and back
        let serializable = SerializableBlock::from(block);
        let restored = Block::from(serializable);

        // withdrawals should be preserved
        assert!(restored.body.withdrawals.is_some());
        let withdrawals = restored.body.withdrawals.unwrap();
        assert_eq!(withdrawals.len(), 1);
        assert_eq!(withdrawals[0].index, 42);
        assert_eq!(withdrawals[0].validator_index, 123);
    }

    #[test]
    fn load_state_fork_read_error_allows_snapshot_rollback() {
        let provider =
            ProviderBuilder::<_, _, AnyNetwork>::default().connect_mocked_client(Asserter::new());
        let remote = BlockchainDb::new(
            BlockchainDbMeta::new(BlockEnv::default(), "http://localhost".to_string()),
            None,
        );
        let (backend, handler) = SharedBackend::new(provider, remote.clone(), None);
        drop(handler);
        let mut db = ForkedDatabase::new(backend, remote);
        let cached = Address::with_last_byte(1);
        let uncached = Address::with_last_byte(2);
        let original = AccountInfo { nonce: 7, balance: U256::ONE, ..Default::default() };
        db.insert_account(cached, original.clone());
        let snapshot = db.snapshot_state();
        let mut state = SerializableState::default();
        for address in [cached, uncached] {
            state.accounts.insert(
                address,
                SerializableAccountRecord {
                    nonce: 3,
                    balance: U256::from(10),
                    code: Bytes::new(),
                    storage: Default::default(),
                },
            );
        }

        // The cached account imports first; the next lookup reaches the closed fork backend.
        assert!(matches!(db.load_state(state), Err(DatabaseError::Send(_))));
        let imported = db.basic_ref(cached).unwrap().unwrap();
        assert_eq!(imported.nonce, original.nonce);
        assert_eq!(imported.balance, U256::from(10));
        assert!(!db.database().cache.accounts.contains_key(&uncached));

        // Mirror Backend::load_state's error path and restore the earlier partial import.
        assert!(db.revert_state(snapshot, RevertStateSnapshotAction::RevertRemove));
        assert_eq!(db.basic_ref(cached).unwrap().unwrap(), original);
        assert!(!db.database().cache.accounts.contains_key(&uncached));
    }
}
