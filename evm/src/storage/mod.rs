//! disk storage support and helpers

use ethers::types::{Address, U256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub mod diskmap;

/// Maps the storage of an account
pub type AccountStorage = BTreeMap<Address, BTreeMap<U256, U256>>;

/// Disk-backed map for storage slots, uses json.
pub struct StorageMap {
    cache: diskmap::DiskMap<Address, BTreeMap<U256, U256>>,
}

impl StorageMap {
    /// Creates new storage map at given _directory_.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use std::collections::BTreeMap;
    /// use foundry_evm::Address;
    /// use foundry_evm::storage::StorageMap;
    /// let mut map = StorageMap::read("data dir");
    /// map.insert(Address::random(), BTreeMap::from([(100u64.into(), 200u64.into())]));
    /// map.save();
    /// ```
    pub fn read(path: impl AsRef<Path>) -> Self {
        StorageMap { cache: diskmap::DiskMap::read(path.as_ref().join("storage.json"), read_u256) }
    }

    /// Creates a new, `DiskMap` filled with the cache data
    pub fn with_data(path: impl AsRef<Path>, cache: AccountStorage) -> Self {
        StorageMap { cache: diskmap::DiskMap::with_data(path.as_ref().join("storage.json"), cache) }
    }

    /// Creates transient storage map (no changes are saved to disk).
    pub fn transient() -> Self {
        StorageMap { cache: diskmap::DiskMap::new("storage.json").transient() }
    }

    /// Whether this cache is transient
    pub fn is_transient(&self) -> bool {
        self.cache.is_transient()
    }

    /// Sets the given data as the content of this cache
    pub fn set_storage(&mut self, data: AccountStorage) {
        *self.cache = data;
    }

    /// Returns the storage data and replaces it with an empty map
    pub fn take_storage(&mut self) -> AccountStorage {
        std::mem::take(&mut *self.cache)
    }

    /// The path where the diskmap is (will be) stored
    pub fn path(&self) -> &PathBuf {
        self.cache.path()
    }

    /// Get the cache map.
    pub fn inner(&self) -> &AccountStorage {
        &self.cache
    }

    /// Consumes the type and returns the underlying map
    pub fn into_inner(self) -> AccountStorage {
        self.cache.into_inner()
    }

    pub fn save(&self) {
        self.cache.save(write_u256)
    }

    /// Sets new value for given address.
    pub fn insert(
        &mut self,
        key: Address,
        value: BTreeMap<U256, U256>,
    ) -> Option<BTreeMap<U256, U256>> {
        self.cache.insert(key, value)
    }

    /// Removes an entry
    pub fn remove(&mut self, key: Address) -> Option<BTreeMap<U256, U256>> {
        self.cache.remove(&key)
    }
}

/// Read a hash map of U256 -> U256
fn read_u256<R>(reader: R) -> Result<AccountStorage, serde_json::Error>
where
    R: ::std::io::Read,
{
    serde_json::from_reader(reader)
}

/// Write a hash map of U256 -> U256
fn write_u256<W>(m: &AccountStorage, writer: &mut W) -> Result<(), serde_json::Error>
where
    W: ::std::io::Write,
{
    serde_json::to_writer(writer, m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn can_save_and_reload_storage_map() {
        let tempdir = tempdir().unwrap();
        let mut map = StorageMap::read(tempdir.path());
        let addr = Address::random();
        map.insert(addr, BTreeMap::from([(100u64.into(), 200u64.into())]));
        map.save();

        let map = StorageMap::read(tempdir.path());
        assert_eq!(
            map.into_inner(),
            BTreeMap::from([(addr, BTreeMap::from([(100u64.into(), 200u64.into()),]))])
        );
    }
}
