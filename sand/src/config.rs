//! Configurable abstractions

use std::path::PathBuf;

// TODO parts can be reused from ethers-solc

/// Where to find all files or where to write them
#[derive(Debug, Clone)]
pub struct ProjectPathsConfig {
    /// Project root
    pub root: PathBuf,
    /// Where to store build artifacts
    pub artifacts: PathBuf,
    /// Where to find sources
    pub sources: PathBuf,
    /// Where to find tests
    pub tests: PathBuf,
    /// Where to look for libraries
    pub libraries: Vec<PathBuf>,
}

impl Default for ProjectPathsConfig {
    fn default() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| "".into());

        Self {
            artifacts: root.join("artifacts"),
            sources: root.join("contracts"),
            tests: root.join("test"),
            root,
            libraries: vec![],
        }
    }
}
