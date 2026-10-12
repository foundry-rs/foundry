//! Symbolic report data independent of the symbolic execution engine.

use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};

/// One concrete storage value required to replay a symbolic invariant candidate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolicStorageAssignment {
    /// Account whose storage slot should be initialized.
    pub address: Address,
    /// Concrete storage slot.
    pub slot: U256,
    /// Concrete value extracted from the solver model.
    pub value: U256,
}

/// High-level reason a symbolic run stopped without a proof or replayed counterexample.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolicStopReason {
    /// The executor reached a supported-but-incomplete semantic boundary.
    Stuck,
    /// Every explored execution path ended in an ordinary revert.
    RevertAll,
    /// The solver timed out or returned `unknown`.
    Timeout,
    /// An internal engine, backend, or solver process error occurred.
    Error,
}

/// Symbolic execution counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolicStats {
    /// Number of explored symbolic paths.
    pub paths: usize,
    /// Number of normalized solver queries issued during the run.
    pub solver_queries: usize,
    /// Number of queries sent to the SMT backend after local fast paths.
    #[serde(default)]
    pub smt_queries: usize,
    /// Number of satisfiability checks requested by the executor.
    #[serde(default)]
    pub sat_queries: usize,
    /// Number of concrete model requests requested by the executor.
    #[serde(default)]
    pub model_queries: usize,
    /// Number of satisfiability checks served from the normalized cache.
    #[serde(default)]
    pub sat_cache_hits: usize,
    /// Number of model requests served from the normalized model cache.
    #[serde(default)]
    pub model_cache_hits: usize,
    /// Number of satisfiable witnesses produced by local hard-arithmetic search.
    #[serde(default)]
    pub heuristic_witnesses: usize,
    /// Wall-clock time spent waiting on backend solver subprocesses, in milliseconds.
    #[serde(default)]
    pub solver_time_ms: u64,
    /// Total SMT-LIB input bytes sent to backend solver subprocesses.
    #[serde(default)]
    pub smt_input_bytes: u64,
    /// Largest single SMT-LIB query input sent to a backend solver subprocess, in bytes.
    #[serde(default)]
    pub smt_max_query_bytes: u64,
    /// Wall-clock time spent building SMT-LIB query strings, in milliseconds.
    #[serde(default)]
    pub smt_build_time_ms: u64,
    /// Longest single backend solver subprocess query, in milliseconds.
    #[serde(default)]
    pub smt_max_query_time_ms: u64,
}
