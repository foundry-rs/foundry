//! Vyper specific configuration types.

use foundry_compilers::artifacts::vyper::VyperOptimizationMode;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VyperConfig {
    /// Vyper optimization mode. "gas", "none" or "codesize"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optimize: Option<VyperOptimizationMode>,
    /// The Vyper instance to use if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}
