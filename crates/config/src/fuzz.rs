//! Configuration for fuzz testing.

use alloy_primitives::U256;
use foundry_compilers::utils::canonicalized;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Contains for fuzz testing
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzConfig {
    /// The number of test cases that must execute for each property test
    pub runs: u32,
    /// Fails the fuzzed test if a revert occurs.
    pub fail_on_revert: bool,
    /// The maximum number of test case rejections allowed by proptest, to be
    /// encountered during usage of `vm.assume` cheatcode. This will be used
    /// to set the `max_global_rejects` value in proptest test runner config.
    /// `max_local_rejects` option isn't exposed here since we're not using
    /// `prop_filter`.
    pub max_test_rejects: u32,
    /// Optional seed for the fuzzing RNG algorithm
    pub seed: Option<U256>,
    /// The fuzz dictionary configuration
    #[serde(flatten)]
    pub dictionary: FuzzDictionaryConfig,
    /// Number of runs to execute and include in the gas report.
    pub gas_report_samples: u32,
    /// The fuzz corpus configuration.
    #[serde(flatten)]
    pub corpus: FuzzCorpusConfig,
    /// Path where fuzz failures are recorded and replayed.
    pub failure_persist_dir: Option<PathBuf>,
    /// show `console.log` in fuzz test, defaults to `false`
    pub show_logs: bool,
    /// Optional timeout (in seconds) for each property test
    pub timeout: Option<u32>,
}

impl Default for FuzzConfig {
    fn default() -> Self {
        Self {
            runs: 256,
            fail_on_revert: true,
            max_test_rejects: 65536,
            seed: None,
            dictionary: FuzzDictionaryConfig::default(),
            gas_report_samples: 256,
            corpus: FuzzCorpusConfig::default(),
            failure_persist_dir: None,
            show_logs: false,
            timeout: None,
        }
    }
}

impl FuzzConfig {
    /// Creates fuzz configuration to write failures in `{PROJECT_ROOT}/cache/fuzz` dir.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self { failure_persist_dir: Some(cache_dir), ..Default::default() }
    }
}

/// Contains for fuzz testing
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzDictionaryConfig {
    /// The weight of the dictionary
    #[serde(deserialize_with = "crate::deserialize_stringified_percent")]
    pub dictionary_weight: u32,
    /// The flag indicating whether to include values from storage
    pub include_storage: bool,
    /// The flag indicating whether to include push bytes values
    pub include_push_bytes: bool,
    /// How many addresses to record at most.
    /// Once the fuzzer exceeds this limit, it will start evicting random entries
    ///
    /// This limit is put in place to prevent memory blowup.
    #[serde(deserialize_with = "crate::deserialize_usize_or_max")]
    pub max_fuzz_dictionary_addresses: usize,
    /// How many values to record at most.
    /// Once the fuzzer exceeds this limit, it will start evicting random entries
    #[serde(deserialize_with = "crate::deserialize_usize_or_max")]
    pub max_fuzz_dictionary_values: usize,
    /// How many literal values to seed from the AST, at most.
    ///
    /// This value is independent from the max amount of addresses and values.
    #[serde(deserialize_with = "crate::deserialize_usize_or_max")]
    pub max_fuzz_dictionary_literals: usize,
}

impl Default for FuzzDictionaryConfig {
    fn default() -> Self {
        const MB: usize = 1024 * 1024;

        Self {
            dictionary_weight: 40,
            include_storage: true,
            include_push_bytes: true,
            max_fuzz_dictionary_addresses: 300 * MB / 20,
            max_fuzz_dictionary_values: 300 * MB / 32,
            max_fuzz_dictionary_literals: 200 * MB / 32,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzCorpusConfig {
    // Path to corpus directory, enabled coverage guided fuzzing mode.
    // If not set then sequences producing new coverage are not persisted and mutated.
    pub corpus_dir: Option<PathBuf>,
    // Whether corpus to use gzip file compression and decompression.
    pub corpus_gzip: bool,
    // Number of mutations until entry marked as eligible to be flushed from in-memory corpus.
    // Mutations will be performed at least `corpus_min_mutations` times.
    pub corpus_min_mutations: usize,
    // Number of corpus that won't be evicted from memory.
    pub corpus_min_size: usize,
    /// Whether to collect and display edge coverage metrics.
    pub show_edge_coverage: bool,
}

impl FuzzCorpusConfig {
    pub fn with_test(&mut self, contract: &str, test: &str) {
        if let Some(corpus_dir) = &self.corpus_dir {
            self.corpus_dir = Some(canonicalized(corpus_dir.join(contract).join(test)));
        }
    }

    /// Whether edge coverage should be collected and displayed.
    pub fn collect_edge_coverage(&self) -> bool {
        self.corpus_dir.is_some() || self.show_edge_coverage
    }

    /// Whether coverage guided fuzzing is enabled.
    pub fn is_coverage_guided(&self) -> bool {
        self.corpus_dir.is_some()
    }
}

impl Default for FuzzCorpusConfig {
    fn default() -> Self {
        Self {
            corpus_dir: None,
            corpus_gzip: true,
            corpus_min_mutations: 5,
            corpus_min_size: 0,
            show_edge_coverage: false,
        }
    }
}
