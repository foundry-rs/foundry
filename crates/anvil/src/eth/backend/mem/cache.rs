use crate::config::anvil_tmp_dir;
use alloy_primitives::B256;
use foundry_evm::backend::StateSnapshot;
use std::{
    io,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

/// On disk state cache
///
/// A basic tempdir which stores states on disk
pub struct DiskStateCache {
    /// The path where to create the tempdir in
    pub(crate) temp_path: Option<PathBuf>,
    /// Holds the temp dir object.
    pub(crate) temp_dir: Option<TempDir>,
}

impl DiskStateCache {
    /// Returns the cache file for the given hash
    fn with_cache_file<F, R>(&mut self, hash: B256, f: F) -> Option<R>
    where
        F: FnOnce(PathBuf) -> R,
    {
        if self.temp_dir.is_none() {
            let tmp_dir = self
                .temp_path
                .as_ref()
                .map(|p| -> io::Result<TempDir> {
                    std::fs::create_dir_all(p)?;
                    build_tmp_dir(Some(p))
                })
                .unwrap_or_else(|| build_tmp_dir(None));

            match tmp_dir {
                Ok(temp_dir) => {
                    trace!(target: "backend", path=?temp_dir.path(), "created disk state cache dir");
                    self.temp_dir = Some(temp_dir);
                }
                Err(err) => {
                    error!(target: "backend", %err, "failed to create disk state cache dir");
                }
            }
        }
        if let Some(ref temp_dir) = self.temp_dir {
            let path = temp_dir.path().join(format!("{hash:?}.json"));
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
    pub fn write(&mut self, hash: B256, state: StateSnapshot) {
        self.with_cache_file(hash, |file| {
            tokio::task::spawn(async move {
                match foundry_common::fs::write_json_file(&file, &state) {
                    Ok(_) => {
                        trace!(target: "backend", ?hash, "wrote state json file");
                    }
                    Err(err) => {
                        error!(target: "backend", %err, ?hash, "Failed to load state snapshot");
                    }
                };
            });
        });
    }

    /// Loads the snapshot file for the given hash
    ///
    /// Returns None if it doesn't exist or deserialization failed
    pub fn read(&mut self, hash: B256) -> Option<StateSnapshot> {
        self.with_cache_file(hash, |file| {
            match foundry_common::fs::read_json_file::<StateSnapshot>(&file) {
                Ok(state) => {
                    trace!(target: "backend", ?hash,"loaded cached state");
                    Some(state)
                }
                Err(err) => {
                    error!(target: "backend", %err, ?hash, "Failed to load state snapshot");
                    None
                }
            }
        })
        .flatten()
    }

    /// Removes the cache file for the given hash, if it exists
    pub fn remove(&mut self, hash: B256) {
        self.with_cache_file(hash, |file| {
            foundry_common::fs::remove_file(file).map_err(|err| {
                error!(target: "backend", %err, %hash, "Failed to remove state snapshot");
            })
        });
    }
}

impl Default for DiskStateCache {
    fn default() -> Self {
        Self { temp_path: anvil_tmp_dir(), temp_dir: None }
    }
}

/// Returns the temporary dir for the cached state
///
/// This will create a prefixed temp dir with `anvil-state-06-11-2022-12-50`
fn build_tmp_dir(p: Option<&Path>) -> io::Result<TempDir> {
    let mut builder = tempfile::Builder::new();
    let now = chrono::offset::Utc::now();
    let prefix = now.format("anvil-state-%d-%m-%Y-%H-%M").to_string();
    builder.prefix(&prefix);

    if let Some(p) = p {
        builder.tempdir_in(p)
    } else {
        builder.tempdir()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn can_build_temp_dir() {
        let dir = tempdir().unwrap();
        let p = dir.path();
        let cache_dir = build_tmp_dir(Some(p)).unwrap();
        assert!(cache_dir
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("anvil-state-"));
        let cache_dir = build_tmp_dir(None).unwrap();
        assert!(cache_dir
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("anvil-state-"));
    }
}
