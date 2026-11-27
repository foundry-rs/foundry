use crate::filter::GlobMatcher;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Contains the config for `forge bind-json`
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindJsonConfig {
    /// Solidity file that `forge bind-json` writes the generated bindings to.
    ///
    /// A relative path is resolved from the project root; a path passed to `forge bind-json`
    /// overrides it. The EIP-712 cheatcodes (`vm.eip712HashType`, `vm.eip712HashStruct`) look up
    /// type names in this file when no bindings path is given.
    pub out: PathBuf,
    /// Glob patterns of the source files whose structs get bindings.
    ///
    /// When set, only matching files are included. When empty, all project source files are
    /// included except those in library directories.
    pub include: Vec<GlobMatcher>,
    /// Glob patterns of source files whose structs get no bindings.
    ///
    /// Applied after `include`.
    pub exclude: Vec<GlobMatcher>,
}

impl Default for BindJsonConfig {
    fn default() -> Self {
        Self {
            out: PathBuf::from("utils/JsonBindings.sol"),
            exclude: Vec::new(),
            include: Vec::new(),
        }
    }
}
