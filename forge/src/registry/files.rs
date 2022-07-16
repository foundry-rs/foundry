//! OS-specific file access

use crate::registry::RegistryConfig;
use foundry_common::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct FileLock {}

/// The `FileSystem` is a shareable abstraction for accessing an underlying path that supports
/// locking and concurrent access.
#[derive(Clone, Debug)]
pub struct FileSystem {
    root: PathBuf,
}

// === impl FileSystem ===

impl FileSystem {}

/// Acquires a file lock and provides a status update if the lock can't be acquired immediately.
///
/// This is useful because there can be a long-running, conflicting forge action that's currently
/// locking the file and we want to relay this state.
fn acquire(config: &RegistryConfig, msg: &str, path: &Path) {}
