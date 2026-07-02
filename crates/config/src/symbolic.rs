//! Configuration for symbolic testing.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Storage modelling mode for symbolic tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolicStorageLayout {
    /// Model Solidity storage layout precisely where the symbolic executor knows the layout shape.
    #[default]
    Solidity,
    /// Treat every storage read as potentially arbitrary symbolic storage.
    Generic,
    /// Treat unwritten symbolic storage reads as zero.
    ZeroInit,
}

/// Pending symbolic path exploration order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolicExplorationOrder {
    /// Explore pending paths in first-in, first-out order.
    #[default]
    Bfs,
    /// Explore pending paths in last-in, first-out order.
    Dfs,
}

/// Configuration for symbolic testing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolicConfig {
    /// Whether symbolic tests are enabled.
    pub enabled: bool,
    /// Whether fuzz tests should be symbolically concretized into fuzz corpus seeds.
    pub seed_corpus: bool,
    /// Whether fuzz corpus seeds should guide symbolic fuzz-test exploration.
    pub use_fuzz_corpus: bool,
    /// Maximum number of fuzz corpus seeds to import for one symbolic run.
    pub corpus_seed_limit: usize,
    /// Whether fuzz branch frontiers should guide targeted symbolic fuzz-test seeding.
    pub use_fuzz_frontiers: bool,
    /// Maximum number of fuzz branch frontiers to try for one symbolic run.
    pub frontier_limit: usize,
    /// Solver executable to invoke.
    pub solver: String,
    /// Exact solver command to invoke. When set, this overrides `solver`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solver_command: Option<String>,
    /// Solver names or exact commands to race in parallel. Ignored when `solver_command` is set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub solver_portfolio: Vec<String>,
    /// Optional timeout in seconds for solver-backed symbolic execution.
    pub timeout: Option<u32>,
    /// Halmos-compatible loop bound accepted by config and annotations.
    #[serde(default, rename = "loop", skip_serializing_if = "Option::is_none")]
    pub loop_bound: Option<u32>,
    /// Halmos-compatible execution depth alias. When set, this overrides `max_depth`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
    /// Halmos-compatible path width alias. When set, this overrides `max_paths`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Maximum number of opcodes to execute along a path.
    pub max_depth: u32,
    /// Maximum number of symbolic paths to explore.
    pub max_paths: u32,
    /// Maximum number of calls in a bounded symbolic invariant sequence.
    pub invariant_depth: u32,
    /// Order used to select the next pending symbolic path.
    #[serde(default)]
    pub exploration_order: SymbolicExplorationOrder,
    /// Maximum number of solver queries.
    pub max_solver_queries: u32,
    /// Default bounded length for dynamic ABI inputs.
    pub default_dynamic_length: u32,
    /// Maximum permitted bounded length for a dynamic ABI input.
    pub max_dynamic_length: u32,
    /// Per-dynamic-leaf bounded lengths, applied in ABI traversal order.
    pub array_lengths: Vec<u32>,
    /// Per-symbolic-input bounded lengths keyed by ABI argument name or generated symbolic name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dynamic_lengths: BTreeMap<String, Vec<u32>>,
    /// Default bounded lengths for dynamic ABI arrays without an explicit length.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_array_lengths: Vec<u32>,
    /// Default bounded lengths for ABI `bytes` and `string` values without an explicit length.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_bytes_lengths: Vec<u32>,
    /// Maximum symbolic calldata size in bytes.
    pub max_calldata_bytes: u32,
    /// Whether symbolic call targets may be expanded over known deployed contracts.
    pub symbolic_call_targets: bool,
    /// Whether to dump SMT-LIB queries before invoking the configured solver.
    pub dump_smt: bool,
    /// Storage modelling mode used for symbolic storage reads.
    pub storage_layout: SymbolicStorageLayout,
}

impl Default for SymbolicConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            seed_corpus: false,
            use_fuzz_corpus: false,
            corpus_seed_limit: 32,
            use_fuzz_frontiers: false,
            frontier_limit: 256,
            solver: "z3".to_string(),
            solver_command: None,
            solver_portfolio: Vec::new(),
            timeout: Some(30),
            loop_bound: None,
            depth: None,
            width: None,
            max_depth: 10_000,
            max_paths: 1_024,
            invariant_depth: 10,
            exploration_order: SymbolicExplorationOrder::default(),
            max_solver_queries: 10_000,
            default_dynamic_length: 2,
            max_dynamic_length: 256,
            array_lengths: Vec::new(),
            dynamic_lengths: BTreeMap::new(),
            default_array_lengths: Vec::new(),
            default_bytes_lengths: Vec::new(),
            max_calldata_bytes: 4_096,
            symbolic_call_targets: false,
            dump_smt: false,
            storage_layout: SymbolicStorageLayout::Solidity,
        }
    }
}

impl SymbolicConfig {
    /// Returns the effective per-path opcode depth limit used by the symbolic executor.
    ///
    /// The Halmos-compatible `depth` alias takes precedence over `max_depth` so inline
    /// compatibility annotations and native Foundry config resolve to one internal limit.
    pub fn execution_depth(&self) -> u32 {
        self.depth.unwrap_or(self.max_depth)
    }

    /// Returns the effective symbolic path width limit used by the symbolic executor.
    ///
    /// The Halmos-compatible `width` alias takes precedence over `max_paths` so both
    /// configuration spellings feed the same path exploration budget.
    pub fn path_width(&self) -> u32 {
        self.width.unwrap_or(self.max_paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_exploration_order_defaults_to_bfs() {
        let value = serde_json::json!({
            "enabled": false,
            "seed_corpus": false,
            "use_fuzz_corpus": false,
            "corpus_seed_limit": 32,
            "use_fuzz_frontiers": false,
            "frontier_limit": 256,
            "solver": "z3",
            "timeout": 30,
            "max_depth": 10000,
            "max_paths": 1024,
            "invariant_depth": 10,
            "max_solver_queries": 10000,
            "default_dynamic_length": 2,
            "max_dynamic_length": 256,
            "array_lengths": [],
            "max_calldata_bytes": 4096,
            "symbolic_call_targets": false,
            "dump_smt": false,
            "storage_layout": "solidity"
        });

        let config: SymbolicConfig = serde_json::from_value(value).unwrap();

        assert_eq!(config.exploration_order, SymbolicExplorationOrder::Bfs);
    }
}
