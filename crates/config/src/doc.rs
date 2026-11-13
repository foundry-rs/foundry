//! Configuration specific to the `forge doc` command and the `forge_doc` package

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Contains the config for parsing and rendering docs
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocConfig {
    /// Directory where `forge doc` writes the generated documentation site.
    ///
    /// A relative path is resolved from the project root. `forge doc --out` overrides it.
    pub out: PathBuf,
    /// Title of the generated documentation.
    ///
    /// When empty, the site is titled `Documentation`.
    pub title: String,
    /// Legacy setting kept for compatibility; `forge doc` no longer reads it.
    ///
    /// `forge doc` now generates a Vocs site and does not use a `book.toml`.
    pub book: PathBuf,
    /// Markdown file used as the documentation homepage.
    ///
    /// A relative path is resolved from the project root. If the file can't be read,
    /// `forge doc` falls back to `README.md` in the sources directory, then `README.md` in the
    /// project root, and otherwise leaves the homepage empty. Relative links in the homepage to
    /// `.sol` files point to their generated pages; other relative links point to the file in
    /// `repository` at `commit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<PathBuf>,
    /// URL of the project's Git repository, used to link to source files.
    ///
    /// Also adds "edit this page" links to the site. If unset, `forge doc` derives it from the
    /// `origin` remote of the project's Git repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// Git commit hash, tag or branch used in source and homepage links.
    ///
    /// If unset, `forge doc` uses the commit currently checked out (`HEAD`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Legacy setting kept for compatibility; `forge doc` no longer reads it.
    ///
    /// Source links are now derived from the project root and each source file's path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Glob patterns of files to leave out of the generated documentation.
    ///
    /// Patterns are resolved relative to the project root.
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
            commit: None,
            path: None,
            ignore: Vec::default(),
        }
    }
}
