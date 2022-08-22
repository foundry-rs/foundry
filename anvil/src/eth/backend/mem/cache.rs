use crate::eth::backend::db::StateDb;
use ethers::{
    prelude::H256,
    types::{Address, U256},
};
use forge::revm::AccountInfo;
use foundry_evm::HashMap as Map;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};
use tempfile::TempDir;
use tracing::{error, trace};
use foundry_evm::executor::backend::snapshot::StateSnapshot;

/// On disk state cache
///
/// A basic tempdir which stores states on disk
#[derive(Default)]
pub struct DiskStateCache {
    temp_dir: Option<TempDir>,
}

impl DiskStateCache {
    /// Returns the cache file for the given hash
    fn with_cache_file<F, R>(&mut self, hash: H256, f: F) -> Option<R>
    where
        F: FnOnce(PathBuf) -> R,
    {
        if self.temp_dir.is_none() {
            match TempDir::new() {
                Ok(temp_dir) => {
                    trace!(path=?temp_dir.path(), "created disk state cache dir");
                    self.temp_dir = Some(temp_dir);
                }
                Err(err) => {
                    error!(?err, "failed to create disk state cache dir");
                }
            }
        }
        if let Some(ref temp_dir) = self.temp_dir {
            let path = temp_dir.path().join(format!("{:?}.json", hash));
            Some(f(path))
        } else {
            None
        }
    }

    fn store(&mut self, hash: H256, state: StateSnapshot) {
        self.with_cache_file(hash, |file| ());
    }

    fn load(&mut self, hash: H256, state: StateDb) -> Option<StateSnapshot> {
        self.with_cache_file(hash, |file| {
           match foundry_common::fs::read_

        });
    }
}
