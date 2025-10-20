//! File locking utilities.

use crate::util::pretty_err;
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

pub use fd_lock::*;

/// Creates a new lock file at the given path.
pub fn new_lock(lock_path: impl AsRef<Path>) -> RwLock<File> {
    let lock_file = pretty_err(
        lock_path.as_ref(),
        OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock_path.as_ref()),
    );
    RwLock::new(lock_file)
}

pub(crate) const LOCK_TOKEN: &[u8] = b"1";

pub(crate) fn lock_exists(lock_path: &Path) -> bool {
    std::fs::read(lock_path).is_ok_and(|b| b == LOCK_TOKEN)
}
