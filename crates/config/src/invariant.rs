//! Configuration for invariant testing

use crate::fuzz::FuzzDictionaryConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Contains for invariant testing
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantConfig {
    /// The number of runs that must execute for each invariant test group.
    pub runs: u32,
    /// The number of calls executed to attempt to break invariants in one run.
    pub depth: u32,
    /// Fails the invariant fuzzing if a revert occurs
    pub fail_on_revert: bool,
    /// Allows overriding an unsafe external call when running invariant tests. eg. reentrancy
    /// checks
    pub call_override: bool,
    /// The fuzz dictionary configuration
    #[serde(flatten)]
    pub dictionary: FuzzDictionaryConfig,
    /// The maximum number of attempts to shrink the sequence
    pub shrink_run_limit: u32,
    /// The maximum number of rejects via `vm.assume` which can be encountered during a single
    /// invariant run.
    pub max_assume_rejects: u32,
    /// Number of runs to execute and include in the gas report.
    pub gas_report_samples: u32,
    /// Path where invariant corpus is stored, enables coverage guided fuzzing and edge coverage
    /// metrics.
    pub corpus_dir: Option<PathBuf>,
    /// Whether corpus to use gzip file compression and decompression.
    pub corpus_gzip: bool,
    // Number of corpus mutations until marked as eligible to be flushed from memory.
    pub corpus_min_mutations: usize,
    // Number of corpus that won't be evicted from memory.
    pub corpus_min_size: usize,
    /// Path where invariant failures are recorded and replayed.
    pub failure_persist_dir: Option<PathBuf>,
    /// Whether to collect and display fuzzed selectors metrics.
    pub show_metrics: bool,
    /// Optional timeout (in seconds) for each invariant test.
    pub timeout: Option<u32>,
    /// Display counterexample as solidity calls.
    pub show_solidity: bool,
    /// Whether to collect and display edge coverage metrics.
    pub show_edge_coverage: bool,
}

impl Default for InvariantConfig {
    fn default() -> Self {
        Self {
            runs: 256,
            depth: 500,
            fail_on_revert: false,
            call_override: false,
            dictionary: FuzzDictionaryConfig { dictionary_weight: 80, ..Default::default() },
            shrink_run_limit: 5000,
            max_assume_rejects: 65536,
            gas_report_samples: 256,
            corpus_dir: None,
            corpus_gzip: true,
            corpus_min_mutations: 5,
            corpus_min_size: 0,
            failure_persist_dir: None,
            show_metrics: true,
            timeout: None,
            show_solidity: false,
            show_edge_coverage: false,
        }
    }
}

impl InvariantConfig {
    /// Creates invariant configuration to write failures in `{PROJECT_ROOT}/cache/fuzz` dir.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            runs: 256,
            depth: 500,
            fail_on_revert: false,
            call_override: false,
            dictionary: FuzzDictionaryConfig { dictionary_weight: 80, ..Default::default() },
            shrink_run_limit: 5000,
            max_assume_rejects: 65536,
            gas_report_samples: 256,
            corpus_dir: None,
            corpus_gzip: true,
            corpus_min_mutations: 5,
            corpus_min_size: 0,
            failure_persist_dir: Some(cache_dir),
            show_metrics: true,
            timeout: None,
            show_solidity: false,
            show_edge_coverage: false,
        }
    }
}
