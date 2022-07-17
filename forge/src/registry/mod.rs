//! forge's registry support

use crate::registry::{files::FileSystem, shell::Shell};
use std::cell::RefCell;

mod files;
mod package;
mod shell;

/// Forge registry related config.
#[derive(Debug)]
pub struct RegistryConfig {
    /// The location of the foundry home directory
    foundry_home: FileSystem,
    /// Holds the output shell used for emitting messages
    // This is a `RefCell` so we can access all `mut` output functions
    shell: RefCell<Shell>,
}
