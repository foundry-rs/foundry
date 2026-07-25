//! Configuration for trace rendering.

use alloy_primitives::map::AddressHashMap;
use serde::{Deserialize, Serialize};

/// Configuration for trace rendering.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TracingConfig {
    /// Verbosity to use for trace rendering.
    pub verbosity: u8,
    /// Address labels to use in traces.
    #[serde(default, skip_serializing_if = "AddressHashMap::is_empty")]
    pub labels: AddressHashMap<String>,
    /// Whether to disable labels in traces.
    pub disable_labels: bool,
    /// Whether to hide addresses in trace parameters when labels are available.
    pub compact_labels: bool,
    /// Maximum depth of rendered traces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_depth: Option<usize>,
    /// Whether to identify internal functions in traces.
    pub decode_internal: bool,
    /// Cumulative Sourcify/Etherscan metadata lookup budget, in seconds.
    /// Zero disables all external trace identification, including OpenChain.
    pub external_identification_timeout: u64,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            verbosity: 0,
            labels: Default::default(),
            disable_labels: false,
            compact_labels: false,
            trace_depth: None,
            decode_internal: false,
            external_identification_timeout: 5,
        }
    }
}
