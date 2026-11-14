//! Configuration for fuzz testing.

use alloy_primitives::U256;
use foundry_compilers::utils::canonicalized;
use serde::{Deserialize, Deserializer, Serialize, de::Error};
use std::path::PathBuf;

/// Fuzz testing configuration: the `[fuzz]` section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzConfig {
    /// Number of fuzz runs (generated inputs) to execute for each fuzz test.
    ///
    /// Higher values give more confidence in the results at the cost of testing speed. Must be
    /// greater than 0. With `timeout` set, the campaign stops at whichever comes first.
    #[serde(deserialize_with = "deserialize_fuzz_runs")]
    pub runs: u32,
    /// Execute only the fuzz case at this 1-based run index instead of the whole campaign.
    ///
    /// The case is regenerated from the worker's random stream, so it reproduces a case from an
    /// earlier campaign only with the same `seed` and `worker`, such as the ones a branch frontier
    /// record lists. When set, the test runs that single case on a single worker.
    pub run: Option<u32>,
    /// Worker whose fuzz case `run` executes.
    ///
    /// A campaign splits its runs across workers, and each worker derives its own random stream
    /// from `seed` (worker `0` uses `seed` unchanged). Only takes effect together with `run`; when
    /// unset, worker `0` is used.
    pub worker: Option<u32>,
    /// Whether a fuzz case fails when the test call reverts.
    ///
    /// When `true`, any revert fails the test. When `false`, a case whose revert originated in a
    /// contract other than the test contract is treated as passing; reverts raised by the test
    /// contract itself or by cheatcodes, such as failed assertions, still fail it.
    pub fail_on_revert: bool,
    /// Maximum number of inputs rejected with `vm.assume` before the whole fuzz test fails.
    ///
    /// Rejections are counted across all runs of the test: a rejected input is discarded and a new
    /// one is generated. If a test hits this limit, narrow its inputs with `bound` instead of
    /// rejecting them, or raise the limit.
    pub max_test_rejects: u32,
    /// Seed for the fuzzer's random number generator, which makes fuzz and invariant campaigns
    /// reproducible.
    ///
    /// Accepts an integer, or a decimal or hexadecimal string such as `"0x1234"`. Each worker
    /// derives its own random stream from the seed; worker `0` uses it unchanged. In fuzz tests
    /// the seed also drives cheatcode randomness, such as `vm.randomUint`. Invariant tests read
    /// this value too: there is no `invariant.seed`. When unset, every campaign uses a fresh
    /// random seed.
    pub seed: Option<U256>,
    /// Fuzz dictionary settings, flattened into this section.
    #[serde(flatten)]
    pub dictionary: FuzzDictionaryConfig,
    /// Number of fuzz runs whose traces are kept for the gas report.
    ///
    /// At least one run is always kept, so its trace can be displayed.
    pub gas_report_samples: u32,
    /// Fuzz corpus settings, flattened into this section.
    #[serde(flatten)]
    pub corpus: FuzzCorpusConfig,
    /// Directory where fuzz failures are recorded and replayed.
    ///
    /// When a fuzz test fails, its counterexample is saved under `failures/<contract>/<test>` in
    /// this directory and replayed first the next time the test runs, so the test keeps failing
    /// until the bug is fixed. `forge clean` removes the directory.
    pub failure_persist_dir: Option<PathBuf>,
    /// Whether to show the `console.log` output of every fuzz run instead of only the last one.
    ///
    /// Logs are displayed only at verbosity 2 or higher: run `forge test -vv` or set `verbosity`
    /// to at least 2. Without this setting, only the logs of the last run, or of the failing case,
    /// are shown.
    pub show_logs: bool,
    /// Time limit, in seconds, for each fuzz test campaign.
    ///
    /// The campaign stops at the timeout or after `runs` runs, whichever comes first; stopping at
    /// the timeout does not fail the test. When unset, there is no time limit.
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
            corpus: FuzzCorpusConfig { payable_value_weight: 0, ..Default::default() },
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

/// Fuzz dictionary configuration, shared by the `[fuzz]` and `[invariant]` sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzDictionaryConfig {
    /// Percentage (0 to 100) of fuzzed values drawn from the fuzz dictionary instead of generated
    /// at random.
    ///
    /// The dictionary holds values collected from the project and its execution, such as literals
    /// in the source code, values in storage, constants pushed by contract bytecode and addresses
    /// of deployed contracts. A higher weight biases inputs towards these "interesting" values; a
    /// lower weight favors random ones. Invariant campaigns also use it to pick senders and block
    /// delays.
    #[serde(deserialize_with = "crate::deserialize_stringified_percent")]
    pub dictionary_weight: u32,
    /// Whether to add the storage values of contracts touched during execution to the fuzz
    /// dictionary.
    pub include_storage: bool,
    /// Whether to add the constants pushed by contract bytecode (`PUSH` operands) and the
    /// contracts' addresses to the fuzz dictionary.
    pub include_push_bytes: bool,
    /// Whether invariant campaigns add the results of targeted contracts' view functions to the
    /// fuzz dictionary.
    ///
    /// After each successful call, view functions with no inputs or only address inputs are called
    /// on the targeted contracts that the call accessed, with address inputs set to the call's
    /// sender, and their results, whole structs included, are added to the dictionary. This lets
    /// later calls pass back values that only exist in storage, such as a struct read from a
    /// getter. At most 32 views are called after each call; when more are eligible, the selection
    /// rotates across calls so every view is reached.
    ///
    /// Only affects invariant tests; like the other shared keys, a value set under `[fuzz]` also
    /// applies to `[invariant]` unless `[invariant]` sets its own.
    pub include_view_values: bool,
    /// Maximum number of addresses kept in the fuzz dictionary.
    ///
    /// Once the limit is exceeded, the fuzzer starts evicting random entries. The limit prevents
    /// memory blowup. Set it to `"max"` for no limit.
    #[serde(
        deserialize_with = "crate::deserialize_usize_or_max",
        serialize_with = "crate::serialize_usize_or_max"
    )]
    pub max_fuzz_dictionary_addresses: usize,
    /// Maximum number of values kept in the fuzz dictionary.
    ///
    /// Once the limit is exceeded, the fuzzer starts evicting random entries. The dictionary
    /// always keeps a zero value, so the effective minimum is one. Set it to `"max"` for no limit.
    #[serde(
        deserialize_with = "crate::deserialize_usize_or_max",
        serialize_with = "crate::serialize_usize_or_max"
    )]
    pub max_fuzz_dictionary_values: usize,
    /// Maximum number of literal values from the source code (AST) seeded into the fuzz
    /// dictionary.
    ///
    /// This limit is independent of `max_fuzz_dictionary_addresses` and
    /// `max_fuzz_dictionary_values`. Set it to `"max"` for no limit.
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
            include_view_values: false,
            max_fuzz_dictionary_addresses: 300 * MB / 20,
            max_fuzz_dictionary_values: 300 * MB / 32,
            max_fuzz_dictionary_literals: 200 * MB / 32,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzCorpusConfig {
    /// Directory for the corpus; setting it enables coverage-guided fuzzing.
    ///
    /// In coverage-guided mode, inputs (call sequences, for invariant tests) that reach new edge
    /// coverage are kept in a corpus, persisted in this directory per contract and test, and
    /// mutated to generate new inputs, and edge coverage metrics are displayed. When unset, inputs
    /// that produce new coverage are neither persisted nor mutated. `forge clean` removes the
    /// directory.
    pub corpus_dir: Option<PathBuf>,
    /// Directory for branch frontier artifacts, which symbolic execution can follow up on.
    ///
    /// When set, and `frontier_limit` is above 0, each test records the comparisons its inputs
    /// reached without flipping, each with the concrete call sequence that reached it and how
    /// close its operands came, in this directory per contract and test. The `[symbolic]` settings
    /// `use_fuzz_frontiers` and `check_invariant_frontiers` read these records to try to take the
    /// missed branches. Frontier capture is not supported together with `invariant.call_override`.
    pub frontier_dir: Option<PathBuf>,
    /// Maximum number of branch frontier records written for one test.
    ///
    /// Records are deduplicated per comparison site across workers, keeping the closest one. `0`
    /// disables frontier capture even when `frontier_dir` is set.
    pub frontier_limit: usize,
    /// Whether corpus entries are written with gzip compression.
    ///
    /// Only entries above a small size threshold are compressed. When `false`, every entry is
    /// written as a plain JSON file. Only relevant when `corpus_dir` is set.
    pub corpus_gzip: bool,
    /// Number of times a corpus entry must be mutated before it may be evicted from memory.
    ///
    /// Only entries mutated more than this many times that are not favored can be evicted from the
    /// in-memory corpus. Only relevant when `corpus_dir` is set.
    pub corpus_min_mutations: usize,
    /// Number of corpus entries kept in memory without eviction.
    ///
    /// Entries are evicted only while the in-memory corpus holds more than this many entries, and
    /// always at least one is kept. Only relevant when `corpus_dir` is set.
    pub corpus_min_size: usize,
    /// Whether to collect and display edge coverage metrics.
    ///
    /// Edge coverage is always collected when `corpus_dir` is set; enable this to see it without
    /// coverage-guided fuzzing.
    pub show_edge_coverage: bool,
    /// Whether EVM edge coverage gives every edge its own ID instead of hashing edges into a fixed
    /// map.
    ///
    /// When `true`, distinct edges never share a coverage entry. When `false`, edges are hashed
    /// into a fixed 64 KiB map, where unrelated edges can collide and hide new coverage.
    pub evm_edge_coverage_collision_free: bool,
    /// Whether EVM edge coverage IDs include the call-frame depth.
    ///
    /// When `true`, the same edge reached at different call depths counts as different coverage,
    /// which rewards inputs that reach code through new call paths but enlarges the coverage map.
    pub evm_edge_coverage_include_call_depth: bool,
    /// Whether to collect edge coverage from native Rust crates compiled with SanitizerCoverage
    /// instrumentation, such as precompile implementations.
    ///
    /// Requires building forge with a `RUSTC_WRAPPER` that injects sancov flags. When enabled,
    /// sancov edges replace EVM edges as the coverage signal, and EVM comparison operands are no
    /// longer captured for mutation.
    pub sancov_edges: bool,
    /// Whether to capture comparison operands from sancov-instrumented native crates and add them
    /// to the fuzz dictionary.
    ///
    /// Independent of `sancov_edges`: when only this is enabled, EVM edge coverage stays on, since
    /// comparison operands only add dictionary entries. Requires the same sancov-instrumented
    /// forge build.
    pub sancov_trace_cmp: bool,
    /// Whether EVM edge coverage comes from compiler instrumentation instead of the `JUMP`/`JUMPI`
    /// edge inspector.
    ///
    /// Coverage builds of a compiler emit `PUSHn <id>; EDGECOV` (opcode `0xC0`) on every
    /// control-flow edge, with dense IDs starting at 1. When enabled, the EVM executes that
    /// opcode, which records a hit for the edge and costs no gas (it also refunds its `PUSH`), and
    /// these hits replace the inspector's EVM edges as the corpus guidance signal. Comparison
    /// operands then come from the compiler's `CMPLOG` (`0xC1`) probes instead of the cmp-log
    /// inspector. Instrumented artifacts only run with this on: on any other EVM `0xC0` and `0xC1`
    /// are undefined opcodes.
    ///
    /// Hits are keyed by ID alone, so IDs must be unique across all instrumented code: build it in
    /// one compiler invocation.
    pub instrumented_edges: bool,
    /// Percent chance (0 to 100) of generating a fresh input instead of mutating a corpus entry in
    /// coverage-guided campaigns.
    ///
    /// In invariant campaigns with more than two workers, the last worker uses 50 instead, to
    /// explore more broadly, unless `[invariant]` sets this key or it differs from its default.
    #[serde(deserialize_with = "crate::deserialize_stringified_percent")]
    pub corpus_random_sequence_weight: u32,
    /// Percent chance (0 to 100) that a generated call to a payable function carries a non-zero
    /// `msg.value`.
    ///
    /// `0` never sends value. Values are biased towards edge cases and dictionary values, and
    /// amounts above the sender's balance are clamped to it.
    #[serde(deserialize_with = "crate::deserialize_stringified_percent")]
    pub payable_value_weight: u32,
    /// Weights for coverage-guided corpus mutation strategies, flattened into this section.
    #[serde(flatten)]
    pub mutation_weights: FuzzCorpusMutationWeights,
}

impl FuzzCorpusConfig {
    pub const DEFAULT_CORPUS_RANDOM_SEQUENCE_WEIGHT: u32 = 10;
    pub const ENSEMBLE_CORPUS_RANDOM_SEQUENCE_WEIGHT: u32 = 50;
    pub const DEFAULT_FRONTIER_LIMIT: usize = 256;

    pub fn with_test(&mut self, contract: &str, test: &str) {
        if let Some(corpus_dir) = &self.corpus_dir {
            self.corpus_dir = Some(canonicalized(corpus_dir.join(contract).join(test)));
        }
        if let Some(frontier_dir) = &self.frontier_dir {
            self.frontier_dir = Some(canonicalized(frontier_dir.join(contract).join(test)));
        }
    }

    /// Whether any edge coverage (EVM, instrumented or sancov) should be collected.
    pub const fn collect_edge_coverage(&self) -> bool {
        self.corpus_dir.is_some()
            || self.show_edge_coverage
            || self.sancov_edges
            || self.instrumented_edges
    }

    /// Whether the EVM `EdgeCovInspector` should be enabled.
    ///
    /// Disabled when sancov edge coverage is active — sancov provides the
    /// coverage signal and EVM hits from the Solidity handler would dilute it.
    /// Also disabled when instrumented edges are on, since compiler instrumentation then
    /// provides the EVM edges.
    /// Trace-cmp-only mode keeps EVM edges enabled since trace-cmp only
    /// contributes dictionary entries, not edge coverage.
    pub const fn collect_evm_edge_coverage(&self) -> bool {
        !self.sancov_edges
            && !self.instrumented_edges
            && (self.corpus_dir.is_some() || self.show_edge_coverage)
    }

    /// Whether EVM edge coverage comes from the compiler-instrumented `EDGECOV` opcode.
    pub const fn collect_instrumented_edges(&self) -> bool {
        self.instrumented_edges
    }

    /// Whether EVM comparison operand capture is enabled.
    ///
    /// EVM comparison operands for corpus mutation are only useful for coverage-guided fuzzing, so
    /// they are derived from corpus mode and disabled when sancov edge coverage is active.
    /// Frontier capture still records EVM comparison sites because those artifacts target
    /// Solidity bytecode branches, not the active coverage guidance source.
    pub fn collect_evm_cmp_log(&self) -> bool {
        self.capture_branch_frontiers()
            || (!self.sancov_edges
                && self.corpus_dir.is_some()
                && self.mutation_weights.effective().mutation_weight_cmp > 0)
    }

    /// Whether fuzz branch frontier artifacts should be captured.
    pub const fn capture_branch_frontiers(&self) -> bool {
        self.frontier_dir.is_some() && self.frontier_limit > 0
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
            frontier_dir: None,
            frontier_limit: Self::DEFAULT_FRONTIER_LIMIT,
            corpus_gzip: true,
            corpus_min_mutations: 5,
            corpus_min_size: 0,
            show_edge_coverage: false,
            evm_edge_coverage_collision_free: true,
            evm_edge_coverage_include_call_depth: false,
            sancov_edges: false,
            sancov_trace_cmp: false,
            instrumented_edges: false,
            corpus_random_sequence_weight: Self::DEFAULT_CORPUS_RANDOM_SEQUENCE_WEIGHT,
            payable_value_weight: 15,
            mutation_weights: FuzzCorpusMutationWeights::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzCorpusMutationWeights {
    /// Relative weight of the coverage-guided mutation that splices two corpus sequences together.
    ///
    /// Mutation strategies are picked in proportion to their weights, and `0` disables a strategy.
    /// If every `mutation_weight_*` key is 0, all of them fall back to their defaults.
    pub mutation_weight_splice: u32,
    /// Relative weight of the coverage-guided mutation that repeats part of a corpus sequence.
    ///
    /// Mutation strategies are picked in proportion to their weights, and `0` disables a strategy.
    /// If every `mutation_weight_*` key is 0, all of them fall back to their defaults.
    pub mutation_weight_repeat: u32,
    /// Relative weight of the coverage-guided mutation that interleaves the calls of two corpus
    /// sequences.
    ///
    /// Mutation strategies are picked in proportion to their weights, and `0` disables a strategy.
    /// If every `mutation_weight_*` key is 0, all of them fall back to their defaults.
    pub mutation_weight_interleave: u32,
    /// Relative weight of the coverage-guided mutation that replaces the start of a corpus
    /// sequence with generated calls.
    ///
    /// Mutation strategies are picked in proportion to their weights, and `0` disables a strategy.
    /// If every `mutation_weight_*` key is 0, all of them fall back to their defaults.
    pub mutation_weight_prefix: u32,
    /// Relative weight of the coverage-guided mutation that replaces the end of a corpus sequence
    /// with generated calls.
    ///
    /// Mutation strategies are picked in proportion to their weights, and `0` disables a strategy.
    /// If every `mutation_weight_*` key is 0, all of them fall back to their defaults.
    pub mutation_weight_suffix: u32,
    /// Relative weight of the coverage-guided mutation that mutates call arguments based on their
    /// ABI types.
    ///
    /// Mutation strategies are picked in proportion to their weights, and `0` disables a strategy.
    /// If every `mutation_weight_*` key is 0, all of them fall back to their defaults.
    pub mutation_weight_abi: u32,
    /// Relative weight of the coverage-guided mutation that replaces call arguments with operands
    /// of comparisons seen during execution.
    ///
    /// Mutation strategies are picked in proportion to their weights, and `0` disables a strategy.
    /// If every `mutation_weight_*` key is 0, all of them fall back to their defaults. At 0, EVM
    /// comparison operands are not captured unless branch frontiers are.
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

fn deserialize_fuzz_runs<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let runs = u32::deserialize(deserializer)?;
    if runs == 0 {
        return Err(D::Error::custom("`fuzz.runs` must be greater than 0"));
    }
    Ok(runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instrumented_edges_replace_evm_edge_coverage() {
        let corpus = FuzzCorpusConfig {
            corpus_dir: Some("corpus".into()),
            instrumented_edges: true,
            ..Default::default()
        };
        assert!(corpus.collect_edge_coverage());
        assert!(corpus.collect_instrumented_edges());
        assert!(!corpus.collect_evm_edge_coverage());
        // Cmp-log is configured independently of the edge source.
        assert!(corpus.collect_evm_cmp_log());

        let default = FuzzCorpusConfig { corpus_dir: Some("corpus".into()), ..Default::default() };
        assert!(!default.collect_instrumented_edges());
        assert!(default.collect_evm_edge_coverage());
    }
}
