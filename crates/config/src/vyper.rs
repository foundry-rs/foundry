//! Vyper specific configuration types.

use foundry_compilers::compilers::vyper::settings::VyperOptimizationMode;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VyperConfig {
    /// Vyper optimization mode. "gas", "none" or "codesize"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optimize: Option<VyperOptimizationMode>,
    /// The Vyper instance to use if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}
