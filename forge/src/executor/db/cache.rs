use bytes::Bytes;
use ethers::{
    types::{Address, H256, U256},
    utils::keccak256,
};
use hashbrown::hash_map::{Entry, HashMap};
use revm::{
    db::{Database, DatabaseCommit, DatabaseRef},
    Account, AccountInfo, Filth, KECCAK_EMPTY,
};

pub struct CacheDB<D: DatabaseRef> {
    cache: HashMap<Address, AccountInfo>,
    storage: HashMap<Address, HashMap<U256, U256>>,
    contracts: HashMap<H256, Bytes>,
    db: D,
}

impl<D: DatabaseRef> CacheDB<D> {
    pub fn new(db: D) -> Self {
        let mut contracts = HashMap::new();
        contracts.insert(KECCAK_EMPTY, Bytes::new());
        contracts.insert(H256::zero(), Bytes::new());
        Self { cache: HashMap::new(), storage: HashMap::new(), contracts, db }
    }

    pub fn insert_cache(&mut self, address: Address, mut account: AccountInfo) {
        let code = core::mem::take(&mut account.code);
        if let Some(code) = code {
            if !code.is_empty() {
                let code_hash = H256::from_slice(&keccak256(&code));
                account.code_hash = code_hash;
                self.contracts.insert(code_hash, code);
            }
        }
        if account.code_hash.is_zero() {
            account.code_hash = KECCAK_EMPTY;
        }
        self.cache.insert(address, account);
    }
}

impl<D: DatabaseRef> DatabaseCommit for CacheDB<D> {
    fn commit(&mut self, changes: HashMap<Address, Account>) {
        for (add, acc) in changes {
            if acc.is_empty() || matches!(acc.filth, Filth::Destroyed) {
                self.cache.remove(&add);
                self.storage.remove(&add);
            } else {
                self.insert_cache(add, acc.info);
                let storage = self.storage.entry(add).or_default();
                if acc.filth.abandon_old_storage() {
                    storage.clear();
                }
                for (index, value) in acc.storage {
                    if value.is_zero() {
                        storage.remove(&index);
                    } else {
                        storage.insert(index, value);
                    }
                }
                if storage.is_empty() {
                    self.storage.remove(&add);
                }
            }
        }
    }
}

impl<D: DatabaseRef> Database for CacheDB<D> {
    fn block_hash(&mut self, number: U256) -> H256 {
        self.db.block_hash(number)
    }

    fn basic(&mut self, address: Address) -> AccountInfo {
        match self.cache.entry(address) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => {
                let acc = self.db.basic(address);
                if !acc.is_empty() {
                    entry.insert(acc.clone());
                }
                acc
            }
        }
    }

    fn storage(&mut self, address: Address, index: U256) -> U256 {
        match self.storage.entry(address) {
            Entry::Occupied(mut entry) => match entry.get_mut().entry(index) {
                Entry::Occupied(entry) => *entry.get(),
                Entry::Vacant(entry) => {
                    let slot = self.db.storage(address, index);
                    entry.insert(slot);
                    slot
                }
            },
            Entry::Vacant(entry) => {
                let mut storage = HashMap::new();
                let slot = self.db.storage(address, index);
                storage.insert(index, slot);
                entry.insert(storage);
                slot
            }
        }
    }

    fn code_by_hash(&mut self, code_hash: H256) -> Bytes {
        match self.contracts.entry(code_hash) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => entry.insert(self.db.code_by_hash(code_hash)).clone(),
        }
    }
}

impl<D: DatabaseRef> DatabaseRef for CacheDB<D> {
    fn block_hash(&self, number: U256) -> H256 {
        self.db.block_hash(number)
    }

    fn basic(&self, address: Address) -> AccountInfo {
        match self.cache.get(&address) {
            Some(info) => info.clone(),
            None => self.db.basic(address),
        }
    }

    fn storage(&self, address: Address, index: U256) -> U256 {
        match self.storage.get(&address) {
            Some(entry) => match entry.get(&index) {
                Some(entry) => *entry,
                None => self.db.storage(address, index),
            },
            None => self.db.storage(address, index),
        }
    }

    fn code_by_hash(&self, code_hash: H256) -> Bytes {
        match self.contracts.get(&code_hash) {
            Some(entry) => entry.clone(),
            None => self.db.code_by_hash(code_hash),
        }
    }
}
