use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::PathBuf;

/// Configuration for an external compiler adapter executable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalCompiler {
    /// Stable user-defined identifier used to namespace cache and artifact entries.
    pub id: String,
    /// Absolute or project-relative adapter executable path.
    pub command: PathBuf,
    /// Arguments passed to the adapter executable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Compiler-native project roots, relative to the Foundry project root.
    pub roots: Vec<PathBuf>,
    /// Opaque adapter-specific settings.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub settings: Map<String, Value>,
}
