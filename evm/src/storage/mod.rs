//! disk storage support and helpers

use ethers::types::U256;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub mod diskmap;

/// Disk-backed map for storage slots, uses json.
pub struct StorageMap {
    cache: diskmap::DiskMap<U256, U256>,
}

impl StorageMap {
    /// Creates new storage map at given _directory_.
    ///
    /// # Example
    ///
    /// ```no_run
    ///  use foundry_evm::storage::StorageMap;
    /// let mut map = StorageMap::read("data dir");
    /// map.insert(100u64.into(), 99u64.into());
    /// map.save();
    /// ```
    pub fn read(path: impl AsRef<Path>) -> Self {
        StorageMap { cache: diskmap::DiskMap::read(path.as_ref().join("storage.json"), read_u256) }
    }

    /// Creates transient storage map (no changes are saved to disk).
    pub fn transient() -> Self {
        StorageMap { cache: diskmap::DiskMap::new("storage.json").transient() }
    }

    /// The path where the diskmap is (will be) stored
    pub fn path(&self) -> &PathBuf {
        self.cache.path()
    }

    /// Get the cache map.
    pub fn inner(&self) -> &HashMap<U256, U256> {
        &self.cache
    }

    /// Consumes the type and returns the underlying map
    pub fn into_inner(self) -> HashMap<U256, U256> {
        self.cache.into_inner()
    }

    pub fn save(&self) {
        self.cache.save(write_u256)
    }

    /// Sets new value for given U256.
    pub fn set_value(&mut self, key: U256, value: U256) -> Option<U256> {
        self.cache.insert(key, value)
    }

    /// Removes an entry
    pub fn remove(&mut self, key: U256) {
        self.cache.remove(&key);
    }
}

/// Read a hash map of U256 -> U256
fn read_u256<R>(reader: R) -> Result<HashMap<U256, U256>, serde_json::Error>
where
    R: ::std::io::Read,
{
    serde_json::from_reader(reader)
}

/// Write a hash map of U256 -> U256
fn write_u256<W>(m: &HashMap<U256, U256>, writer: &mut W) -> Result<(), serde_json::Error>
where
    W: ::std::io::Write,
{
    serde_json::to_writer(writer, m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::tempdir;

    #[test]
    fn can_save_and_reload_storage_map() {
        let tempdir = tempdir().unwrap();
        let mut map = StorageMap::read(tempdir.path());
        map.set_value(100u64.into(), 300u64.into());
        map.set_value(1337u64.into(), 99u64.into());
        map.save();

        let map = StorageMap::read(tempdir.path());
        assert_eq!(
            map.into_inner(),
            HashMap::<U256, U256>::from([
                (100u64.into(), 300u64.into()),
                (1337u64.into(), 99u64.into()),
            ])
        );
    }
}
