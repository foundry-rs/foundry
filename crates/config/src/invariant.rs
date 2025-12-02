//! Configuration for invariant testing

use crate::fuzz::{FuzzCorpusConfig, FuzzDictionaryConfig};
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
    /// The fuzz corpus configuration.
    #[serde(flatten)]
    pub corpus: FuzzCorpusConfig,
    /// Path where invariant failures are recorded and replayed.
    pub failure_persist_dir: Option<PathBuf>,
    /// Whether to collect and display fuzzed selectors metrics.
    pub show_metrics: bool,
    /// Optional timeout (in seconds) for each invariant test.
    pub timeout: Option<u32>,
    /// Display counterexample as solidity calls.
    pub show_solidity: bool,
    /// Maximum time (in seconds) between generated txs.
    pub max_time_delay: Option<u32>,
    /// Maximum number of blocks elapsed between generated txs.
    pub max_block_delay: Option<u32>,
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
            corpus: FuzzCorpusConfig::default(),
            failure_persist_dir: None,
            show_metrics: true,
            timeout: None,
            show_solidity: false,
            max_time_delay: None,
            max_block_delay: None,
        }
    }
}

impl InvariantConfig {
    /// Creates invariant configuration to write failures in `{PROJECT_ROOT}/cache/fuzz` dir.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self { failure_persist_dir: Some(cache_dir), ..Default::default() }
    }
}
