//! Configuration specific to the `forge doc` command and the `forge_doc` package

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Contains the config for parsing and rendering docs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocConfig {
    /// Doc output path.
    pub out: PathBuf,
    /// The documentation title.
    pub title: String,
    /// Path to user provided `book.toml`.
    pub book: PathBuf,
    /// Path to user provided welcome markdown.
    ///
    /// If none is provided, it defaults to `README.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<PathBuf>,
    /// The repository url.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// Globs to ignore
    pub ignore: Vec<String>,
}

impl Default for DocConfig {
    fn default() -> Self {
        Self {
            out: PathBuf::from("docs"),
            book: PathBuf::from("book.toml"),
            homepage: Some(PathBuf::from("README.md")),
            title: String::default(),
            repository: None,
            ignore: Vec::default(),
        }
    }
}
