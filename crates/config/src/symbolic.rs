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
    /// Whether to run symbolic tests.
    ///
    /// When enabled, `forge test` runs `check*` and `prove*` functions symbolically, and
    /// `invariant*` and `statefulFuzz*` functions explore a bounded symbolic call sequence.
    /// A `PASS` holds only for the modeled EVM surface and the configured bounds, and a
    /// counterexample is reported only after it replays concretely. Symbolic tests need the
    /// configured `solver` installed. The `--symbolic` flag also enables this.
    pub enabled: bool,
    /// Whether to run fuzz tests symbolically and persist the concrete inputs found to the fuzz
    /// corpus.
    ///
    /// Non-failing concrete inputs from the symbolic run seed later fuzz campaigns. This requires
    /// `fuzz.corpus_dir`; without it, seeding is skipped with a warning.
    pub seed_corpus: bool,
    /// Whether to run fuzz tests symbolically, using existing fuzz corpus entries as
    /// path-priority hints.
    ///
    /// Fuzz tests run as symbolic tests instead of being fuzzed. Up to `corpus_seed_limit`
    /// entries are imported from `fuzz.corpus_dir`; without a corpus directory, the test runs
    /// without imported seeds and a warning is shown.
    pub use_fuzz_corpus: bool,
    /// Maximum number of fuzz corpus entries to import for one symbolic test.
    ///
    /// Applies when `use_fuzz_corpus` is enabled. `0` imports none.
    pub corpus_seed_limit: usize,
    /// Whether to solve for branches recorded in fuzz branch frontier artifacts and add the
    /// results to the corpus.
    ///
    /// For a fuzz test, Forge imports frontiers from `fuzz.frontier_dir`, replays each recorded
    /// one-call seed as a path-priority hint, constrains symbolic execution to flip the captured
    /// comparison, and writes to `fuzz.corpus_dir` only candidates that replay with the expected
    /// concrete outcome. For an invariant test, it imports frontiers from
    /// `invariant.frontier_dir`, replays each recorded call prefix, symbolically solves only the
    /// call that reached the comparison, and writes candidates to `invariant.corpus_dir`. Missing
    /// directories skip the seeding with a warning.
    ///
    /// This is an explicit follow-up to a concrete campaign that captured frontiers, not symbolic
    /// work in every fuzz run. Keep `timeout` short to bound iteration, and use `frontier_ids`,
    /// `frontier_pcs`, `frontier_selectors` or a lower `frontier_limit` to spend solver time only
    /// on selected sites.
    pub use_fuzz_frontiers: bool,
    /// Whether invariant frontier seeding first checks if a selected call can break an invariant.
    ///
    /// Enabling this also enables invariant frontier seeding, as `use_fuzz_frontiers` does for
    /// invariant tests. For each selected frontier, Forge replays the concrete call prefix,
    /// symbolically executes the target call, and checks the configured invariant functions from
    /// the resulting state, before separately trying to flip the recorded branch. Without
    /// `afterInvariant`, each invariant function is checked independently; with it, the check
    /// uses the campaign's anchor invariant and hook. Only candidates that reproduce the failure
    /// during concrete replay are persisted. Like invariant frontier seeding, it needs
    /// `invariant.frontier_dir` and `invariant.corpus_dir`.
    ///
    /// An incomplete symbolic search is not a proof that the invariant holds.
    ///
    /// ```toml
    /// [profile.default.symbolic]
    /// use_fuzz_frontiers = true
    /// check_invariant_frontiers = true
    /// ```
    #[serde(default)]
    pub check_invariant_frontiers: bool,
    /// Whether one bounded symbolic solver worker assists a running invariant campaign.
    ///
    /// The worker takes comparisons observed by the first concrete fuzz worker and tries to flip
    /// their branches from the replayed call prefix. The concrete campaign executes the sequences
    /// it returns from setup, with its normal predicate checks, failure reporting, shrinking and
    /// corpus persistence, and they can expose new frontiers during the same campaign. This does
    /// not require `enabled` (the `--symbolic-live` flag also enables it).
    ///
    /// Budgets apply per campaign: `timeout` (30 seconds when unset) limits when new searches
    /// start, while `max_solver_queries` and `frontier_limit` bound solver queries and submitted
    /// frontiers. A zero timeout, query budget or frontier limit disables it. Each search has its
    /// own one-second solver timeout, and an in-flight search can finish after the deadline.
    ///
    /// The worker keeps a backlog of up to 32 frontiers, preferring shorter prefixes; admission
    /// is lossy. At most 1024 calls and 64 KiB of calldata are retained per sequence; longer
    /// sequences are not used. It adds one solver thread to the configured fuzz workers, and
    /// short campaigns can finish before candidates arrive.
    ///
    /// It does not support optimization campaigns or `invariant.call_override`, and solves only
    /// setup-known targets with zero transaction value, keeping the concrete sender. Unsupported
    /// operations and exhausted budgets leave the concrete campaign running. Frontier filters do
    /// not apply. Only the direct solver process is terminated on shutdown, so wrapper commands
    /// must stop any subprocesses they start.
    ///
    /// Live assistance does not prove safety. JSON output reports worker counters under
    /// `symbolic_live`, where `scheduled` counts sequences handed to the campaign, not coverage
    /// gains.
    ///
    /// ```toml
    /// [profile.default.symbolic]
    /// live = true
    /// timeout = 30
    /// max_solver_queries = 10000
    /// frontier_limit = 256
    /// ```
    #[serde(default)]
    pub live: bool,
    /// Maximum number of branch frontiers to import per symbolic run, or to submit per live
    /// campaign.
    ///
    /// Applies to `use_fuzz_frontiers`, `check_invariant_frontiers` and `live`. `0` imports none.
    pub frontier_limit: usize,
    /// Fuzz branch frontier artifact IDs to import.
    ///
    /// Empty means any ID. Non-empty frontier filters combine, so a frontier is imported only if
    /// it matches every non-empty filter (`frontier_ids`, `frontier_pcs`, `frontier_selectors`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frontier_ids: Vec<u64>,
    /// Program counters of fuzz branch frontier comparisons to import.
    ///
    /// Empty means any program counter. Combines with the other frontier filters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frontier_pcs: Vec<usize>,
    /// Calldata selectors of fuzz branch frontiers to import.
    ///
    /// Each entry is a 4-byte hex selector, with or without `0x`, such as `"0x12345678"`; invalid
    /// entries are ignored with a warning. Empty means any selector. Combines with the other
    /// frontier filters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frontier_selectors: Vec<String>,
    /// SMT solver to invoke for symbolic tests.
    ///
    /// Known names are `z3`, `yices`, `cvc5`, `cvc5-int`, `bitwuzla` and `bitwuzla-abs`. Any other
    /// value is treated as the path or name of a z3-compatible executable and invoked with
    /// `-in -smt2`. Use `solver_command` for other command lines or wrapper tools. A nonempty
    /// `solver_command` or `solver_portfolio` takes precedence.
    ///
    /// Custom solver values run local programs when symbolic tests run, including values from
    /// inline `forge-config` or `@custom:halmos` annotations: review solver settings before
    /// running symbolic tests from an untrusted project.
    pub solver: String,
    /// Exact solver command line to invoke, overriding `solver` and `solver_portfolio`.
    ///
    /// The command, such as `"z3 -in -smt2"`, is split into arguments and run directly, not
    /// through a shell. Timeouts terminate only the direct solver process, so wrapper commands
    /// must stop any subprocesses they start. Like `solver`, it runs a local program.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solver_command: Option<String>,
    /// Solver names or commands to race in parallel.
    ///
    /// Solvers start in the listed order with staged starts: the first immediately, the second
    /// shortly after if the query is still unresolved, and later ones as rescue solvers. The first
    /// validated `sat` result wins; `unsat` is used only once every solver needed to rule out
    /// `sat` has finished. A nonempty portfolio overrides `solver` and is ignored when
    /// `solver_command` is set. Entries without whitespace, quotes or backslashes are resolved
    /// like `solver`; others are split into arguments like `solver_command`, never run through a
    /// shell.
    ///
    /// Start with a small portfolio such as `["yices", "z3"]`: broader ones can use more CPU and
    /// be slower when one solver already handles most queries. `dump_smt` prints each solver's
    /// order, launch delay and per-query outcome for comparison.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub solver_portfolio: Vec<String>,
    /// Solver timeout in seconds.
    ///
    /// It bounds each SMT query, and also the deferred hard-arithmetic solver phase of stateless
    /// tests, frontier solving, and the whole wall-clock exploration of a symbolic invariant test
    /// before it falls back to fuzzing. A timeout makes the result incomplete. `0` removes these
    /// limits, except for `live`, where it disables assistance.
    pub timeout: Option<u32>,
    /// Halmos-compatible bound on loop iterations per path.
    ///
    /// Each backward jump, counted per loop and calling context, is taken at most this many times
    /// on a path; paths that would iterate further are dropped. A `PASS` then covers only the
    /// explored iterations, and skipped iterations are not necessarily reported as incomplete.
    /// When unset, loops are bounded only by `max_depth`.
    #[serde(default, rename = "loop", skip_serializing_if = "Option::is_none")]
    pub loop_bound: Option<u32>,
    /// Halmos-compatible alias for `max_depth`, which it overrides when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
    /// Halmos-compatible alias for `max_paths`, which it overrides when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Maximum number of opcodes to execute along one symbolic path.
    ///
    /// Reaching it makes the result incomplete. `depth` overrides it when set.
    pub max_depth: u32,
    /// Maximum number of symbolic paths to explore per test.
    ///
    /// It also caps how many calldata shapes dynamic length sets expand into. Reaching it makes
    /// the result incomplete. `width` overrides it when set.
    pub max_paths: u32,
    /// Maximum number of calls in a bounded symbolic invariant sequence.
    ///
    /// A `PASS` covers only sequences up to this length; deeper sequences are not explored and
    /// not reported as incomplete.
    pub invariant_depth: u32,
    /// Order in which pending symbolic paths are explored.
    ///
    /// It only changes which queued path runs next, not path or solver limits or portfolio
    /// scheduling.
    ///
    /// - `bfs`: breadth-first, oldest pending path first.
    /// - `dfs`: depth-first, newest pending path first.
    #[serde(default)]
    pub exploration_order: SymbolicExplorationOrder,
    /// Maximum number of solver queries per symbolic test.
    ///
    /// Reaching it makes the result incomplete. For `live`, it is the budget for the whole
    /// campaign.
    pub max_solver_queries: u32,
    /// Length used for a dynamic ABI input when no other length setting applies.
    ///
    /// Dynamic inputs (`bytes`, `string` and dynamic arrays) take one of a bounded set of lengths.
    /// Lengths resolve in this order: `dynamic_lengths`, then `default_array_lengths` or
    /// `default_bytes_lengths`, then `array_lengths`, then this value.
    pub default_dynamic_length: u32,
    /// Maximum length allowed for a symbolic dynamic ABI input.
    ///
    /// A configured length above it is a configuration error.
    pub max_dynamic_length: u32,
    /// Positional lengths for dynamic ABI inputs, applied in ABI traversal order.
    ///
    /// Each entry is the length of the next dynamic leaf, in deterministic ABI pre-order, that
    /// `dynamic_lengths`, `default_array_lengths` and `default_bytes_lengths` did not set. Extra
    /// entries that no dynamic leaf uses are a configuration error.
    ///
    /// ```solidity
    /// /// forge-config: default.symbolic.array_lengths = [3]
    /// function check_bytes(bytes memory data) external pure {}
    /// ```
    pub array_lengths: Vec<u32>,
    /// Sets of lengths for dynamic ABI inputs, keyed by ABI argument name or generated symbolic
    /// name.
    ///
    /// Keys are argument names or generated names such as `calldata_0`. Each value is a set of
    /// lengths, such as `{ data = [3] }`; every length becomes a separate calldata shape, and
    /// nested dynamic values explore the cross product. These lengths take precedence over every
    /// other length setting.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dynamic_lengths: BTreeMap<String, Vec<u32>>,
    /// Set of lengths for dynamic ABI arrays without a `dynamic_lengths` entry.
    ///
    /// Each length becomes a separate calldata shape. When empty, `array_lengths` and then
    /// `default_dynamic_length` apply.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_array_lengths: Vec<u32>,
    /// Set of lengths for ABI `bytes` and `string` values without a `dynamic_lengths` entry.
    ///
    /// Each length becomes a separate calldata shape. When empty, `array_lengths` and then
    /// `default_dynamic_length` apply.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_bytes_lengths: Vec<u32>,
    /// Maximum size of symbolic calldata, in bytes.
    ///
    /// Generated calldata larger than this, and symbolic-size data copies above it during
    /// execution, are unsupported and make the result incomplete.
    pub max_calldata_bytes: u32,
    /// Whether a symbolic call target may be an address other than a known contract.
    ///
    /// Concrete call targets, and symbolic targets constrained to known deployed contracts or
    /// active precompiles, are always supported. By default, a symbolic target that may be any
    /// other address makes the result incomplete. When enabled, that case is modeled as a
    /// successful call to an empty account with no code, including the value transfer for `CALL`;
    /// arbitrary unknown code is still not modeled.
    pub symbolic_call_targets: bool,
    /// Whether to print each SMT-LIB query to standard error before invoking the solver.
    ///
    /// With a `solver_portfolio`, it also prints each solver's order, launch delay and per-query
    /// outcome.
    pub dump_smt: bool,
    /// How symbolic tests model storage they have not written.
    ///
    /// - `solidity`: reads at concrete slots use the existing state; a read of an unwritten slot
    ///   at a symbolic key returns any slot the account already has, such as those `setUp` wrote,
    ///   or else an unknown value.
    /// - `generic`: every storage read may return an arbitrary value, as if
    ///   `vm.setArbitraryStorage` applied to every account.
    /// - `zero_init`: like `solidity`, but a read of an unwritten slot at a symbolic key returns
    ///   zero. Use it for aggregate ghost variables that start from zero.
    ///
    /// Proofs that involve unknown existing storage hold only under the selected layout.
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
            check_invariant_frontiers: false,
            live: false,
            frontier_limit: 256,
            frontier_ids: Vec::new(),
            frontier_pcs: Vec::new(),
            frontier_selectors: Vec::new(),
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
            "frontier_ids": [],
            "frontier_pcs": [],
            "frontier_selectors": [],
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
        assert!(!config.live);
    }
}
