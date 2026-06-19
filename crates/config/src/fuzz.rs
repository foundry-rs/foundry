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
    /// Optional 1-based fuzz run to execute.
    pub run: Option<u32>,
    /// Optional fuzz worker ID to pair with `run`.
    pub worker: Option<u32>,
    /// Fails the fuzzed test if a revert occurs.
    pub fail_on_revert: bool,
    /// The maximum number of test case rejections allowed,
    /// encountered during usage of `vm.assume` cheatcode.
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
            run: None,
            worker: None,
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
    #[serde(
        deserialize_with = "crate::deserialize_usize_or_max",
        serialize_with = "crate::serialize_usize_or_max"
    )]
    pub max_fuzz_dictionary_addresses: usize,
    /// How many values to record at most.
    /// Once the fuzzer exceeds this limit, it will start evicting random entries
    #[serde(
        deserialize_with = "crate::deserialize_usize_or_max",
        serialize_with = "crate::serialize_usize_or_max"
    )]
    pub max_fuzz_dictionary_values: usize,
    /// How many literal values to seed from the AST, at most.
    ///
    /// This value is independent from the max amount of addresses and values.
    #[serde(
        deserialize_with = "crate::deserialize_usize_or_max",
        serialize_with = "crate::serialize_usize_or_max"
    )]
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
    /// Whether EVM edge coverage should use collision-free dense IDs.
    pub evm_edge_coverage_collision_free: bool,
    /// Whether EVM edge coverage IDs should include call-frame depth.
    pub evm_edge_coverage_include_call_depth: bool,
    /// Whether to collect edge coverage from native Rust crates compiled with
    /// SanitizerCoverage instrumentation (e.g. precompile implementations).
    /// Requires building forge with a `RUSTC_WRAPPER` that injects sancov flags.
    pub sancov_edges: bool,
    /// Whether to capture comparison operands from sancov-instrumented crates
    /// and inject them into the fuzz dictionary. Independent of `sancov_edges`.
    pub sancov_trace_cmp: bool,
    /// Percent chance of generating a fresh transaction sequence instead of reusing the current
    /// corpus sequence during coverage-guided invariant campaigns.
    #[serde(deserialize_with = "crate::deserialize_stringified_percent")]
    pub corpus_random_sequence_weight: u32,
    /// Weights for coverage-guided corpus mutation strategies.
    #[serde(flatten)]
    pub mutation_weights: FuzzCorpusMutationWeights,
}

impl FuzzCorpusConfig {
    pub fn with_test(&mut self, contract: &str, test: &str) {
        if let Some(corpus_dir) = &self.corpus_dir {
            self.corpus_dir = Some(canonicalized(corpus_dir.join(contract).join(test)));
        }
    }

    /// Whether any edge coverage (EVM or sancov) should be collected.
    pub const fn collect_edge_coverage(&self) -> bool {
        self.corpus_dir.is_some() || self.show_edge_coverage || self.sancov_edges
    }

    /// Whether the EVM `EdgeCovInspector` should be enabled.
    ///
    /// Disabled when sancov edge coverage is active — sancov provides the
    /// coverage signal and EVM hits from the Solidity handler would dilute it.
    /// Trace-cmp-only mode keeps EVM edges enabled since trace-cmp only
    /// contributes dictionary entries, not edge coverage.
    pub const fn collect_evm_edge_coverage(&self) -> bool {
        !self.sancov_edges && (self.corpus_dir.is_some() || self.show_edge_coverage)
    }

    /// Whether EVM comparison operand capture is enabled.
    ///
    /// EVM comparison operands are only useful for coverage-guided fuzzing, so they are derived
    /// from corpus mode. Disabled when sancov edge coverage is active because sancov replaces EVM
    /// bytecode coverage as the guidance signal.
    pub const fn collect_evm_cmp_log(&self) -> bool {
        !self.sancov_edges && self.corpus_dir.is_some()
    }

    /// Whether EVM edge coverage should use collision-free dense IDs.
    pub const fn evm_edge_coverage_collision_free(&self) -> bool {
        self.evm_edge_coverage_collision_free
    }

    /// Whether EVM edge coverage IDs should include call-frame depth.
    pub const fn evm_edge_coverage_include_call_depth(&self) -> bool {
        self.evm_edge_coverage_include_call_depth
    }

    /// Whether sancov edge coverage collection is enabled.
    pub const fn collect_sancov_edges(&self) -> bool {
        self.sancov_edges
    }

    /// Whether sancov trace-cmp capture is enabled.
    pub const fn collect_sancov_trace_cmp(&self) -> bool {
        self.sancov_trace_cmp
    }

    /// Whether either sancov coverage mode is active.
    pub const fn sancov_active(&self) -> bool {
        self.sancov_edges || self.sancov_trace_cmp
    }

    /// Whether coverage guided fuzzing is enabled.
    pub const fn is_coverage_guided(&self) -> bool {
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
            evm_edge_coverage_collision_free: true,
            evm_edge_coverage_include_call_depth: false,
            sancov_edges: false,
            sancov_trace_cmp: false,
            corpus_random_sequence_weight: 50,
            mutation_weights: FuzzCorpusMutationWeights::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzCorpusMutationWeights {
    /// Weight for splicing two corpus sequences.
    pub mutation_weight_splice: u32,
    /// Weight for repeating part of a corpus sequence.
    pub mutation_weight_repeat: u32,
    /// Weight for interleaving two corpus sequences.
    pub mutation_weight_interleave: u32,
    /// Weight for replacing a corpus sequence prefix with generated calls.
    pub mutation_weight_prefix: u32,
    /// Weight for replacing a corpus sequence suffix with generated calls.
    pub mutation_weight_suffix: u32,
    /// Weight for ABI-aware argument mutation.
    pub mutation_weight_abi: u32,
    /// Weight for comparison-operand guided argument mutation.
    pub mutation_weight_cmp: u32,
}

impl FuzzCorpusMutationWeights {
    pub const fn total(&self) -> u64 {
        self.mutation_weight_splice as u64
            + self.mutation_weight_repeat as u64
            + self.mutation_weight_interleave as u64
            + self.mutation_weight_prefix as u64
            + self.mutation_weight_suffix as u64
            + self.mutation_weight_abi as u64
            + self.mutation_weight_cmp as u64
    }

    pub const fn abi_or_cmp_total(&self) -> u64 {
        self.mutation_weight_abi as u64 + self.mutation_weight_cmp as u64
    }

    /// Returns defaults if every configured weight is zero.
    pub fn effective(self) -> Self {
        if self.total() == 0 { Self::default() } else { self }
    }
}

impl Default for FuzzCorpusMutationWeights {
    fn default() -> Self {
        Self {
            mutation_weight_splice: 1,
            mutation_weight_repeat: 1,
            mutation_weight_interleave: 1,
            mutation_weight_prefix: 1,
            mutation_weight_suffix: 1,
            mutation_weight_abi: 1,
            mutation_weight_cmp: 1,
        }
    }
}
