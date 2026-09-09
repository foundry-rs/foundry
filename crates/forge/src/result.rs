//! Test outcomes.

use crate::{
    fuzz::{BaseCounterExample, BasicTxDetails},
    gas_report::GasReport,
};
use alloy_primitives::{
    Address, B256, Bytes, I256, Log, Selector, U256,
    map::{AddressHashMap, HashMap},
};
use eyre::Report;
use foundry_common::{ContractsByArtifact, get_contract_name, shell};
use foundry_config::{SymbolicConfig, SymbolicExplorationOrder, SymbolicStorageLayout};
use foundry_evm::{
    core::{Breakpoints, evm::FoundryEvmNetwork},
    coverage::HitMaps,
    decode::SkipReason,
    executors::{
        RawCallResult,
        invariant::{CheckSequenceFailureSite, CheckSequenceOutcome, InvariantMetrics},
    },
    fuzz::{
        CallDetails, CounterExample, FuzzCase, FuzzFixtures, FuzzTestResult,
        strategies::EvmFuzzState,
    },
    traces::{CallTraceArena, CallTraceDecoder, TraceKind, Traces},
};
use foundry_evm_symbolic::{
    PortfolioDiagnostics, SymbolicStats, SymbolicStopReason, SymbolicStorageAssignment,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap as Map},
    fmt::{self, Write},
    path::PathBuf,
    sync::OnceLock,
    time::Duration,
};
use yansi::Paint;

const INVARIANT_CAMPAIGN_FALLBACK_NAME: &str = "Invariant campaign";
const SYMBOLIC_RESULT_SCHEMA_VERSION: u32 = 1;
pub const SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA: &str = "foundry:symbolic.counterexample@v1";
pub const SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA_VERSION: u32 = 1;

/// The aggregated result of a test run.
#[derive(Clone, Debug)]
pub struct TestOutcome {
    /// The results of all test suites by their identifier (`path:contract_name`).
    ///
    /// Essentially `identifier => signature => result`.
    pub results: BTreeMap<String, SuiteResult>,
    /// Complete results for JSON file output, including suites hidden from fail-fast console
    /// output.
    pub(crate) json_file_results: Option<BTreeMap<String, SuiteResult>>,
    /// Whether to allow test failures without failing the entire test run.
    pub allow_failure: bool,
    /// The decoder used to decode traces and logs.
    ///
    /// This is `None` if traces and logs were not decoded.
    ///
    /// Note that `Address` fields only contain the last executed test case's data.
    pub last_run_decoder: Option<CallTraceDecoder>,
    /// The gas report, if requested.
    pub gas_report: Option<GasReport>,
    /// Known contracts from the test run (used for coverage).
    pub known_contracts: Option<ContractsByArtifact>,
    /// The fuzz seed used for the test run.
    pub fuzz_seed: Option<U256>,
}

impl TestOutcome {
    /// Creates a new test outcome with the given results.
    pub const fn new(
        known_contracts: Option<ContractsByArtifact>,
        results: BTreeMap<String, SuiteResult>,
        allow_failure: bool,
        fuzz_seed: Option<U256>,
    ) -> Self {
        Self {
            results,
            json_file_results: None,
            allow_failure,
            last_run_decoder: None,
            gas_report: None,
            known_contracts,
            fuzz_seed,
        }
    }

    /// Creates a new empty test outcome.
    pub const fn empty(known_contracts: Option<ContractsByArtifact>, allow_failure: bool) -> Self {
        Self::new(known_contracts, BTreeMap::new(), allow_failure, None)
    }

    /// Returns an iterator over all individual succeeding tests and their names.
    pub fn successes(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.tests().filter(|(_, t)| t.status.is_success())
    }

    /// Returns an iterator over all individual skipped tests and their names.
    pub fn skips(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.tests().filter(|(_, t)| t.status.is_skipped())
    }

    /// Returns an iterator over all individual failing tests and their names.
    pub fn failures(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.tests().filter(|(_, t)| t.status.is_failure())
    }

    /// Returns an iterator over all individual tests and their names.
    pub fn tests(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.results.values().flat_map(|suite| suite.tests())
    }

    /// Flattens the test outcome into a list of individual tests.
    pub fn into_tests(self) -> impl Iterator<Item = SuiteTestResult> {
        self.results.into_iter().flat_map(|(artifact_id, suite)| {
            suite.test_results.into_iter().map(move |(signature, result)| SuiteTestResult {
                artifact_id: artifact_id.clone(),
                signature,
                result,
            })
        })
    }

    /// Returns the number of tests that passed.
    pub fn passed(&self) -> usize {
        self.results.values().map(SuiteResult::passed).sum()
    }

    /// Returns the number of tests that were skipped.
    pub fn skipped(&self) -> usize {
        self.results.values().map(SuiteResult::skipped).sum()
    }

    /// Returns the number of tests that failed.
    pub fn failed(&self) -> usize {
        self.results.values().map(SuiteResult::failed).sum()
    }

    /// Returns `true` if any fuzz or invariant test failed.
    pub fn has_fuzz_failures(&self) -> bool {
        self.failures().any(|(_, t)| t.kind.is_fuzz() || t.kind.is_invariant())
    }

    /// Returns `true` if all failing tests can be meaningfully inspected with `forge test --debug`.
    fn failed_tests_are_debuggable(&self) -> bool {
        self.failures().all(|(_, result)| result.is_debuggable_failure())
    }

    /// Returns the shared parallel worker count of all failing invariant tests, if they agree.
    fn invariant_workers_hint(&self) -> Option<usize> {
        let mut workers = self.failures().filter_map(|(_, result)| result.kind.invariant_workers());
        let first = workers.next()?;
        (first > 1 && workers.all(|workers| workers == first)).then_some(first)
    }

    /// Sums up all the durations of all individual test suites.
    ///
    /// Note that this is not necessarily the wall clock time of the entire test run.
    pub fn total_time(&self) -> Duration {
        self.results.values().map(|suite| suite.duration).sum()
    }

    /// Formats the aggregated summary of all test suites into a string (for printing).
    pub fn summary(&self, wall_clock_time: Duration) -> String {
        let num_test_suites = self.results.len();
        let suites = if num_test_suites == 1 { "suite" } else { "suites" };
        let (passed, failed, skipped) = (self.passed(), self.failed(), self.skipped());
        format!(
            "\nRan {num_test_suites} test {suites} in {wall_clock_time:.2?} ({:.2?} CPU time): {} tests passed, {} failed, {} skipped ({} total tests)",
            self.total_time(),
            passed.green(),
            failed.red(),
            skipped.yellow(),
            passed + failed + skipped
        )
    }

    /// Checks if there are any failures and failures are disallowed.
    pub fn ensure_ok(&self, silent: bool) -> eyre::Result<()> {
        let failures = self.failures().count();
        if self.allow_failure || failures == 0 {
            return Ok(());
        }

        if shell::is_quiet() || silent {
            std::process::exit(1);
        }

        sh_println!("\nFailing tests:")?;
        for (suite_name, suite) in &self.results {
            let failed = suite.failed();
            if failed == 0 {
                continue;
            }

            let term = if failed > 1 { "tests" } else { "test" };
            sh_println!("Encountered {failed} failing {term} in {suite_name}")?;
            for (name, result) in suite.failures() {
                sh_println!("{}", result.short_result_with_suite(name, suite_name))?;
            }
            sh_println!()?;
        }
        sh_println!(
            "Encountered a total of {} failing tests, {} tests succeeded",
            failures.to_string().red(),
            self.passed().to_string().green()
        )?;

        let test_word = if failures == 1 { "test" } else { "tests" };
        sh_println!(
            "\nTip: Run {} to retry only the {failures} failed {test_word}",
            "`forge test --rerun`".cyan()
        )?;
        if self.failed_tests_are_debuggable() {
            sh_println!(
                "Tip: Run {} to inspect one failing test in the debugger",
                "`forge test --debug --match-test <TEST_NAME>`".cyan()
            )?;
        }

        // Print seed for fuzz/invariant test failures to enable reproduction.
        if let Some(seed) = self.fuzz_seed
            && self.has_fuzz_failures()
        {
            sh_println!(
                "\nFuzz seed: {} (use {} to reproduce)",
                format!("{seed:#x}").cyan(),
                "`--fuzz-seed`".cyan()
            )?;
            if let Some(invariant_workers) = self.invariant_workers_hint() {
                sh_println!(
                    "Invariant workers: {invariant_workers} (use {} to reproduce)",
                    format!("`--invariant-workers {invariant_workers}`").cyan()
                )?;
            }
        }

        std::process::exit(1);
    }

    /// Removes first test result, if any.
    pub fn remove_first(&mut self) -> Option<(String, String, TestResult)> {
        self.results.iter_mut().find_map(|(suite_name, suite)| {
            let (test_name, result) = suite.test_results.pop_first()?;
            Some((suite_name.clone(), test_name, result))
        })
    }
}

/// A set of test results for a single test suite, which is all the tests in a single contract.
#[derive(Clone, Debug, Serialize)]
pub struct SuiteResult {
    /// Wall clock time it took to execute all tests in this suite.
    #[serde(with = "foundry_common::serde_helpers::duration")]
    pub duration: Duration,
    /// Individual test results: `test fn signature -> TestResult`.
    pub test_results: BTreeMap<String, TestResult>,
    /// Generated warnings.
    pub warnings: Vec<String>,
}

impl SuiteResult {
    pub fn new(
        duration: Duration,
        test_results: BTreeMap<String, TestResult>,
        mut warnings: Vec<String>,
    ) -> Self {
        // Add deprecated cheatcodes warning, if any of them used in current test suite.
        let deprecated_cheatcodes = test_results
            .values()
            .flat_map(|result| result.deprecated_cheatcodes.iter().map(|(k, v)| (*k, *v)))
            .collect::<HashMap<_, _>>();
        if !deprecated_cheatcodes.is_empty() {
            let mut warning =
                "the following cheatcode(s) are deprecated and will be removed in future versions:"
                    .to_string();
            for (cheatcode, reason) in deprecated_cheatcodes {
                write!(warning, "\n  {cheatcode}").unwrap();
                if let Some(reason) = reason {
                    write!(warning, ": {reason}").unwrap();
                }
            }
            warnings.push(warning);
        }

        Self { duration, test_results, warnings }
    }

    /// Returns an iterator over all individual succeeding tests and their names.
    pub fn successes(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.tests().filter(|(_, t)| t.status.is_success())
    }

    /// Returns an iterator over all individual skipped tests and their names.
    pub fn skips(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.tests().filter(|(_, t)| t.status.is_skipped())
    }

    /// Returns an iterator over all individual failing tests and their names.
    pub fn failures(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.tests().filter(|(_, t)| t.status.is_failure())
    }

    /// Returns the number of tests that passed.
    pub fn passed(&self) -> usize {
        self.test_results.values().filter(|t| t.status.is_success()).count()
    }

    /// Returns the number of tests that were skipped.
    pub fn skipped(&self) -> usize {
        self.test_results.values().map(TestResult::skipped_count).sum()
    }

    /// Returns the number of tests that failed.
    pub fn failed(&self) -> usize {
        self.test_results.values().filter(|t| t.status.is_failure()).count()
    }

    /// Iterator over all tests and their names
    pub fn tests(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.test_results.iter()
    }

    /// Whether this test suite is empty.
    pub fn is_empty(&self) -> bool {
        self.test_results.is_empty()
    }

    /// The number of tests in this test suite.
    pub fn len(&self) -> usize {
        self.test_results.values().map(TestResult::logical_count).sum()
    }

    /// Sums up all the durations of all individual tests in this suite.
    ///
    /// Note that this is not necessarily the wall clock time of the entire test suite.
    pub fn total_time(&self) -> Duration {
        self.test_results.values().map(|result| result.duration).sum()
    }

    /// Returns the summary of a single test suite.
    pub fn summary(&self) -> String {
        let failed = self.failed();
        let result = if failed == 0 { "ok".green() } else { "FAILED".red() };
        format!(
            "Suite result: {result}. {} passed; {} failed; {} skipped; finished in {:.2?} ({:.2?} CPU time)",
            self.passed().green(),
            failed.red(),
            self.skipped().yellow(),
            self.duration,
            self.total_time(),
        )
    }
}

/// The result of a single test in a test suite.
///
/// This is flattened from a [`TestOutcome`].
#[derive(Clone, Debug)]
pub struct SuiteTestResult {
    /// The identifier of the artifact/contract in the form:
    /// `<artifact file name>:<contract name>`.
    pub artifact_id: String,
    /// The function signature of the Solidity test.
    pub signature: String,
    /// The result of the executed test.
    pub result: TestResult,
}

impl SuiteTestResult {
    /// Returns the gas used by the test.
    pub const fn gas_used(&self) -> u64 {
        self.result.kind.report().gas()
    }

    /// Returns the contract name of the artifact ID.
    pub fn contract_name(&self) -> &str {
        get_contract_name(&self.artifact_id)
    }
}

/// The status of a test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestStatus {
    Success,
    #[default]
    Failure,
    Skipped,
}

impl TestStatus {
    /// Returns `true` if the test was successful.
    #[inline]
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Success)
    }

    /// Returns `true` if the test failed.
    #[inline]
    pub const fn is_failure(self) -> bool {
        matches!(self, Self::Failure)
    }

    /// Returns `true` if the test was skipped.
    #[inline]
    pub const fn is_skipped(self) -> bool {
        matches!(self, Self::Skipped)
    }
}

/// A failure surfaced by an invariant test campaign — either a broken `invariant_*`
/// predicate ([`Self::Predicate`]) or a handler-side assertion bug ([`Self::Handler`]).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InvariantFailure {
    /// A broken `invariant_*` predicate.
    Predicate {
        /// Invariant function name (e.g. `invariant_cond3`).
        name: String,
        /// Revert reason or assertion failure message.
        reason: String,
        /// Counterexample sequence, when one is available.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        counterexample: Option<CounterExample>,
        /// Durable replay artifact for this counterexample, when one was written.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact: Option<SymbolicArtifactRef>,
        /// Deterministic concrete minimization details for this sequence, when minimized.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        minimization: Option<SymbolicCounterexampleMinimization>,
        /// Path where the counterexample was persisted for re-running and shrinking.
        persisted_path: PathBuf,
        /// Whether this failure is the stable campaign anchor.
        /// When `true` and this is the only single-predicate failure, the function name is
        /// omitted on the `[FAIL: ...]` line (the trailing summary already identifies it).
        #[serde(default)]
        is_anchor: bool,
    },
    /// A handler-side assertion bug discovered during the campaign.
    Handler {
        /// Best-effort human-readable name of the failing call, e.g. `Counter::increment` or
        /// `0xabc...::0x12345678` when the contract/function cannot be resolved.
        name: String,
        /// Address of the handler whose call asserted/reverted with an assertion.
        reverter: Address,
        /// 4-byte selector of the failing handler function.
        selector: Selector,
        /// Decoded revert/assert reason.
        reason: String,
        /// Counterexample sequence leading up to (and including) the failing call.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        counterexample: Option<CounterExample>,
        /// Durable replay artifact for this counterexample, when one was written.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact: Option<SymbolicArtifactRef>,
    },
}

impl InvariantFailure {
    /// Reason rendered on the `[FAIL: ...]` line.
    pub fn reason(&self) -> &str {
        match self {
            Self::Predicate { reason, .. } | Self::Handler { reason, .. } => reason,
        }
    }

    /// Human-readable name (invariant fn name, or `Contract::function` for handler bugs).
    pub fn name(&self) -> &str {
        match self {
            Self::Predicate { name, .. } | Self::Handler { name, .. } => name,
        }
    }

    /// Invariant predicate name, if this is a predicate failure.
    pub fn predicate_name(&self) -> Option<&str> {
        match self {
            Self::Predicate { name, .. } => Some(name),
            Self::Handler { .. } => None,
        }
    }

    /// Counterexample sequence, when one is available.
    pub const fn counterexample(&self) -> Option<&CounterExample> {
        match self {
            Self::Predicate { counterexample, .. } | Self::Handler { counterexample, .. } => {
                counterexample.as_ref()
            }
        }
    }

    /// Durable replay artifact for this failure, when one was written.
    pub const fn artifact(&self) -> Option<&SymbolicArtifactRef> {
        match self {
            Self::Predicate { artifact, .. } | Self::Handler { artifact, .. } => artifact.as_ref(),
        }
    }

    /// Deterministic concrete minimization details for predicate failures.
    pub const fn minimization(&self) -> Option<&SymbolicCounterexampleMinimization> {
        match self {
            Self::Predicate { minimization, .. } => minimization.as_ref(),
            Self::Handler { .. } => None,
        }
    }
}

/// Pass/fail status for an invariant predicate evaluated inside a contract-level campaign.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvariantPredicateResult {
    /// Invariant function name (e.g. `invariant_balance`).
    pub name: String,
    /// Predicate status within the logical campaign.
    pub status: TestStatus,
    /// Revert reason or assertion message when the predicate failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Stable machine-readable outcome for `forge test --symbolic` JSON output.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolicResult {
    /// Schema version for the symbolic result object.
    #[serde(default = "symbolic_result_schema_version")]
    pub schema_version: u32,
    /// Normalized symbolic outcome.
    pub status: SymbolicResultStatus,
    /// Incomplete reason when [`Self::status`] is [`SymbolicResultStatus::Incomplete`].
    pub incomplete: Option<SymbolicIncomplete>,
    /// Effective bounds used by this symbolic run.
    pub bounds: SymbolicBounds,
    /// Solver identity and counters collected during this run.
    pub solver: SymbolicSolverMetadata,
    /// Soundness assumptions that bound what a `pass` proves.
    pub assumptions: Vec<SymbolicAssumption>,
    /// Where an agent can find the concrete replay trace, when one was produced.
    pub call_trace: SymbolicCallTrace,
    /// Concrete replay metadata for counterexample candidates.
    pub replay: SymbolicReplayMetadata,
    /// Concrete counterexample data, when the solver produced a candidate.
    pub counterexample: Option<SymbolicCounterexample>,
    /// Fuzz corpus seeds imported into symbolic execution, when enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corpus_seeds: Option<SymbolicCorpusSeedMetadata>,
    /// Durable counterexample artifact, when one was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<SymbolicArtifactRef>,
    /// Deterministic concrete minimization details, when a replayed counterexample was minimized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimization: Option<SymbolicCounterexampleMinimization>,
}

impl SymbolicResult {
    /// Creates a symbolic pass result.
    pub fn pass(config: &SymbolicConfig, stats: SymbolicStats) -> Self {
        Self::base(config, stats)
    }

    /// Creates a symbolic counterexample result that concrete replay confirmed.
    pub fn fail_counterexample(
        config: &SymbolicConfig,
        stats: SymbolicStats,
        call_trace: SymbolicCallTrace,
        counterexample: SymbolicCounterexample,
    ) -> Self {
        Self {
            counterexample: Some(counterexample),
            ..Self::fail_counterexample_sequence(config, stats, call_trace)
        }
    }

    /// Creates a symbolic sequence counterexample result that concrete replay confirmed.
    pub fn fail_counterexample_sequence(
        config: &SymbolicConfig,
        stats: SymbolicStats,
        call_trace: SymbolicCallTrace,
    ) -> Self {
        Self {
            status: SymbolicResultStatus::FailCounterexample,
            replay: SymbolicReplayMetadata::confirmed(),
            call_trace,
            ..Self::base(config, stats)
        }
    }

    /// Creates an incomplete symbolic result.
    pub fn incomplete(
        config: &SymbolicConfig,
        kind: SymbolicStopReason,
        reason: impl Into<String>,
        stats: SymbolicStats,
        replay: SymbolicReplayMetadata,
        call_trace: SymbolicCallTrace,
        counterexample: Option<SymbolicCounterexample>,
    ) -> Self {
        Self {
            status: SymbolicResultStatus::Incomplete,
            incomplete: Some(SymbolicIncomplete::new(kind, reason)),
            replay,
            call_trace,
            counterexample,
            ..Self::base(config, stats)
        }
    }

    /// A passing result carrying the run's bounds, solver metadata and assumptions.
    fn base(config: &SymbolicConfig, stats: SymbolicStats) -> Self {
        Self {
            schema_version: SYMBOLIC_RESULT_SCHEMA_VERSION,
            status: SymbolicResultStatus::Pass,
            incomplete: None,
            bounds: SymbolicBounds::from_config(config),
            solver: SymbolicSolverMetadata {
                name: config.solver.clone(),
                command: config.solver_command.clone(),
                portfolio: config.solver_portfolio.clone(),
                stats,
            },
            assumptions: SymbolicAssumption::default_assumptions(),
            call_trace: SymbolicCallTrace::none(),
            replay: SymbolicReplayMetadata::not_required(),
            counterexample: None,
            corpus_seeds: None,
            artifact: None,
            minimization: None,
        }
    }

    /// Attaches fuzz corpus import metadata to this symbolic result.
    pub fn with_corpus_seeds(mut self, corpus_seeds: SymbolicCorpusSeedMetadata) -> Self {
        self.corpus_seeds = Some(corpus_seeds);
        self
    }

    /// Attaches a durable replay artifact reference to this symbolic result.
    pub fn with_artifact(mut self, artifact: SymbolicArtifactRef) -> Self {
        self.artifact = Some(artifact);
        self
    }

    /// Attaches deterministic minimization metadata to this symbolic result.
    pub fn with_minimization(mut self, minimization: SymbolicCounterexampleMinimization) -> Self {
        self.minimization = Some(minimization);
        self
    }
}

/// Fuzz corpus import metadata for a symbolic run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolicCorpusSeedMetadata {
    /// Corpus root used for the current test, after contract/test path expansion.
    pub corpus_dir: Option<PathBuf>,
    /// Maximum imported seeds allowed by configuration.
    pub limit: usize,
    /// Number of corpus files considered.
    pub loaded: usize,
    /// Number of corpus files skipped because they were unreadable or not a matching single call.
    pub skipped: usize,
    /// Seeds modeled by symbolic execution as path-priority hints.
    pub used: Vec<SymbolicCorpusSeedRef>,
}

/// One fuzz corpus seed modeled by symbolic execution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolicCorpusSeedRef {
    /// Corpus file path.
    pub path: PathBuf,
    /// ABI-encoded calldata imported from the corpus file.
    pub calldata: Bytes,
}

/// Reference to a durable symbolic counterexample artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolicArtifactRef {
    /// Artifact schema id.
    pub schema: String,
    /// Path to the artifact file.
    pub path: PathBuf,
}

impl SymbolicArtifactRef {
    /// Creates a reference to a symbolic counterexample artifact.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { schema: SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA.to_string(), path: path.into() }
    }
}

/// Reference to a generated Solidity regression test for a symbolic counterexample.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolicRegressionRef {
    /// Source counterexample artifact path.
    pub artifact: PathBuf,
    /// Generated Solidity regression test path.
    pub path: PathBuf,
}

/// Before/after artifact references and counters for concrete symbolic counterexample minimization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolicCounterexampleMinimization {
    /// Original confirmed replay artifact before minimization.
    pub original: SymbolicArtifactRef,
    /// Minimized confirmed replay artifact after minimization.
    pub minimized: SymbolicArtifactRef,
    /// Number of concrete replay candidates tried.
    pub attempts: usize,
    /// Number of replay candidates accepted.
    pub accepted: usize,
    /// ABI calldata byte length before minimization.
    pub original_calldata_bytes: usize,
    /// ABI calldata byte length after minimization.
    pub minimized_calldata_bytes: usize,
    /// Stateful sequence length before minimization, when this minimized a sequence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_sequence_len: Option<usize>,
    /// Stateful sequence length after minimization, when this minimized a sequence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimized_sequence_len: Option<usize>,
}

impl SymbolicCounterexampleMinimization {
    /// Creates concrete minimization metadata.
    pub const fn new(
        original: SymbolicArtifactRef,
        minimized: SymbolicArtifactRef,
        attempts: usize,
        accepted: usize,
        original_calldata_bytes: usize,
        minimized_calldata_bytes: usize,
    ) -> Self {
        Self {
            original,
            minimized,
            attempts,
            accepted,
            original_calldata_bytes,
            minimized_calldata_bytes,
            original_sequence_len: None,
            minimized_sequence_len: None,
        }
    }

    /// Adds stateful sequence lengths to minimization metadata.
    pub const fn with_sequence_lengths(
        mut self,
        original_sequence_len: usize,
        minimized_sequence_len: usize,
    ) -> Self {
        self.original_sequence_len = Some(original_sequence_len);
        self.minimized_sequence_len = Some(minimized_sequence_len);
        self
    }
}

/// Normalized symbolic outcome names for agents and other JSON consumers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolicResultStatus {
    /// All explored paths completed without a feasible failure.
    Pass,
    /// A solver counterexample was replayed concretely and still failed.
    FailCounterexample,
    /// The engine stopped before a proof or replayed counterexample.
    Incomplete,
}

/// Incomplete symbolic run reason.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolicIncomplete {
    /// Stable reason kind.
    pub kind: String,
    /// Human-readable detail.
    pub reason: String,
}

impl SymbolicIncomplete {
    fn new(kind: SymbolicStopReason, reason: impl Into<String>) -> Self {
        let kind = match kind {
            SymbolicStopReason::Stuck => "stuck",
            SymbolicStopReason::RevertAll => "revert_all",
            SymbolicStopReason::Timeout => "timeout",
            SymbolicStopReason::Error => "error",
        };
        Self { kind: kind.to_string(), reason: reason.into() }
    }
}

/// Effective symbolic exploration bounds used by the run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolicBounds {
    /// Optional solver timeout in seconds.
    pub timeout_seconds: Option<u32>,
    /// Optional loop-unrolling bound.
    pub loop_bound: Option<u32>,
    /// Effective per-path opcode depth limit.
    pub max_depth: u32,
    /// Effective symbolic path width limit.
    pub max_paths: u32,
    /// Maximum calls in a bounded symbolic invariant sequence.
    pub invariant_depth: u32,
    /// Pending path exploration order.
    pub exploration_order: SymbolicExplorationOrder,
    /// Maximum normalized solver queries.
    pub max_solver_queries: u32,
    /// Default bounded length for dynamic ABI inputs.
    pub default_dynamic_length: u32,
    /// Maximum permitted bounded dynamic ABI input length.
    pub max_dynamic_length: u32,
    /// Positional dynamic-leaf bounded lengths.
    pub array_lengths: Vec<u32>,
    /// Named dynamic-leaf bounded lengths.
    pub dynamic_lengths: BTreeMap<String, Vec<u32>>,
    /// Default array lengths when no explicit dynamic length exists.
    pub default_array_lengths: Vec<u32>,
    /// Default bytes/string lengths when no explicit dynamic length exists.
    pub default_bytes_lengths: Vec<u32>,
    /// Maximum generated symbolic calldata size in bytes.
    pub max_calldata_bytes: u32,
    /// Whether symbolic call targets can range over known deployed contracts.
    pub symbolic_call_targets: bool,
    /// Storage modelling mode.
    pub storage_layout: SymbolicStorageLayout,
}

impl SymbolicBounds {
    fn from_config(config: &SymbolicConfig) -> Self {
        Self {
            timeout_seconds: config.timeout,
            loop_bound: config.loop_bound,
            max_depth: config.execution_depth(),
            max_paths: config.path_width(),
            invariant_depth: config.invariant_depth,
            exploration_order: config.exploration_order,
            max_solver_queries: config.max_solver_queries,
            default_dynamic_length: config.default_dynamic_length,
            max_dynamic_length: config.max_dynamic_length,
            array_lengths: config.array_lengths.clone(),
            dynamic_lengths: config.dynamic_lengths.clone(),
            default_array_lengths: config.default_array_lengths.clone(),
            default_bytes_lengths: config.default_bytes_lengths.clone(),
            max_calldata_bytes: config.max_calldata_bytes,
            symbolic_call_targets: config.symbolic_call_targets,
            storage_layout: config.storage_layout,
        }
    }
}

/// Solver identity and counters.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolicSolverMetadata {
    /// Configured solver name.
    pub name: String,
    /// Exact configured solver command, when set.
    pub command: Option<String>,
    /// Configured solver portfolio entries, when any.
    pub portfolio: Vec<String>,
    /// Run counters.
    pub stats: SymbolicStats,
}

/// Explicit symbolic assumption attached to a result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolicAssumption {
    /// Stable assumption kind.
    pub kind: String,
    /// Human-readable detail.
    pub description: String,
}

impl SymbolicAssumption {
    fn default_assumptions() -> Vec<Self> {
        vec![
            Self {
                kind: "bounded_exploration".to_string(),
                description: "Result is scoped to the configured path, depth, solver-query, loop, calldata, and dynamic-length bounds.".to_string(),
            },
            Self {
                kind: "hash_model".to_string(),
                description: "Symbolic Keccak and hash-like precompile reasoning assumes collision and preimage resistance for modeled cases.".to_string(),
            },
        ]
    }
}

/// Concrete replay trace locator.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolicCallTrace {
    /// Whether replay produced a trace that may be present in this test result.
    pub available: bool,
    /// JSON location for the trace when available.
    pub source: Option<String>,
    /// Trace format at the source location.
    pub format: Option<String>,
}

impl SymbolicCallTrace {
    /// No concrete trace was produced.
    pub const fn none() -> Self {
        Self { available: false, source: None, format: None }
    }

    /// A concrete replay trace may be available in the normal test result traces field.
    pub fn test_result_traces(available: bool) -> Self {
        Self {
            available,
            source: available.then(|| "test_result.traces".to_string()),
            format: available.then(|| "foundry_call_trace_arena".to_string()),
        }
    }
}

/// Counterexample replay status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolicReplayStatus {
    /// No replay was required for this result.
    NotRequired,
    /// Concrete replay confirmed the symbolic counterexample.
    Confirmed,
    /// Concrete replay did not reproduce the symbolic counterexample.
    Mismatch,
    /// Concrete replay could not execute because of an error.
    Error,
    /// Concrete replay was skipped by `vm.skip`.
    Skipped,
}

/// Replay metadata for symbolic counterexample candidates.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolicReplayMetadata {
    /// Whether the symbolic outcome required concrete replay.
    pub required: bool,
    /// Stable replay status.
    pub status: SymbolicReplayStatus,
    /// Optional replay detail or mismatch reason.
    pub reason: Option<String>,
}

impl SymbolicReplayMetadata {
    /// No replay was required.
    pub const fn not_required() -> Self {
        Self { required: false, status: SymbolicReplayStatus::NotRequired, reason: None }
    }

    /// Concrete replay confirmed the counterexample.
    pub const fn confirmed() -> Self {
        Self { required: true, status: SymbolicReplayStatus::Confirmed, reason: None }
    }

    /// Concrete replay did not reproduce the symbolic counterexample.
    pub fn mismatch(reason: impl Into<String>) -> Self {
        Self { required: true, status: SymbolicReplayStatus::Mismatch, reason: Some(reason.into()) }
    }

    /// Concrete replay errored before the candidate could be confirmed.
    pub fn error(reason: impl Into<String>) -> Self {
        Self { required: true, status: SymbolicReplayStatus::Error, reason: Some(reason.into()) }
    }

    /// Concrete replay was skipped by the test.
    pub fn skipped(reason: impl Into<String>) -> Self {
        Self { required: true, status: SymbolicReplayStatus::Skipped, reason: Some(reason.into()) }
    }
}

/// Stable symbolic counterexample payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolicCounterexample {
    /// ABI-encoded calldata for replay.
    pub calldata: Bytes,
    /// Pretty-formatted ABI arguments, when decoded.
    pub args: Option<String>,
    /// Raw ABI arguments, when decoded.
    pub raw_args: Option<String>,
    /// Ether value sent with the call, when any.
    pub value: Option<U256>,
}

impl From<&BaseCounterExample> for SymbolicCounterexample {
    fn from(counterexample: &BaseCounterExample) -> Self {
        Self {
            calldata: counterexample.calldata.clone(),
            args: counterexample.args.clone(),
            raw_args: counterexample.raw_args.clone(),
            value: counterexample.value,
        }
    }
}

/// Durable symbolic counterexample artifact.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolicCounterexampleArtifact {
    /// Artifact schema version.
    pub schema_version: u32,
    /// Artifact schema id.
    pub schema: String,
    /// Whether this counterexample is a single test call or a stateful sequence.
    pub kind: SymbolicCounterexampleArtifactKind,
    /// Test identity that produced this counterexample.
    pub test: SymbolicCounterexampleTestIdentity,
    /// Concrete replay metadata for the counterexample candidate.
    pub replay: SymbolicReplayMetadata,
    /// Replay semantics that must remain stable when this artifact is replayed.
    pub replay_semantics: SymbolicCounterexampleReplaySemantics,
    /// Effective bounds used by this symbolic run.
    pub bounds: SymbolicBounds,
    /// Solver identity and counters collected during this run.
    pub solver: SymbolicSolverMetadata,
    /// Soundness assumptions that bound what a `pass` proves.
    pub assumptions: Vec<SymbolicAssumption>,
    /// Where an agent can find the concrete replay trace, when one was produced.
    pub call_trace: SymbolicCallTrace,
    /// Concrete setup-storage assignments required before replaying this artifact.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub storage: Vec<SymbolicStorageAssignment>,
    /// Stateful invariant failure origin, when this sequence came from symbolic invariants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invariant_failure: Option<SymbolicInvariantArtifactFailure>,
    /// Concrete replay calls.
    pub calls: Vec<SymbolicCounterexampleCall>,
}

impl SymbolicCounterexampleArtifact {
    /// Creates a durable symbolic counterexample artifact from a symbolic result and call list.
    pub fn new(
        kind: SymbolicCounterexampleArtifactKind,
        test: SymbolicCounterexampleTestIdentity,
        symbolic: &SymbolicResult,
        replay_semantics: SymbolicCounterexampleReplaySemantics,
        calls: Vec<SymbolicCounterexampleCall>,
    ) -> Self {
        Self {
            schema_version: SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA_VERSION,
            schema: SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA.to_string(),
            kind,
            test,
            replay: symbolic.replay.clone(),
            replay_semantics,
            bounds: symbolic.bounds.clone(),
            solver: symbolic.solver.clone(),
            assumptions: symbolic.assumptions.clone(),
            call_trace: symbolic.call_trace.clone(),
            storage: Vec::new(),
            invariant_failure: None,
            calls,
        }
    }

    /// Attaches setup-storage assignments required for concrete replay.
    pub fn with_storage(mut self, storage: Vec<SymbolicStorageAssignment>) -> Self {
        self.storage = storage;
        self
    }

    /// Attaches stateful invariant failure origin metadata.
    pub fn with_invariant_failure(
        mut self,
        invariant_failure: SymbolicInvariantArtifactFailure,
    ) -> Self {
        self.invariant_failure = Some(invariant_failure);
        self
    }
}

/// Concrete replay semantics captured when a symbolic artifact is confirmed.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolicCounterexampleReplaySemantics {
    /// Whether an invariant sequence replay treats any target-call revert as a failure.
    pub fail_on_revert: bool,
}

/// Symbolic counterexample artifact shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolicCounterexampleArtifactKind {
    /// A single stateless symbolic test call.
    SingleCall,
    /// A stateful sequence of calls.
    Sequence,
}

/// Stateful invariant failure origin for a persisted symbolic sequence artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SymbolicInvariantArtifactFailure {
    /// An invariant predicate failed.
    Predicate {
        /// Invariant function name.
        name: String,
        /// Exact concrete failure site confirmed during replay.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        site: Option<SymbolicInvariantFailureSite>,
    },
    /// A target/handler call asserted before an invariant predicate failed.
    Handler {
        /// Best-effort human-readable handler function name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// Address of the handler whose call asserted.
        reverter: Address,
        /// 4-byte selector of the failing handler call.
        selector: Selector,
        /// Stable edge fingerprint for the failing handler site.
        fingerprint: B256,
    },
}

/// Concrete invariant failure site stored in symbolic replay artifacts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SymbolicInvariantFailureSite {
    /// Target/handler call failed before the invariant predicate.
    SequenceCall { target: Address, selector: Selector, fingerprint: B256 },
    /// Invariant predicate failed.
    Invariant { target: Address, selector: Selector, fingerprint: B256 },
    /// `afterInvariant` hook failed.
    AfterInvariant { target: Address, selector: Selector, fingerprint: B256 },
}

impl From<CheckSequenceFailureSite> for SymbolicInvariantFailureSite {
    fn from(site: CheckSequenceFailureSite) -> Self {
        match site {
            CheckSequenceFailureSite::SequenceCall { target, selector, fingerprint } => {
                Self::SequenceCall { target, selector, fingerprint }
            }
            CheckSequenceFailureSite::Invariant { target, selector, fingerprint } => {
                Self::Invariant { target, selector, fingerprint }
            }
            CheckSequenceFailureSite::AfterInvariant { target, selector, fingerprint } => {
                Self::AfterInvariant { target, selector, fingerprint }
            }
        }
    }
}

/// Test identity for a symbolic counterexample artifact.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolicCounterexampleTestIdentity {
    /// Contract identifier as reported by Forge.
    pub contract: String,
    /// Test function signature.
    pub test: String,
}

/// One concrete call in a symbolic counterexample artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolicCounterexampleCall {
    /// Amount to increase block timestamp before executing the call.
    pub warp: Option<U256>,
    /// Amount to increase block number before executing the call.
    pub roll: Option<U256>,
    /// Sender used for the call.
    pub sender: Address,
    /// Target address called.
    pub target: Address,
    /// ABI-encoded calldata for replay.
    pub calldata: Bytes,
    /// Ether value sent with the call, when any.
    pub value: Option<U256>,
    /// Human-readable contract identifier, when known.
    pub contract_name: Option<String>,
    /// ABI function name, when known.
    pub function_name: Option<String>,
    /// ABI function signature, when known.
    pub signature: Option<String>,
    /// Pretty-formatted ABI arguments, when decoded.
    pub args: Option<String>,
    /// Raw ABI arguments, when decoded.
    pub raw_args: Option<String>,
}

impl SymbolicCounterexampleCall {
    /// Creates an artifact call from Foundry's base counterexample shape.
    pub fn from_base_counterexample(
        counterexample: &BaseCounterExample,
        default_sender: Address,
        default_target: Address,
    ) -> Self {
        Self {
            warp: counterexample.warp,
            roll: counterexample.roll,
            sender: counterexample.sender.unwrap_or(default_sender),
            target: counterexample.addr.unwrap_or(default_target),
            calldata: counterexample.calldata.clone(),
            value: counterexample.value,
            contract_name: counterexample.contract_name.clone(),
            function_name: counterexample.func_name.clone(),
            signature: counterexample.signature.clone(),
            args: counterexample.args.clone(),
            raw_args: counterexample.raw_args.clone(),
        }
    }

    /// Creates Foundry's display counterexample shape from an artifact call.
    pub fn to_base_counterexample(&self) -> BaseCounterExample {
        BaseCounterExample {
            warp: self.warp,
            roll: self.roll,
            sender: Some(self.sender),
            addr: Some(self.target),
            calldata: self.calldata.clone(),
            value: self.value,
            contract_name: self.contract_name.clone(),
            func_name: self.function_name.clone(),
            signature: self.signature.clone(),
            args: self.args.clone(),
            raw_args: self.raw_args.clone(),
            traces: None,
            show_solidity: false,
            fuzz: Default::default(),
        }
    }

    /// Converts an artifact call into Foundry's invariant replay transaction shape.
    pub fn to_basic_tx_details(&self) -> BasicTxDetails {
        BasicTxDetails {
            warp: self.warp,
            roll: self.roll,
            sender: self.sender,
            call_details: CallDetails {
                target: self.target,
                calldata: self.calldata.clone(),
                value: self.value,
            },
        }
    }
}

/// The result of an executed test.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TestResult {
    /// The test status, indicating whether the test case succeeded, failed, or was marked as
    /// skipped. This means that the transaction executed properly, the test was marked as
    /// skipped with vm.skip(), or that there was a revert and that the test was expected to
    /// fail (prefixed with `testFail`)
    pub status: TestStatus,

    /// If there was a revert, this field will be populated. Note that the test can
    /// still be successful (i.e self.success == true) when it's expected to fail.
    pub reason: Option<String>,

    /// The active fork's block number after execution, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_block_number: Option<u64>,

    /// All broken invariant predicates in this campaign in source declaration order.
    ///
    /// For invariant tests, this is the single source of truth used by the renderer.
    /// `reason` and `counterexample` are not populated for invariant tests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invariant_failures: Vec<InvariantFailure>,

    /// Per-predicate outcomes for invariant campaigns. This preserves individual
    /// `invariant_*` / `statefulFuzz*` pass/fail reporting when multiple predicates are checked
    /// by one contract-level campaign.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invariant_predicate_results: Vec<InvariantPredicateResult>,

    /// Directory where invariant failure counterexamples have been persisted (set when one or more
    /// secondary invariant failures were written, so users can locate persisted counterexamples).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invariant_failure_dir: Option<PathBuf>,

    /// Total number of invariant predicates exercised in this campaign. When `Some(n)` the
    /// user-facing report renders a contract-level `<broken>/<n> invariants broken` summary so
    /// users get an at-a-glance health line without counting `[FAIL]` blocks. `None` for
    /// single-predicate campaigns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invariant_count: Option<usize>,

    /// Handler-side assertion bugs found during the campaign, deduped by
    /// `(reverter, selector)` site (Medusa/Echidna semantics). Rendered in a dedicated
    /// `Assertion Tests` section.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invariant_handler_failures: Vec<InvariantFailure>,

    /// Minimal reproduction test case for failing test
    pub counterexample: Option<CounterExample>,

    /// Legacy durable replay artifact for the top-level counterexample, when one was written.
    ///
    /// Prefer [`Self::counterexample_artifacts`] for new consumers; this compatibility field is
    /// maintained by [`Self::add_counterexample_artifact`] for older JSON readers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterexample_artifact: Option<SymbolicArtifactRef>,

    /// All durable replay artifacts produced for this test result, normalized for JSON consumers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub counterexample_artifacts: Vec<SymbolicArtifactRef>,

    /// Generated Solidity regression tests for this symbolic counterexample.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbolic_regressions: Vec<SymbolicRegressionRef>,

    /// Any captured & parsed as strings logs along the test's execution which should
    /// be printed to the user.
    pub logs: Vec<Log>,

    /// The decoded DSTest logging events and Hardhat's `console.log` from [logs](Self::logs).
    /// Used for json output.
    pub decoded_logs: Vec<String>,

    /// What kind of test this was
    pub kind: TestKind,

    /// Stable symbolic result object for `forge test --symbolic --json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbolic: Option<SymbolicResult>,

    /// Traces
    pub traces: Traces,

    /// Runtime bytecodes for contracts seen in debug traces.
    #[serde(skip)]
    pub debug_bytecodes: AddressHashMap<Bytes>,

    /// Additional traces to use for gas report.
    ///
    /// These are cleared after the gas report is analyzed.
    #[serde(skip)]
    pub gas_report_traces: Vec<Vec<CallTraceArena>>,

    /// Raw line coverage info
    #[serde(skip)]
    pub line_coverage: Option<HitMaps>,

    /// Labeled addresses
    #[serde(rename = "labeled_addresses")] // Backwards compatibility.
    pub labels: AddressHashMap<String>,

    #[serde(with = "foundry_common::serde_helpers::duration")]
    pub duration: Duration,

    /// pc breakpoint char map
    pub breakpoints: Breakpoints,

    /// Any captured gas snapshots along the test's execution which should be accumulated.
    pub gas_snapshots: BTreeMap<String, BTreeMap<String, String>>,

    /// Deprecated cheatcodes (mapped to their replacements, if any) used in current test.
    #[serde(skip)]
    pub deprecated_cheatcodes: HashMap<&'static str, Option<&'static str>>,

    /// Staged solver portfolio diagnostics collected during symbolic execution.
    #[serde(skip)]
    pub symbolic_portfolio_diagnostics: Option<PortfolioDiagnostics>,

    /// Verbose symbolic solver diagnostics deferred until test output rendering.
    #[serde(skip)]
    pub symbolic_diagnostics: Option<String>,
}

impl fmt::Display for TestResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render(false, None))
    }
}

/// Appends a `[label] (original: N, shrunk: M)` header followed by one line per call.
fn write_sequence(s: &mut String, label: &str, original: usize, sequence: &[BaseCounterExample]) {
    writeln!(s, "\n\t[{label}] (original: {original}, shrunk: {})", sequence.len()).unwrap();
    for ex in sequence {
        writeln!(s, "{ex}").unwrap();
    }
}

/// Appends `[FAIL: reason]{name_suffix}` plus the counterexample sequence, if any.
///
/// Returns `true` if a sequence (ending in a newline) was written.
fn write_failure(s: &mut String, failure: &InvariantFailure, name_suffix: &str) -> bool {
    write!(s, "[FAIL: {}]{name_suffix}", failure.reason()).unwrap();
    if let Some(CounterExample::Sequence(original, sequence)) = failure.counterexample() {
        write_sequence(s, "Sequence", *original, sequence);
        return true;
    }
    false
}

/// All durable replay artifacts referenced by a counterexample: its own artifact plus the
/// before/after artifacts of its minimization, if any.
fn replay_artifacts<'a>(
    artifact: Option<&'a SymbolicArtifactRef>,
    minimization: Option<&'a SymbolicCounterexampleMinimization>,
) -> impl Iterator<Item = &'a SymbolicArtifactRef> {
    artifact.into_iter().chain(minimization.into_iter().flat_map(|m| [&m.original, &m.minimized]))
}

impl TestResult {
    /// Returns `true` if this failed result can be meaningfully inspected with
    /// `forge test --debug --match-test`.
    const fn is_debuggable_failure(&self) -> bool {
        self.status.is_failure()
            && !self.kind.is_invariant()
            && !self.kind.is_symbolic()
            && self.symbolic.is_none()
    }

    /// Adds a durable replay artifact to the normalized list and legacy top-level field.
    pub fn add_counterexample_artifact(&mut self, artifact: SymbolicArtifactRef) {
        if !self.counterexample_artifacts.contains(&artifact) {
            self.counterexample_artifacts.push(artifact.clone());
        }
        if self.counterexample_artifact.is_none() {
            self.counterexample_artifact = Some(artifact);
        }
    }

    /// Renders the status block, either for the console (`user_facing`) or for JUnit output.
    fn render(&self, user_facing: bool, campaign_name: Option<&str>) -> String {
        let header = if user_facing {
            campaign_name.unwrap_or(INVARIANT_CAMPAIGN_FALLBACK_NAME)
        } else {
            "Predicates"
        };
        let mut s = String::new();
        match self.status {
            TestStatus::Success => {
                s.push_str("[PASS]");
                // For optimization mode, show the best example sequence in green.
                if let Some(CounterExample::Sequence(original, sequence)) = &self.counterexample {
                    write_sequence(&mut s, "Best sequence", *original, sequence);
                }
                self.write_predicates(&mut s, header, true);
                s.green().wrap().to_string()
            }
            TestStatus::Skipped => {
                s.push_str("[SKIP");
                if let Some(reason) = &self.reason {
                    write!(s, ": {reason}").unwrap();
                }
                s.push(']');
                self.write_predicates(&mut s, header, true);
                s.yellow().to_string()
            }
            TestStatus::Failure => {
                let is_invariant_failure = !self.invariant_failures.is_empty()
                    || !self.invariant_handler_failures.is_empty();
                if is_invariant_failure {
                    // Contract-level campaigns identify the broken predicate even when only one
                    // predicate failed. Preserve the compact legacy shape only for the anchor of a
                    // single-predicate run.
                    let named = self.invariant_count.is_some() || self.invariant_failures.len() > 1;
                    for (i, failure) in self.invariant_failures.iter().enumerate() {
                        if i > 0 {
                            s.push('\n');
                        }
                        let is_anchor =
                            matches!(failure, InvariantFailure::Predicate { is_anchor: true, .. });
                        let suffix = if named || !is_anchor {
                            format!(" {}", failure.name())
                        } else {
                            String::new()
                        };
                        write_failure(&mut s, failure, &suffix);
                    }
                } else {
                    // Non-invariant failure (unit / fuzz / DS-style): render from the legacy
                    // `reason` / `counterexample` fields.
                    s.push_str("[FAIL");
                    if let Some(reason) = &self.reason {
                        write!(s, ": {reason}").unwrap();
                    }
                    match &self.counterexample {
                        Some(CounterExample::Single(ex)) => {
                            write!(s, "; counterexample: {ex}]").unwrap();
                        }
                        Some(CounterExample::Sequence(original, sequence)) => {
                            s.push(']');
                            write_sequence(&mut s, "Sequence", *original, sequence);
                        }
                        None => s.push(']'),
                    }
                }

                let broken = self.invariant_failures.len();
                let rollup = match self.invariant_count {
                    Some(total) if total > 1 && is_invariant_failure => {
                        writeln!(s, "\n{header}: {broken}/{total} invariants broken").unwrap();
                        true
                    }
                    _ => false,
                };
                self.write_predicates(&mut s, header, !user_facing || !rollup);
                if broken > 1
                    && let Some(dir) = &self.invariant_failure_dir
                {
                    writeln!(
                        s,
                        "{broken} invariant failure(s) persisted to {} — rerun to shrink",
                        dir.display()
                    )
                    .unwrap();
                }

                if !self.invariant_handler_failures.is_empty() {
                    // Separate the section from anything rendered above it.
                    let preceded = rollup
                        || broken > 0
                        || (user_facing && self.invariant_predicate_results.len() > 1);
                    writeln!(
                        s,
                        "{}{}: {} assertion bug(s) found",
                        if preceded { "\n" } else { "" },
                        if user_facing { "Assertion Tests" } else { "Handler assertions" },
                        self.invariant_handler_failures.len()
                    )
                    .unwrap();
                    for failure in &self.invariant_handler_failures {
                        if !write_failure(&mut s, failure, &format!(" {}", failure.name())) {
                            s.push('\n');
                        }
                    }
                }

                s.red().wrap().to_string()
            }
        }
    }

    /// Appends the per-predicate summary for multi-predicate campaigns.
    fn write_predicates(&self, s: &mut String, header: &str, show_header: bool) {
        if self.invariant_predicate_results.len() <= 1 {
            return;
        }
        if show_header {
            write!(s, "\n{header}:\n").unwrap();
        }
        for predicate in &self.invariant_predicate_results {
            let name = &predicate.name;
            match (predicate.status, &predicate.reason) {
                (TestStatus::Success, _) => writeln!(s, "[PASS] {name}"),
                (TestStatus::Failure, reason) => {
                    writeln!(s, "[FAIL: {}] {name}", reason.as_deref().unwrap_or_default())
                }
                (TestStatus::Skipped, Some(reason)) => writeln!(s, "[SKIP: {reason}] {name}"),
                (TestStatus::Skipped, None) => writeln!(s, "[SKIP] {name}"),
            }
            .unwrap();
        }
    }
}

macro_rules! extend {
    ($a:expr, $b:expr, $trace_kind:expr) => {
        if $b.fork_block_number.is_some() {
            $a.fork_block_number = $b.fork_block_number;
        }
        $a.logs.extend($b.logs);
        $a.labels.extend($b.labels);
        $a.traces.extend($b.traces.map(|traces| ($trace_kind, traces)));
        $a.debug_bytecodes.extend($b.debug_bytecodes);
        $a.merge_coverages($b.line_coverage);
    };
}

/// Forge-side outcome of an invariant campaign, recorded into a [`TestResult`].
#[derive(Default)]
pub struct InvariantOutcome {
    /// Whether every checked invariant held.
    pub success: bool,
    /// Fork block number the campaign ran against, if any.
    pub fork_block_number: Option<u64>,
    /// Broken invariants, each with its shrunk call sequence.
    pub failures: Vec<InvariantFailure>,
    /// Handler assertion failures found while running the campaign.
    pub handler_failures: Vec<InvariantFailure>,
    /// Per-invariant pass/fail/skip rows.
    pub predicate_results: Vec<InvariantPredicateResult>,
    /// Directory the failing sequences were persisted to.
    pub failure_dir: Option<PathBuf>,
    /// Number of invariants checked, when the campaign ran more than one.
    pub invariant_count: Option<usize>,
    /// Best sequence found in optimization mode.
    pub counterexample: Option<CounterExample>,
    /// Traces collected for the gas report.
    pub gas_report_traces: Vec<Vec<CallTraceArena>>,
}

/// Invariant kind for results that did not run a real campaign (setup failures, replays, skips).
pub(crate) fn invariant_kind(runs: usize, calls: usize, reverts: usize) -> TestKind {
    TestKind::Invariant {
        runs,
        calls,
        reverts,
        workers: 1,
        metrics: Default::default(),
        failed_corpus_replays: 0,
        optimization_best_value: None,
    }
}

impl TestResult {
    /// Creates a new test result starting from test setup results.
    pub fn new(setup: &TestSetup) -> Self {
        Self {
            labels: setup.labels.clone(),
            logs: setup.logs.clone(),
            traces: setup.traces.clone(),
            debug_bytecodes: setup.debug_bytecodes.clone(),
            line_coverage: setup.coverage.clone(),
            fork_block_number: setup.fork_block_number,
            ..Default::default()
        }
    }

    /// Creates a failed test result with given reason.
    pub fn fail(reason: String) -> Self {
        Self { status: TestStatus::Failure, reason: Some(reason), ..Default::default() }
    }

    /// Creates a test setup result.
    pub fn setup_result(setup: TestSetup) -> Self {
        Self {
            status: if setup.skipped { TestStatus::Skipped } else { TestStatus::Failure },
            reason: setup.reason,
            logs: setup.logs,
            traces: setup.traces,
            debug_bytecodes: setup.debug_bytecodes,
            line_coverage: setup.coverage,
            labels: setup.labels,
            fork_block_number: setup.fork_block_number,
            ..Default::default()
        }
    }

    /// Returns the skipped result for single test (used in skipped fuzz test too).
    pub fn single_skip(&mut self, reason: SkipReason) {
        self.status = TestStatus::Skipped;
        self.reason = reason.0;
    }

    /// Returns the failed result with reason for single test.
    pub fn single_fail(&mut self, reason: Option<String>) {
        self.status = TestStatus::Failure;
        self.reason = reason;
    }

    /// Returns the result for single test. Merges execution results (logs, labeled addresses,
    /// traces and coverages) in initial setup results.
    pub fn single_result<FEN: FoundryEvmNetwork>(
        &mut self,
        success: bool,
        reason: Option<String>,
        raw_call_result: RawCallResult<FEN>,
    ) {
        self.kind = TestKind::Unit {
            gas: raw_call_result.gas_used.saturating_sub(raw_call_result.stipend),
        };

        extend!(self, raw_call_result, TraceKind::Execution);

        self.status = if success { TestStatus::Success } else { TestStatus::Failure };
        self.reason = reason;
        self.duration = Duration::default();
        self.gas_report_traces = Vec::new();

        if let Some(cheatcodes) = raw_call_result.cheatcodes {
            self.breakpoints = cheatcodes.breakpoints;
            self.gas_snapshots = cheatcodes.gas_snapshots;
            self.deprecated_cheatcodes = cheatcodes.deprecated;
        }
    }

    /// Returns the result for a fuzzed test. Merges fuzz execution results (logs, labeled
    /// addresses, traces and coverages) in initial setup results.
    pub fn fuzz_result(&mut self, mut result: FuzzTestResult) {
        let kind = TestKind::Fuzz {
            median_gas: result.median_gas(false),
            mean_gas: result.mean_gas(false),
            first_case: std::mem::take(&mut result.first_case),
            runs: result.gas_by_case.len(),
            failed_corpus_replays: result.failed_corpus_replays,
        };
        self.campaign_result(kind, result);
    }

    /// Returns the result for a table test. Merges table test execution results (logs, labeled
    /// addresses, traces and coverages) in initial setup results.
    pub fn table_result(&mut self, result: FuzzTestResult) {
        let kind = TestKind::Table {
            median_gas: result.median_gas(false),
            mean_gas: result.mean_gas(false),
            runs: result.gas_by_case.len(),
        };
        self.campaign_result(kind, result);
    }

    fn campaign_result(&mut self, kind: TestKind, result: FuzzTestResult) {
        self.kind = kind;

        extend!(self, result, TraceKind::Execution);

        self.status = if result.skipped {
            TestStatus::Skipped
        } else if result.success {
            TestStatus::Success
        } else {
            TestStatus::Failure
        };
        self.reason = result.reason;
        self.counterexample = result.counterexample;
        self.duration = Duration::default();
        self.gas_report_traces = result.gas_report_traces.into_iter().map(|t| vec![t]).collect();
        self.breakpoints = result.breakpoints.unwrap_or_default();
        self.deprecated_cheatcodes = result.deprecated_cheatcodes;
    }

    /// Returns the fail result for fuzz test setup.
    pub fn fuzz_setup_fail(&mut self, e: Report) {
        self.kind = TestKind::Fuzz {
            first_case: Default::default(),
            runs: 0,
            mean_gas: 0,
            median_gas: 0,
            failed_corpus_replays: 0,
        };
        self.status = TestStatus::Failure;
        debug!(?e, "failed to set up fuzz testing environment");
        self.reason = Some(format!("failed to set up fuzz testing environment: {e}"));
    }

    /// Returns the skipped result for invariant campaign with per-predicate outcomes.
    pub fn invariant_skip_with_predicates(
        &mut self,
        reason: SkipReason,
        invariant_predicate_results: Vec<InvariantPredicateResult>,
    ) {
        self.kind = invariant_kind(1, 1, 1);
        self.status = TestStatus::Skipped;
        let predicate_count = invariant_predicate_results.len();
        let is_campaign = predicate_count > 1;
        self.reason = if is_campaign { None } else { reason.0 };
        self.invariant_count = is_campaign.then_some(predicate_count);
        self.invariant_predicate_results = invariant_predicate_results;
    }

    /// Returns the fail result for replayed invariant test.
    pub fn invariant_replay_fail(
        &mut self,
        outcome: CheckSequenceOutcome,
        invariant_name: &str,
        fallback_reason: Option<String>,
        call_sequence: Vec<BaseCounterExample>,
    ) {
        self.kind = invariant_kind(1, outcome.calls_count, outcome.reverts);
        self.status = TestStatus::Failure;
        self.reason = Some(outcome.reason.or(fallback_reason).unwrap_or_else(|| {
            let what = if outcome.replayed_entirely {
                "replay failure"
            } else {
                "persisted failure revert"
            };
            format!("{invariant_name} {what}")
        }));
        self.counterexample = Some(CounterExample::Sequence(call_sequence.len(), call_sequence));
    }

    /// Returns the success result for a replayed invariant test.
    pub fn invariant_replay_success(&mut self, call_count: usize, reverts: usize) {
        self.kind = invariant_kind(1, call_count, reverts);
        self.status = TestStatus::Success;
        self.reason = None;
    }

    /// Returns the fail result for invariant test setup.
    pub fn invariant_setup_fail(&mut self, e: Report) {
        self.kind = invariant_kind(0, 0, 0);
        self.status = TestStatus::Failure;
        self.reason = Some(format!("failed to set up invariant testing environment: {e}"));
    }

    /// Returns the invariant test result.
    pub fn invariant_result(&mut self, kind: TestKind, outcome: InvariantOutcome) {
        // For optimization mode (Some value), always succeed. For check mode (None), use success.
        let optimizing =
            matches!(kind, TestKind::Invariant { optimization_best_value: Some(_), .. });
        self.kind = kind;
        self.status =
            if optimizing || outcome.success { TestStatus::Success } else { TestStatus::Failure };
        self.fork_block_number = outcome.fork_block_number;
        self.invariant_predicate_results = outcome.predicate_results;
        self.invariant_failure_dir = outcome.failure_dir;
        self.invariant_count = outcome.invariant_count;
        // `counterexample` is only used by the renderer for optimization mode (the "best
        // sequence" rendered on success). Invariant check-mode failures live entirely in
        // `invariant_failures`; `reason`/`counterexample` stay `None` for invariant tests.
        self.counterexample = outcome.counterexample;
        for artifact in outcome
            .failures
            .iter()
            .chain(&outcome.handler_failures)
            .flat_map(|failure| replay_artifacts(failure.artifact(), failure.minimization()))
        {
            self.add_counterexample_artifact(artifact.clone());
        }
        self.invariant_failures = outcome.failures;
        self.invariant_handler_failures = outcome.handler_failures;
        self.gas_report_traces = outcome.gas_report_traces;
    }

    /// Returns the result for a symbolic test.
    pub fn symbolic_result(
        &mut self,
        status: TestStatus,
        reason: Option<String>,
        counterexample: Option<CounterExample>,
        symbolic: SymbolicResult,
    ) {
        self.kind = TestKind::Symbolic(symbolic.solver.stats);
        self.status = status;
        self.reason = reason;
        self.counterexample = counterexample;
        self.record_symbolic(symbolic);
        self.duration = Duration::default();
    }

    /// Records symbolic execution metadata without changing the test status/kind.
    pub(crate) fn record_symbolic(&mut self, symbolic: SymbolicResult) {
        for artifact in replay_artifacts(symbolic.artifact.as_ref(), symbolic.minimization.as_ref())
        {
            self.add_counterexample_artifact(artifact.clone());
        }
        self.symbolic = Some(symbolic);
    }

    /// Records a successful showmap replay result.
    pub fn replay_result(
        &mut self,
        corpus_entries: usize,
        showmap_files: usize,
        skipped_entries: usize,
        duration: Duration,
    ) {
        self.kind = TestKind::Replay { corpus_entries, showmap_files, skipped_entries };
        self.status = TestStatus::Success;
        self.duration = duration;
    }

    /// Records a skipped showmap replay (e.g. unit test or no corpus available).
    pub fn replay_skip(&mut self, reason: impl Into<String>) {
        self.kind = TestKind::Replay { corpus_entries: 0, showmap_files: 0, skipped_entries: 0 };
        self.status = TestStatus::Skipped;
        self.reason = Some(reason.into());
        self.duration = Duration::default();
    }

    /// Formats the test result into a string (for printing), naming invariant campaigns after
    /// the suite's contract.
    pub(crate) fn short_result_with_suite(&self, name: &str, suite_name: &str) -> String {
        let campaign = (self.kind.is_invariant() && self.invariant_count.is_some())
            .then(|| invariant_campaign_display_name(get_contract_name(suite_name)));
        let name = campaign.as_deref().unwrap_or(name);
        let status = self.render(true, campaign.as_deref());
        let block = match self.fork_block_number {
            Some(block) if self.status.is_failure() => format!(" (block: {block})"),
            _ => String::new(),
        };
        format!("{status} {name}{block} {}", self.kind.report())
    }

    /// The number of logical tests this result stands for: skipped predicates of a campaign are
    /// counted individually.
    fn logical_count(&self) -> usize {
        let skipped = self.skipped_predicate_count();
        if skipped == 0 {
            1
        } else if self.status.is_skipped() && skipped == self.invariant_predicate_results.len() {
            skipped
        } else {
            1 + skipped
        }
    }

    fn skipped_count(&self) -> usize {
        let skipped = self.skipped_predicate_count();
        if skipped == 0 && self.status.is_skipped() { 1 } else { skipped }
    }

    fn skipped_predicate_count(&self) -> usize {
        self.invariant_predicate_results.iter().filter(|p| p.status.is_skipped()).count()
    }

    /// Merges the given raw call result into `self`.
    pub fn extend<FEN: FoundryEvmNetwork>(&mut self, call_result: RawCallResult<FEN>) {
        extend!(self, call_result, TraceKind::Execution);
    }

    /// Merges the given pre-test setup result into `self`.
    pub(crate) fn extend_setup<FEN: FoundryEvmNetwork>(&mut self, call_result: RawCallResult<FEN>) {
        extend!(self, call_result, TraceKind::Setup);
    }

    /// Merges the given coverage result into `self`.
    pub fn merge_coverages(&mut self, other_coverage: Option<HitMaps>) {
        HitMaps::merge_opt(&mut self.line_coverage, other_coverage);
    }
}

/// Data report by a test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestKindReport {
    Unit {
        gas: u64,
    },
    Fuzz {
        runs: usize,
        mean_gas: u64,
        median_gas: u64,
        failed_corpus_replays: usize,
    },
    Invariant {
        runs: usize,
        calls: usize,
        reverts: usize,
        failed_corpus_replays: usize,
        /// For optimization mode (int256 return): the best value achieved. None = check mode.
        optimization_best_value: Option<I256>,
    },
    Table {
        runs: usize,
        mean_gas: u64,
        median_gas: u64,
    },
    Symbolic(SymbolicStats),
    /// Showmap corpus replay (no campaign performed).
    Replay {
        corpus_entries: usize,
        showmap_files: usize,
        skipped_entries: usize,
    },
}

impl fmt::Display for TestKindReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit { gas } => write!(f, "(gas: {gas})"),
            Self::Fuzz { runs, mean_gas, median_gas, failed_corpus_replays } => {
                write!(f, "(runs: {runs}, μ: {mean_gas}, ~: {median_gas}")?;
                if *failed_corpus_replays != 0 {
                    write!(f, ", failed corpus replays: {failed_corpus_replays}")?;
                }
                f.write_str(")")
            }
            Self::Invariant {
                runs,
                calls,
                reverts,
                failed_corpus_replays,
                optimization_best_value,
            } => {
                if let Some(best_value) = optimization_best_value {
                    return write!(f, "(best: {best_value}, runs: {runs}, calls: {calls})");
                }
                write!(f, "(runs: {runs}, calls: {calls}, reverts: {reverts}")?;
                if *failed_corpus_replays != 0 {
                    write!(f, ", failed corpus replays: {failed_corpus_replays}")?;
                }
                f.write_str(")")
            }
            Self::Table { runs, mean_gas, median_gas } => {
                write!(f, "(runs: {runs}, μ: {mean_gas}, ~: {median_gas})")
            }
            Self::Symbolic(SymbolicStats {
                paths,
                solver_queries,
                smt_queries,
                sat_queries,
                model_queries,
                sat_cache_hits,
                model_cache_hits,
                heuristic_witnesses,
                solver_time_ms,
                ..
            }) => {
                write!(
                    f,
                    "(paths: {paths}, queries: {solver_queries}, smt: {smt_queries}, sat: {sat_queries} ({sat_cache_hits} cached), models: {model_queries} ({model_cache_hits} cached), hard-arith: {heuristic_witnesses}, solver: {solver_time_ms}ms)"
                )
            }
            Self::Replay { corpus_entries, showmap_files, skipped_entries } => {
                write!(f, "(replay: {corpus_entries} entries, {showmap_files} files")?;
                if *skipped_entries != 0 {
                    write!(f, ", {skipped_entries} skipped")?;
                }
                f.write_str(")")
            }
        }
    }
}

impl TestKindReport {
    /// Returns the main gas value to compare against
    pub const fn gas(&self) -> u64 {
        match *self {
            Self::Unit { gas } => gas,
            // We use the median for comparisons
            Self::Fuzz { median_gas, .. } | Self::Table { median_gas, .. } => median_gas,
            // We return 0 since it's not applicable
            Self::Invariant { .. } | Self::Symbolic { .. } | Self::Replay { .. } => 0,
        }
    }
}

/// Various types of tests
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TestKind {
    /// A unit test.
    Unit { gas: u64 },
    /// A fuzz test.
    Fuzz {
        /// we keep this for the debugger
        first_case: FuzzCase,
        runs: usize,
        mean_gas: u64,
        median_gas: u64,
        failed_corpus_replays: usize,
    },
    /// An invariant test.
    Invariant {
        runs: usize,
        calls: usize,
        reverts: usize,
        /// Actual worker count used by this invariant campaign.
        #[serde(default = "default_invariant_workers")]
        workers: usize,
        metrics: Map<String, InvariantMetrics>,
        failed_corpus_replays: usize,
        /// For optimization mode (int256 return): the best value achieved. None = check mode.
        optimization_best_value: Option<I256>,
    },
    /// A table test.
    Table { runs: usize, mean_gas: u64, median_gas: u64 },
    /// A symbolic test.
    Symbolic(SymbolicStats),
    /// Showmap corpus replay (no campaign performed).
    Replay { corpus_entries: usize, showmap_files: usize, skipped_entries: usize },
}

impl Default for TestKind {
    fn default() -> Self {
        Self::Unit { gas: 0 }
    }
}

impl TestKind {
    /// Returns `true` if this is a fuzz test.
    pub const fn is_fuzz(&self) -> bool {
        matches!(self, Self::Fuzz { .. })
    }

    /// Returns `true` if this is an invariant test.
    pub const fn is_invariant(&self) -> bool {
        matches!(self, Self::Invariant { .. })
    }

    /// Returns `true` if this is a symbolic test.
    pub const fn is_symbolic(&self) -> bool {
        matches!(self, Self::Symbolic { .. })
    }

    /// Actual invariant campaign worker count, if this is an invariant test.
    pub const fn invariant_workers(&self) -> Option<usize> {
        match self {
            Self::Invariant { workers, .. } => Some(*workers),
            _ => None,
        }
    }

    /// The gas consumed by this test
    pub const fn report(&self) -> TestKindReport {
        match *self {
            Self::Unit { gas } => TestKindReport::Unit { gas },
            Self::Fuzz { runs, mean_gas, median_gas, failed_corpus_replays, .. } => {
                TestKindReport::Fuzz { runs, mean_gas, median_gas, failed_corpus_replays }
            }
            Self::Invariant {
                runs,
                calls,
                reverts,
                failed_corpus_replays,
                optimization_best_value,
                ..
            } => TestKindReport::Invariant {
                runs,
                calls,
                reverts,
                failed_corpus_replays,
                optimization_best_value,
            },
            Self::Table { runs, mean_gas, median_gas } => {
                TestKindReport::Table { runs, mean_gas, median_gas }
            }
            Self::Symbolic(stats) => TestKindReport::Symbolic(stats),
            Self::Replay { corpus_entries, showmap_files, skipped_entries } => {
                TestKindReport::Replay { corpus_entries, showmap_files, skipped_entries }
            }
        }
    }
}

const fn default_invariant_workers() -> usize {
    1
}

/// The result of a test setup.
///
/// Includes the deployment of the required libraries and the test contract itself, and the call to
/// the `setUp()` function.
#[derive(Clone, Debug, Default)]
pub struct TestSetup {
    /// The address at which the test contract was deployed.
    pub address: Address,
    /// Defined fuzz test fixtures.
    pub fuzz_fixtures: FuzzFixtures,

    /// The logs emitted during setup.
    pub logs: Vec<Log>,
    /// Addresses labeled during setup.
    pub labels: AddressHashMap<String>,
    /// Call traces of the setup.
    pub traces: Traces,
    /// Runtime bytecodes for contracts seen in setup traces.
    pub debug_bytecodes: AddressHashMap<Bytes>,
    /// Coverage info during setup.
    pub coverage: Option<HitMaps>,
    /// Addresses of external libraries deployed during setup.
    pub deployed_libs: Vec<Address>,
    /// The active fork's block number after setup, if any.
    pub fork_block_number: Option<u64>,
    /// Cached setup-derived fuzz dictionary for stateless fuzz tests.
    pub(crate) fuzz_state: OnceLock<EvmFuzzState>,

    /// The reason the setup failed, if it did.
    pub reason: Option<String>,
    /// Whether setup and entire test suite is skipped.
    pub skipped: bool,
    /// Whether the test failed to deploy.
    pub deployment_failure: bool,
}

impl TestSetup {
    pub fn failed(reason: String) -> Self {
        Self { reason: Some(reason), ..Default::default() }
    }

    pub fn skipped(reason: String) -> Self {
        Self { reason: Some(reason), skipped: true, ..Default::default() }
    }

    pub fn extend<FEN: FoundryEvmNetwork>(
        &mut self,
        raw: RawCallResult<FEN>,
        trace_kind: TraceKind,
    ) {
        extend!(self, raw, trace_kind);
    }

    pub fn merge_coverages(&mut self, other_coverage: Option<HitMaps>) {
        HitMaps::merge_opt(&mut self.coverage, other_coverage);
    }
}

pub(crate) fn invariant_campaign_display_name(contract_name: &str) -> String {
    format!("{contract_name} invariants")
}

const fn symbolic_result_schema_version() -> u32 {
    SYMBOLIC_RESULT_SCHEMA_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYMBOLIC_RESULT_SCHEMA: &str =
        include_str!("../../evm/symbolic/assets/symbolic-result.schema.json");
    const SYMBOLIC_COUNTEREXAMPLE_SCHEMA: &str =
        include_str!("../../evm/symbolic/assets/symbolic-counterexample.schema.json");

    fn schema_defs(schema: &serde_json::Value) -> &serde_json::Map<String, serde_json::Value> {
        schema["$defs"].as_object().expect("schema $defs object")
    }

    /// Collects every `$ref` target in `value`.
    fn collect_refs<'a>(value: &'a serde_json::Value, refs: &mut Vec<&'a str>) {
        match value {
            serde_json::Value::Object(map) => {
                refs.extend(map.get("$ref").and_then(serde_json::Value::as_str));
                for child in map.values() {
                    collect_refs(child, refs);
                }
            }
            serde_json::Value::Array(values) => {
                for child in values {
                    collect_refs(child, refs);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn symbolic_schemas_match_result_types() {
        let result_schema: serde_json::Value =
            serde_json::from_str(SYMBOLIC_RESULT_SCHEMA).unwrap();
        let counterexample_schema: serde_json::Value =
            serde_json::from_str(SYMBOLIC_COUNTEREXAMPLE_SCHEMA).unwrap();
        let result_defs = schema_defs(&result_schema);
        let counterexample_defs = schema_defs(&counterexample_schema);

        // Every counterexample `$ref` must resolve offline, either locally or into the result
        // schema.
        let mut refs = Vec::new();
        collect_refs(&counterexample_schema, &mut refs);
        for reference in refs {
            let resolved = if let Some(name) = reference.strip_prefix(
                "https://foundry-rs.github.io/schemas/symbolic-result.v1.schema.json#/$defs/",
            ) {
                result_defs.contains_key(name)
            } else if let Some(name) = reference.strip_prefix("#/$defs/") {
                counterexample_defs.contains_key(name)
            } else {
                false
            };
            assert!(resolved, "unresolved schema ref {reference}");
        }

        // The solver stats schema must list exactly the serialized `SymbolicStats` fields.
        let stats = serde_json::to_value(SymbolicStats::default()).unwrap();
        let mut expected = stats.as_object().unwrap().keys().collect::<Vec<_>>();
        let mut actual = result_defs["solver_stats"]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>();
        expected.sort();
        actual.sort();
        assert_eq!(actual, expected);
    }

    fn outcome_with_results(test_results: Vec<TestResult>) -> TestOutcome {
        let test_results = test_results
            .into_iter()
            .enumerate()
            .map(|(idx, result)| (format!("test{idx}()"), result))
            .collect();
        let suite = SuiteResult::new(Duration::ZERO, test_results, Vec::new());
        TestOutcome::new(None, BTreeMap::from([("suite".to_string(), suite)]), false, None)
    }

    fn failed_result(kind: TestKind) -> TestResult {
        TestResult { status: TestStatus::Failure, kind, ..Default::default() }
    }

    fn failed_invariant(workers: usize) -> TestResult {
        let mut kind = invariant_kind(0, 0, 0);
        if let TestKind::Invariant { workers: w, .. } = &mut kind {
            *w = workers;
        }
        failed_result(kind)
    }

    #[test]
    fn failed_tests_are_debuggable_only_for_concrete_failures() {
        let unit = failed_result(TestKind::Unit { gas: 0 });
        assert!(outcome_with_results(vec![unit.clone()]).failed_tests_are_debuggable());
        assert!(!outcome_with_results(vec![failed_invariant(1)]).failed_tests_are_debuggable());
        assert!(
            !outcome_with_results(vec![failed_result(
                TestKind::Symbolic(SymbolicStats::default())
            )])
            .failed_tests_are_debuggable()
        );

        let mut symbolic_backed = unit;
        symbolic_backed.symbolic =
            Some(SymbolicResult::pass(&SymbolicConfig::default(), SymbolicStats::default()));
        assert!(!outcome_with_results(vec![symbolic_backed]).failed_tests_are_debuggable());
    }

    #[test]
    fn invariant_workers_hint_requires_matching_parallel_worker_counts() {
        let hint = |workers: &[usize]| {
            outcome_with_results(workers.iter().map(|&w| failed_invariant(w)).collect())
                .invariant_workers_hint()
        };
        assert_eq!(hint(&[3, 3]), Some(3));
        assert_eq!(hint(&[2, 3]), None);
        assert_eq!(hint(&[1]), None);
    }

    #[test]
    fn invariant_kind_deserializes_legacy_payload_without_workers() {
        let kind = serde_json::from_value::<TestKind>(serde_json::json!({
            "Invariant": {
                "runs": 4,
                "calls": 10,
                "reverts": 0,
                "metrics": {},
                "failed_corpus_replays": 0,
                "optimization_best_value": null
            }
        }))
        .unwrap();

        assert_eq!(kind.invariant_workers(), Some(1));
    }
}
