use crate::from_vec_glob;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Contains the config for `forge bind-json`
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindJsonConfig {
    /// Path for the generated bindings file.
    pub out: PathBuf,
    /// Globs to include.
    ///
    /// If provided, only the files matching the globs will be included. Otherwise, defaults to
    /// including all project files.
    #[serde(with = "from_vec_glob")]
    pub include: Vec<globset::Glob>,
    /// Globs to ignore
    #[serde(with = "from_vec_glob")]
    pub exclude: Vec<globset::Glob>,
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
