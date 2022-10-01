use ethers::prelude::H256;

use foundry_evm::executor::backend::snapshot::StateSnapshot;

use std::path::PathBuf;
use tempfile::TempDir;
use tracing::{error, trace};

/// On disk state cache
///
/// A basic tempdir which stores states on disk
#[derive(Default)]
pub struct DiskStateCache {
    pub(crate) temp_dir: Option<TempDir>,
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
                    trace!(target: "backend", path=?temp_dir.path(), "created disk state cache dir");
                    self.temp_dir = Some(temp_dir);
                }
                Err(err) => {
                    error!(target: "backend", ?err, "failed to create disk state cache dir");
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

    /// Stores the snapshot for the given hash
    ///
    /// Note: this writes the state on a new spawned task
    ///
    /// Caution: this requires a running tokio Runtime.
    pub fn write(&mut self, hash: H256, state: StateSnapshot) {
        self.with_cache_file(hash, |file| {
            tokio::task::spawn(async move {
                match foundry_common::fs::write_json_file(&file, &state) {
                    Ok(_) => {
                        trace!(target: "backend", ?hash, "wrote state json file");
                    }
                    Err(err) => {
                        error!(target: "backend", ?err, ?hash, "Failed to load state snapshot");
                    }
                };
            });
        });
    }

    /// Loads the snapshot file for the given hash
    ///
    /// Returns None if it doesn't exist or deserialization failed
    pub fn read(&mut self, hash: H256) -> Option<StateSnapshot> {
        self.with_cache_file(hash, |file| {
            match foundry_common::fs::read_json_file::<StateSnapshot>(&file) {
                Ok(state) => {
                    trace!(target: "backend", ?hash,"loaded cached state");
                    Some(state)
                }
                Err(err) => {
                    error!(target: "backend", ?err, ?hash, "Failed to load state snapshot");
                    None
                }
            }
        })
        .flatten()
    }
}
