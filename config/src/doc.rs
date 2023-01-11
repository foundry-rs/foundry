//! Configuration specific to the `forge doc` command and the `forge_doc` package

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Contains the config for parsing and rendering docs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocConfig {
    /// Doc output path.
    pub out: PathBuf,
    /// The documentation title.
    pub title: String,
    /// Path to user provided `book.toml`.
    pub book: PathBuf,
}

impl Default for DocConfig {
    fn default() -> Self {
        Self { out: PathBuf::from("docs"), title: String::default(), book: PathBuf::default() }
    }
}
