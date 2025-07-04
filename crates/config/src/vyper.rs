//! Vyper specific configuration types.

use foundry_compilers::artifacts::{vyper::VyperOptimizationMode, EvmVersion};
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
    /// Optionally enables experimental Venom pipeline
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental_codegen: Option<bool>,
}

/// Vyper does not yet support the Prague EVM version, so we normalize it to Cancun.
/// This is a temporary workaround until Vyper supports Prague.
pub fn normalize_evm_version_vyper(evm_version: EvmVersion) -> EvmVersion {
    if evm_version >= EvmVersion::Prague {
        return EvmVersion::Cancun;
    }

    evm_version
}
