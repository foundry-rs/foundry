//! The in memory DB

use crate::{
    eth::backend::db::{
        BLOCKHASH_HISTORY, Db, MaybeForkedDatabase, MaybeFullDatabase, SerializableAccountRecord,
        SerializableBlock, SerializableHistoricalStates, SerializableState,
        SerializableTransaction, StateDb, cache_block_hash,
    },
    mem::state::{StateRootCache, state_root},
};
use alloy_primitives::{
    Address, B256, U256,
    map::{AddressMap, B256Map, HashSet},
};
use alloy_rpc_types::BlockId;
use foundry_evm::backend::{BlockchainDb, DatabaseError, DatabaseResult, StateSnapshot};
use imbl::HashMap as PersistentMap;
use parking_lot::Mutex;
use revm::{
    Database, DatabaseCommit,
    bytecode::Bytecode,
    context::BlockEnv,
    database::{AccountState, DatabaseRef, DbAccount},
    state::{Account, AccountInfo},
};
use std::sync::OnceLock;

// reexport for convenience
pub use foundry_evm::backend::MemDb;
use foundry_evm::backend::RevertStateSnapshotAction;

/// An in-memory database that incrementally maintains Anvil's mined-block state root.
#[derive(Debug, Default)]
pub struct StateRootDb {
    inner: MemDb,
    state_root: Mutex<StateRootCache>,
    history: Mutex<HistoricalStateCache>,
}

/// Incrementally maintained, structurally shared state used by block-history snapshots.
#[derive(Debug, Default)]
struct HistoricalStateCache {
    state: Option<PersistentStateDb>,
    dirty: AddressMap<DirtyHistoricalAccount>,
}

#[derive(Debug, Default)]
struct DirtyHistoricalAccount {
    storage: HashSet<U256>,
    reset_storage: bool,
}

impl HistoricalStateCache {
    fn record_changes(&mut self, changes: &AddressMap<Account>) {
        for (address, account) in changes {
            if !account.is_touched() {
                continue;
            }

            let dirty = self.dirty.entry(*address).or_default();
            dirty.reset_storage |= account.is_created() || account.is_selfdestructed();
            dirty.storage.extend(account.changed_storage_slots().map(|(slot, _)| *slot));
        }
    }

    fn record_account(&mut self, address: Address) {
        self.dirty.entry(address).or_default();
    }

    fn record_storage(&mut self, address: Address, slot: U256) {
        self.dirty.entry(address).or_default().storage.insert(slot);
    }

    fn record_block_hash(&mut self, number: U256, hash: B256) {
        let Some(state) = &mut self.state else { return };
        let head = state.block_hashes.keys().copied().max().map_or(number, |head| head.max(number));
        let min_number = head.saturating_sub(U256::from(BLOCKHASH_HISTORY));
        state.block_hashes.retain(|cached, _| *cached >= min_number && *cached <= head);
        if number >= min_number {
            state.block_hashes.insert(number, hash);
        }
    }

    fn invalidate(&mut self) {
        self.state = None;
        self.dirty.clear();
    }

    fn snapshot(&mut self, db: &MemDb) -> PersistentStateDb {
        let Some(state) = &mut self.state else {
            let state = PersistentStateDb::from_mem_db(db);
            self.state = Some(state.clone());
            self.dirty.clear();
            return state;
        };

        for (address, dirty) in std::mem::take(&mut self.dirty) {
            let Some(account) = db.inner.cache.accounts.get(&address) else {
                state.accounts.remove(&address);
                continue;
            };

            let mut storage = if dirty.reset_storage {
                account.storage.iter().map(|(slot, value)| (*slot, *value)).collect()
            } else {
                state
                    .accounts
                    .get(&address)
                    .map(|account| account.storage.clone())
                    .unwrap_or_default()
            };
            if !dirty.reset_storage {
                for slot in dirty.storage {
                    if let Some(value) = account.storage.get(&slot) {
                        storage.insert(slot, *value);
                    } else {
                        storage.remove(&slot);
                    }
                }
            }

            let info = account_info_with_code(&account.info, &db.inner.cache.contracts);
            if let Some(code) = &info.code {
                state.contracts.insert(info.code_hash, code.clone());
            }
            state.accounts.insert(
                address,
                PersistentAccount { info, account_state: account.account_state.clone(), storage },
            );
        }

        state.full = OnceLock::new();
        state.clone()
    }
}

#[derive(Clone, Debug, Default)]
struct PersistentAccount {
    info: AccountInfo,
    account_state: AccountState,
    storage: PersistentMap<U256, U256>,
}

/// A read-only historical state whose maps are cheap structural-sharing clones.
#[derive(Clone, Debug, Default)]
struct PersistentStateDb {
    accounts: PersistentMap<Address, PersistentAccount>,
    contracts: PersistentMap<B256, Bytecode>,
    block_hashes: PersistentMap<U256, B256>,
    #[allow(clippy::type_complexity)]
    full: OnceLock<AddressMap<DbAccount>>,
}

impl PersistentStateDb {
    fn from_mem_db(db: &MemDb) -> Self {
        let contracts = db
            .inner
            .cache
            .contracts
            .iter()
            .map(|(hash, code)| (*hash, code.clone()))
            .collect::<PersistentMap<_, _>>();
        let accounts = db
            .inner
            .cache
            .accounts
            .iter()
            .map(|(address, account)| {
                (
                    *address,
                    PersistentAccount {
                        info: account_info_with_code(&account.info, &db.inner.cache.contracts),
                        account_state: account.account_state.clone(),
                        storage: account
                            .storage
                            .iter()
                            .map(|(slot, value)| (*slot, *value))
                            .collect(),
                    },
                )
            })
            .collect();
        let block_hashes =
            db.inner.cache.block_hashes.iter().map(|(number, hash)| (*number, *hash)).collect();
        Self { accounts, contracts, block_hashes, full: OnceLock::new() }
    }

    fn state_snapshot(&self) -> StateSnapshot {
        StateSnapshot {
            accounts: self
                .accounts
                .iter()
                .map(|(address, account)| (*address, account.info.clone()))
                .collect(),
            storage: self
                .accounts
                .iter()
                .map(|(address, account)| {
                    (
                        *address,
                        account.storage.iter().map(|(slot, value)| (*slot, *value)).collect(),
                    )
                })
                .collect(),
            block_hashes: self.block_hashes.iter().map(|(number, hash)| (*number, *hash)).collect(),
        }
    }

    fn full_db(&self) -> AddressMap<DbAccount> {
        self.accounts
            .iter()
            .map(|(address, account)| {
                (
                    *address,
                    DbAccount {
                        info: account.info.clone(),
                        account_state: account.account_state.clone(),
                        storage: account
                            .storage
                            .iter()
                            .map(|(slot, value)| (*slot, *value))
                            .collect(),
                    },
                )
            })
            .collect()
    }
}

fn account_info_with_code(info: &AccountInfo, contracts: &B256Map<Bytecode>) -> AccountInfo {
    let mut info = info.clone();
    if info.code.is_none() {
        info.code = contracts.get(&info.code_hash).cloned();
    }
    info
}

impl DatabaseRef for PersistentStateDb {
    type Error = DatabaseError;

    fn basic_ref(&self, address: Address) -> DatabaseResult<Option<AccountInfo>> {
        Ok(match self.accounts.get(&address) {
            Some(account) if account.account_state == AccountState::NotExisting => None,
            Some(account) => Some(account.info.clone()),
            None => Some(AccountInfo::default()),
        })
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> DatabaseResult<Bytecode> {
        Ok(self.contracts.get(&code_hash).cloned().unwrap_or_default())
    }

    fn storage_ref(&self, address: Address, index: U256) -> DatabaseResult<U256> {
        Ok(self
            .accounts
            .get(&address)
            .and_then(|account| account.storage.get(&index).copied())
            .unwrap_or_default())
    }

    fn block_hash_ref(&self, number: u64) -> DatabaseResult<B256> {
        Ok(self.block_hashes.get(&U256::from(number)).copied().unwrap_or_default())
    }
}

impl MaybeFullDatabase for PersistentStateDb {
    fn maybe_as_full_db(&self) -> Option<&AddressMap<DbAccount>> {
        Some(self.full.get_or_init(|| self.full_db()))
    }

    fn is_persistent(&self) -> bool {
        true
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        let snapshot = self.state_snapshot();
        self.clear();
        snapshot
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        self.state_snapshot()
    }

    fn clear(&mut self) {
        *self = Self::default();
    }

    fn init_from_state_snapshot(&mut self, snapshot: StateSnapshot) {
        let StateSnapshot { accounts, mut storage, block_hashes } = snapshot;
        let mut contracts = PersistentMap::new();
        let accounts = accounts
            .into_iter()
            .map(|(address, info)| {
                if let Some(code) = &info.code {
                    contracts.insert(info.code_hash, code.clone());
                }
                let storage = storage
                    .remove(&address)
                    .unwrap_or_default()
                    .into_iter()
                    .collect::<PersistentMap<_, _>>();
                (address, PersistentAccount { info, account_state: AccountState::None, storage })
            })
            .collect();
        let block_hashes = block_hashes.into_iter().collect();
        *self = Self { accounts, contracts, block_hashes, full: OnceLock::new() };
    }
}

impl DatabaseRef for StateRootDb {
    type Error = <MemDb as DatabaseRef>::Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.inner.basic_ref(address)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.inner.code_by_hash_ref(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.inner.storage_ref(address, index)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.inner.block_hash_ref(number)
    }
}

impl Database for StateRootDb {
    type Error = <MemDb as Database>::Error;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.state_root.get_mut().record_account(address);
        self.history.get_mut().record_account(address);
        self.inner.basic(address)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.inner.code_by_hash(code_hash)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.state_root.get_mut().record_storage(address, index);
        self.history.get_mut().record_storage(address, index);
        self.inner.storage(address, index)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.inner.block_hash(number)
    }
}

impl DatabaseCommit for StateRootDb {
    fn commit(&mut self, changes: revm::state::EvmState) {
        self.state_root.get_mut().record_changes(&changes);
        self.history.get_mut().record_changes(&changes);
        self.inner.commit(changes);
    }
}

impl Db for StateRootDb {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.state_root.get_mut().record_account(address);
        self.history.get_mut().record_account(address);
        Db::insert_account(&mut self.inner, address, account);
    }

    fn set_storage_at(&mut self, address: Address, slot: B256, val: B256) -> DatabaseResult<()> {
        let storage_slot = slot.into();
        self.state_root.get_mut().record_storage(address, storage_slot);
        self.history.get_mut().record_storage(address, storage_slot);
        Db::set_storage_at(&mut self.inner, address, slot, val)
    }

    fn insert_block_hash(&mut self, number: U256, hash: B256) {
        Db::insert_block_hash(&mut self.inner, number, hash);
        self.history.get_mut().record_block_hash(number, hash);
    }

    fn dump_state(
        &self,
        at: BlockEnv,
        best_number: u64,
        blocks: Vec<SerializableBlock>,
        transactions: Vec<SerializableTransaction>,
        historical_states: Option<SerializableHistoricalStates>,
    ) -> DatabaseResult<Option<SerializableState>> {
        Db::dump_state(&self.inner, at, best_number, blocks, transactions, historical_states)
    }

    fn snapshot_state(&mut self) -> U256 {
        Db::snapshot_state(&mut self.inner)
    }

    fn revert_state(&mut self, id: U256, action: RevertStateSnapshotAction) -> bool {
        let reverted = Db::revert_state(&mut self.inner, id, action);
        if reverted {
            self.state_root.get_mut().invalidate();
            self.history.get_mut().invalidate();
        }
        reverted
    }

    fn maybe_state_root(&self) -> Option<B256> {
        Some(self.state_root.lock().root(&self.inner.inner.cache.accounts))
    }

    fn current_state(&self) -> StateDb {
        StateDb::new(self.history.lock().snapshot(&self.inner))
    }
}

impl MaybeFullDatabase for StateRootDb {
    fn maybe_as_full_db(&self) -> Option<&AddressMap<DbAccount>> {
        MaybeFullDatabase::maybe_as_full_db(&self.inner)
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        self.state_root.get_mut().invalidate();
        self.history.get_mut().invalidate();
        MaybeFullDatabase::clear_into_state_snapshot(&mut self.inner)
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        MaybeFullDatabase::read_as_state_snapshot(&self.inner)
    }

    fn clear(&mut self) {
        self.state_root.get_mut().invalidate();
        self.history.get_mut().invalidate();
        MaybeFullDatabase::clear(&mut self.inner)
    }

    fn init_from_state_snapshot(&mut self, snapshot: StateSnapshot) {
        MaybeFullDatabase::init_from_state_snapshot(&mut self.inner, snapshot);
        self.state_root.get_mut().invalidate();
        self.history.get_mut().invalidate();
    }
}

impl MaybeForkedDatabase for StateRootDb {
    fn maybe_reset(&mut self, urls: Vec<String>, block_number: BlockId) -> Result<(), String> {
        self.inner.maybe_reset(urls, block_number)
    }

    fn maybe_flush_cache(&self) -> Result<(), String> {
        self.inner.maybe_flush_cache()
    }

    fn maybe_inner(&self) -> Result<&BlockchainDb, String> {
        self.inner.maybe_inner()
    }
}

impl Db for MemDb {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.inner.insert_account_info(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: B256, val: B256) -> DatabaseResult<()> {
        self.inner.insert_account_storage(address, slot.into(), val.into())
    }

    fn insert_block_hash(&mut self, number: U256, hash: B256) {
        cache_block_hash(&mut self.inner.cache.block_hashes, number, hash);
    }

    fn dump_state(
        &self,
        at: BlockEnv,
        best_number: u64,
        blocks: Vec<SerializableBlock>,
        transactions: Vec<SerializableTransaction>,
        historical_states: Option<SerializableHistoricalStates>,
    ) -> DatabaseResult<Option<SerializableState>> {
        let accounts = self
            .inner
            .cache
            .accounts
            .clone()
            .into_iter()
            .map(|(k, v)| -> DatabaseResult<_> {
                let code = if let Some(code) = v.info.code {
                    code
                } else {
                    self.inner.code_by_hash_ref(v.info.code_hash)?
                };
                Ok((
                    k,
                    SerializableAccountRecord {
                        nonce: v.info.nonce,
                        balance: v.info.balance,
                        code: code.original_bytes(),
                        storage: v.storage.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
                    },
                ))
            })
            .collect::<Result<_, _>>()?;

        Ok(Some(SerializableState {
            block: Some(at),
            accounts,
            best_block_number: Some(best_number),
            blocks,
            transactions,
            historical_states,
        }))
    }

    /// Creates a new snapshot
    fn snapshot_state(&mut self) -> U256 {
        let id = self.state_snapshots.insert(self.inner.clone());
        trace!(target: "backend::memdb", "Created new state snapshot {}", id);
        id
    }

    fn revert_state(&mut self, id: U256, action: RevertStateSnapshotAction) -> bool {
        if let Some(state_snapshot) = self.state_snapshots.remove(id) {
            if action.is_keep() {
                self.state_snapshots.insert_at(state_snapshot.clone(), id);
            }
            self.inner = state_snapshot;
            trace!(target: "backend::memdb", "Reverted state snapshot {}", id);
            true
        } else {
            warn!(target: "backend::memdb", "No state snapshot to revert for {}", id);
            false
        }
    }

    fn maybe_state_root(&self) -> Option<B256> {
        Some(state_root(&self.inner.cache.accounts))
    }

    fn current_state(&self) -> StateDb {
        StateDb::new(Self { inner: self.inner.clone(), ..Default::default() })
    }
}

impl MaybeFullDatabase for MemDb {
    fn maybe_as_full_db(&self) -> Option<&AddressMap<DbAccount>> {
        Some(&self.inner.cache.accounts)
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        self.inner.clear_into_state_snapshot()
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        self.inner.read_as_state_snapshot()
    }

    fn clear(&mut self) {
        self.inner.clear();
    }

    fn init_from_state_snapshot(&mut self, snapshot: StateSnapshot) {
        self.inner.init_from_state_snapshot(snapshot)
    }
}

impl MaybeForkedDatabase for MemDb {
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Bytes, address};
    use revm::primitives::KECCAK_EMPTY;
    use std::collections::BTreeMap;

    // verifies that all substantial aspects of a loaded account remain the same after an account
    // is dumped and reloaded
    #[test]
    fn test_dump_reload_cycle() {
        let test_addr: Address = address!("0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");

        let mut dump_db = MemDb::default();

        let contract_code = Bytecode::new_raw(Bytes::from("fake contract code"));
        dump_db.insert_account(
            test_addr,
            AccountInfo {
                balance: U256::from(123456),
                code_hash: KECCAK_EMPTY,
                code: Some(contract_code.clone()),
                nonce: 1234,
                account_id: None,
            },
        );
        dump_db
            .set_storage_at(test_addr, U256::from(1234567).into(), U256::from(1).into())
            .unwrap();

        // blocks dumping/loading tested in storage.rs
        let state = dump_db
            .dump_state(Default::default(), 0, Vec::new(), Vec::new(), Default::default())
            .unwrap()
            .unwrap();

        let mut load_db = MemDb::default();

        load_db.load_state(state).unwrap();

        let loaded_account = load_db.basic_ref(test_addr).unwrap().unwrap();

        assert_eq!(loaded_account.balance, U256::from(123456));
        assert_eq!(load_db.code_by_hash_ref(loaded_account.code_hash).unwrap(), contract_code);
        assert_eq!(loaded_account.nonce, 1234);
        assert_eq!(load_db.storage_ref(test_addr, U256::from(1234567)).unwrap(), U256::from(1));
    }

    // verifies that multiple accounts can be loaded at a time, and storage is merged within those
    // accounts as well.
    #[test]
    fn test_load_state_merge() {
        let test_addr: Address = address!("0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
        let test_addr2: Address = address!("0x70997970c51812dc3a010c7d01b50e0d17dc79c8");

        let contract_code = Bytecode::new_raw(Bytes::from("fake contract code"));

        let mut db = MemDb::default();

        db.insert_account(
            test_addr,
            AccountInfo {
                balance: U256::from(123456),
                code_hash: KECCAK_EMPTY,
                code: Some(contract_code.clone()),
                nonce: 1234,
                account_id: None,
            },
        );

        db.set_storage_at(test_addr, U256::from(1234567).into(), U256::from(1).into()).unwrap();
        db.set_storage_at(test_addr, U256::from(1234568).into(), U256::from(2).into()).unwrap();

        let mut new_state = SerializableState::default();

        new_state.accounts.insert(
            test_addr2,
            SerializableAccountRecord {
                balance: Default::default(),
                code: Default::default(),
                nonce: 1,
                storage: Default::default(),
            },
        );

        let mut new_storage = BTreeMap::default();
        new_storage.insert(U256::from(1234568).into(), U256::from(5).into());

        new_state.accounts.insert(
            test_addr,
            SerializableAccountRecord {
                balance: U256::from(100100),
                code: contract_code.bytes()[..contract_code.len()].to_vec().into(),
                nonce: 100,
                storage: new_storage,
            },
        );

        db.load_state(new_state).unwrap();

        let loaded_account = db.basic_ref(test_addr).unwrap().unwrap();
        let loaded_account2 = db.basic_ref(test_addr2).unwrap().unwrap();

        assert_eq!(loaded_account2.nonce, 1);

        assert_eq!(loaded_account.balance, U256::from(100100));
        assert_eq!(db.code_by_hash_ref(loaded_account.code_hash).unwrap(), contract_code);
        assert_eq!(loaded_account.nonce, 1234);
        assert_eq!(db.storage_ref(test_addr, U256::from(1234567)).unwrap(), U256::from(1));
        assert_eq!(db.storage_ref(test_addr, U256::from(1234568)).unwrap(), U256::from(5));
    }

    #[test]
    fn incremental_state_root_matches_full_rebuild() {
        let address = address!("0000000000000000000000000000000000002935");
        let mut db = StateRootDb::default();
        db.insert_account(address, AccountInfo::default());

        assert_eq!(db.maybe_state_root(), Some(state_root(&db.inner.inner.cache.accounts)));

        // Model the EIP-2935 history contract filling one new ring-buffer slot per block.
        for slot in 0..1_024 {
            db.set_storage_at(address, U256::from(slot).into(), B256::from(U256::from(slot + 1)))
                .unwrap();
            let _ = db.maybe_state_root().unwrap();
        }

        db.set_balance(address, U256::from(42)).unwrap();
        db.set_storage_at(address, U256::from(7).into(), B256::ZERO).unwrap();
        db.insert_account(Address::with_last_byte(1), AccountInfo::from_balance(U256::from(1)));
        assert_eq!(db.maybe_state_root(), Some(state_root(&db.inner.inner.cache.accounts)));

        let snapshot = db.snapshot_state();
        db.set_balance(address, U256::from(43)).unwrap();
        assert!(db.revert_state(snapshot, RevertStateSnapshotAction::RevertRemove));
        assert_eq!(db.maybe_state_root(), Some(state_root(&db.inner.inner.cache.accounts)));
    }

    #[test]
    fn evm_block_hash_cache_is_bounded() {
        let mut db = StateRootDb::default();
        for number in 0..1_024 {
            db.insert_block_hash(U256::from(number), B256::from(U256::from(number)));
        }

        let block_hashes = &db.inner.inner.cache.block_hashes;
        assert_eq!(block_hashes.len(), BLOCKHASH_HISTORY as usize + 1);
        assert!(!block_hashes.contains_key(&U256::from(766)));
        assert!(block_hashes.contains_key(&U256::from(767)));
        assert!(block_hashes.contains_key(&U256::from(768)));
        assert!(block_hashes.contains_key(&U256::from(1_023)));
    }

    #[test]
    fn evm_block_hash_cache_is_bounded_across_block_number_jumps() {
        let mut db = StateRootDb::default();
        // Initialize the persistent historical-state cache as well as the live EVM cache.
        db.current_state();

        for number in [0, 516, 400] {
            db.insert_block_hash(U256::from(number), B256::from(U256::from(number)));
        }

        // An out-of-order insertion within the active window must not discard the current head.
        let block_hashes = &db.inner.inner.cache.block_hashes;
        assert_eq!(block_hashes.len(), 2);
        assert!(block_hashes.contains_key(&U256::from(400)));
        assert!(block_hashes.contains_key(&U256::from(516)));

        db.insert_block_hash(U256::from(774), B256::from(U256::from(774)));

        let block_hashes = &db.inner.inner.cache.block_hashes;
        assert_eq!(block_hashes.len(), 1);
        assert!(block_hashes.contains_key(&U256::from(774)));

        let historical = db.history.get_mut().state.as_ref().unwrap();
        assert_eq!(historical.block_hashes.len(), 1);
        assert!(historical.block_hashes.contains_key(&U256::from(774)));
    }

    #[test]
    fn historical_states_are_persistent_and_isolated() {
        let address = address!("0000000000000000000000000000000000002935");
        let slot = U256::from(1);
        let mut db = StateRootDb::default();
        db.insert_account(address, AccountInfo::from_balance(U256::from(1)));

        let first = db.current_state();
        assert!(first.is_persistent());

        db.set_balance(address, U256::from(2)).unwrap();
        db.set_storage_at(address, slot.into(), B256::from(U256::from(3))).unwrap();
        let second = db.current_state();

        assert_eq!(first.basic_ref(address).unwrap().unwrap().balance, U256::from(1));
        assert_eq!(first.storage_ref(address, slot).unwrap(), U256::ZERO);
        assert_eq!(second.basic_ref(address).unwrap().unwrap().balance, U256::from(2));
        assert_eq!(second.storage_ref(address, slot).unwrap(), U256::from(3));
    }

    #[test]
    fn historical_missing_accounts_match_live_state() {
        let address = Address::with_last_byte(1);
        let db = StateRootDb::default();
        let historical = db.current_state();

        let live_account = db.basic_ref(address).unwrap();
        assert_eq!(live_account, Some(AccountInfo::default()));
        assert_eq!(historical.basic_ref(address).unwrap(), live_account);

        let mut persistent = PersistentStateDb::default();
        persistent.accounts.insert(
            address,
            PersistentAccount { account_state: AccountState::NotExisting, ..Default::default() },
        );
        assert_eq!(persistent.basic_ref(address).unwrap(), None);
    }
}
