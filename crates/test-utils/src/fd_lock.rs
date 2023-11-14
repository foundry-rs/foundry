//! File locking utilities.

use crate::util::pretty_err;
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

pub use fd_lock::*;

/// Creates a new lock file at the given path.
pub fn new_lock(lock_path: impl AsRef<Path>) -> RwLock<File> {
    fn new_lock(lock_path: &Path) -> RwLock<File> {
        let lock_file = pretty_err(
            lock_path,
            OpenOptions::new().read(true).write(true).create(true).open(lock_path),
        );
        RwLock::new(lock_file)
    }
    new_lock(lock_path.as_ref())
}
