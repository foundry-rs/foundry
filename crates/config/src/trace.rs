//! Configuration for trace rendering.

use alloy_primitives::map::AddressHashMap;
use serde::{Deserialize, Serialize};

/// Configuration for trace rendering.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TracingConfig {
    /// Verbosity to use for trace rendering.
    pub verbosity: u8,
    /// Address labels to use in traces.
    #[serde(default, skip_serializing_if = "AddressHashMap::is_empty")]
    pub labels: AddressHashMap<String>,
    /// Whether to disable labels in traces.
    pub disable_labels: bool,
    /// Maximum depth of rendered traces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_depth: Option<usize>,
    /// Whether to identify internal functions in traces.
    pub decode_internal: bool,
}
