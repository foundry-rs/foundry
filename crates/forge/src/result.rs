//! Test outcomes.

use crate::{
    fuzz::{BaseCounterExample, FuzzedCases},
    gas_report::GasReport,
};
use alloy_primitives::{
    Address, I256, Log, Selector, U256,
    map::{AddressHashMap, HashMap},
};
use eyre::Report;
use foundry_common::{ContractsByArtifact, get_contract_name, get_file_name, shell};
use foundry_evm::{
    core::{Breakpoints, evm::FoundryEvmNetwork},
    coverage::HitMaps,
    decode::SkipReason,
    executors::{RawCallResult, invariant::InvariantMetrics},
    fuzz::{CounterExample, FuzzCase, FuzzFixtures, FuzzTestResult},
    traces::{CallTraceArena, CallTraceDecoder, TraceKind, Traces},
};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap as Map},
    fmt::{self, Write},
    time::Duration,
};
use yansi::Paint;

/// The aggregated result of a test run.
#[derive(Clone, Debug)]
pub struct TestOutcome {
    /// The results of all test suites by their identifier (`path:contract_name`).
    ///
    /// Essentially `identifier => signature => result`.
    pub results: BTreeMap<String, SuiteResult>,
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
    // TODO: Replace this with `tests` and make it return `TestRef<'_>`
    pub fn into_tests_cloned(&self) -> impl Iterator<Item = SuiteTestResult> + '_ {
        self.results
            .iter()
            .flat_map(|(file, suite)| {
                suite
                    .test_results
                    .iter()
                    .map(move |(sig, result)| (file.clone(), sig.clone(), result.clone()))
            })
            .map(|(artifact_id, signature, result)| SuiteTestResult {
                artifact_id,
                signature,
                result,
            })
    }

    /// Flattens the test outcome into a list of individual tests.
    pub fn into_tests(self) -> impl Iterator<Item = SuiteTestResult> {
        self.results
            .into_iter()
            .flat_map(|(file, suite)| {
                suite.test_results.into_iter().map(move |t| (file.clone(), t))
            })
            .map(|(artifact_id, (signature, result))| SuiteTestResult {
                artifact_id,
                signature,
                result,
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
        let total_passed = self.passed();
        let total_failed = self.failed();
        let total_skipped = self.skipped();
        let total_tests = total_passed + total_failed + total_skipped;
        format!(
            "\nRan {} test {} in {:.2?} ({:.2?} CPU time): {} tests passed, {} failed, {} skipped ({} total tests)",
            num_test_suites,
            suites,
            wall_clock_time,
            self.total_time(),
            total_passed.green(),
            total_failed.red(),
            total_skipped.yellow(),
            total_tests
        )
    }

    /// Checks if there are any failures and failures are disallowed.
    pub fn ensure_ok(&self, silent: bool) -> eyre::Result<()> {
        let outcome = self;
        let failures = outcome.failures().count();
        if outcome.allow_failure || failures == 0 {
            return Ok(());
        }

        if shell::is_quiet() || silent {
            std::process::exit(1);
        }

        sh_println!("\nFailing tests:")?;
        for (suite_name, suite) in &outcome.results {
            let failed = suite.failed();
            if failed == 0 {
                continue;
            }

            let term = if failed > 1 { "tests" } else { "test" };
            sh_println!("Encountered {failed} failing {term} in {suite_name}")?;
            for (name, result) in suite.failures() {
                sh_println!("{}", result.short_result(name))?;
            }
            sh_println!()?;
        }
        let successes = outcome.passed();
        sh_println!(
            "Encountered a total of {} failing tests, {} tests succeeded",
            failures.to_string().red(),
            successes.to_string().green()
        )?;

        // Show helpful hint for rerunning failed tests
        let test_word = if failures == 1 { "test" } else { "tests" };
        sh_println!(
            "\nTip: Run {} to retry only the {} failed {}",
            "`forge test --rerun`".cyan(),
            failures,
            test_word
        )?;

        // Print seed for fuzz/invariant test failures to enable reproduction.
        if let Some(seed) = self.fuzz_seed
            && outcome.has_fuzz_failures()
        {
            sh_println!(
                "\nFuzz seed: {} (use {} to reproduce)",
                format!("{seed:#x}").cyan(),
                "`--fuzz-seed`".cyan()
            )?;
        }

        std::process::exit(1);
    }

    /// Removes first test result, if any.
    pub fn remove_first(&mut self) -> Option<(String, String, TestResult)> {
        self.results.iter_mut().find_map(|(suite_name, suite)| {
            if let Some(test_name) = suite.test_results.keys().next().cloned() {
                let result = suite.test_results.remove(&test_name).unwrap();
                Some((suite_name.clone(), test_name, result))
            } else {
                None
            }
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
        let mut deprecated_cheatcodes = HashMap::new();
        for test_result in test_results.values() {
            deprecated_cheatcodes.extend(test_result.deprecated_cheatcodes.clone());
        }
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
        self.test_results.values().map(TestResult::passed_count).sum()
    }

    /// Returns the number of tests that were skipped.
    pub fn skipped(&self) -> usize {
        self.test_results.values().map(TestResult::skipped_count).sum()
    }

    /// Returns the number of tests that failed.
    pub fn failed(&self) -> usize {
        self.test_results.values().map(TestResult::failed_count).sum()
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
            "Suite result: {}. {} passed; {} failed; {} skipped; finished in {:.2?} ({:.2?} CPU time)",
            result,
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
    pub fn gas_used(&self) -> u64 {
        self.result.kind.report().gas()
    }

    /// Returns the contract name of the artifact ID.
    pub fn contract_name(&self) -> &str {
        get_contract_name(&self.artifact_id)
    }

    /// Returns the file name of the artifact ID.
    pub fn file_name(&self) -> &str {
        get_file_name(&self.artifact_id)
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
        /// Path where the counterexample was persisted for re-running and shrinking.
        persisted_path: std::path::PathBuf,
        /// Whether this failure is the stable campaign anchor.
        /// When `true` and this is the only failure, the function name is omitted on the
        /// `[FAIL: ...]` line (the trailing summary already identifies it).
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
    pub invariant_failure_dir: Option<std::path::PathBuf>,

    /// Total number of invariant predicates exercised in this campaign. When `Some(n)` the report
    /// renders
    /// an `Invariant/Property Tests: <broken>/<n> invariants broken` summary so users get an
    /// at-a-glance health line without counting `[FAIL]` blocks. `None` for single-predicate
    /// campaigns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invariant_count: Option<usize>,

    /// Handler-side assertion bugs found during the campaign, deduped by
    /// `(reverter, selector)` site (Medusa/Echidna semantics). Rendered in a dedicated
    /// `Assertion Tests` section.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invariant_handler_failures: Vec<InvariantFailure>,

    /// Minimal reproduction test case for failing test
    pub counterexample: Option<CounterExample>,

    /// Any captured & parsed as strings logs along the test's execution which should
    /// be printed to the user.
    pub logs: Vec<Log>,

    /// The decoded DSTest logging events and Hardhat's `console.log` from [logs](Self::logs).
    /// Used for json output.
    pub decoded_logs: Vec<String>,

    /// What kind of test this was
    pub kind: TestKind,

    /// Traces
    pub traces: Traces,

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
}

impl fmt::Display for TestResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render_status_block(false))
    }
}

impl TestResult {
    fn render_status_block(&self, user_facing: bool) -> String {
        match self.status {
            TestStatus::Success => {
                // For optimization mode, show the best example sequence in green.
                let mut s = String::from("[PASS]");
                if let Some(CounterExample::Sequence(original, sequence)) = &self.counterexample {
                    s.push_str(
                        format!(
                            "\n\t[Best sequence] (original: {original}, shrunk: {})\n",
                            sequence.len()
                        )
                        .as_str(),
                    );
                    for ex in sequence {
                        writeln!(s, "{ex}").unwrap();
                    }
                }
                self.write_invariant_predicate_results(&mut s, user_facing, true);
                format!("{}", s.green().wrap())
            }
            TestStatus::Skipped => {
                let mut s = String::from("[SKIP");
                if let Some(reason) = &self.reason {
                    write!(s, ": {reason}").unwrap();
                }
                s.push(']');
                self.write_invariant_predicate_results(&mut s, user_facing, true);
                format!("{}", s.yellow())
            }
            TestStatus::Failure => {
                let mut s = String::new();
                let has_handler_failures = !self.invariant_handler_failures.is_empty();
                let is_invariant_failure =
                    !self.invariant_failures.is_empty() || has_handler_failures;
                if !is_invariant_failure {
                    // Non-invariant failure (unit / fuzz / DS-style): render from the legacy
                    // `reason` / `counterexample` fields.
                    s.push_str("[FAIL");
                    if let Some(reason) = &self.reason {
                        write!(s, ": {reason}").unwrap();
                    }
                    if let Some(counterexample) = &self.counterexample {
                        match counterexample {
                            CounterExample::Single(ex) => {
                                write!(s, "; counterexample: {ex}]").unwrap();
                            }
                            CounterExample::Sequence(original, sequence) => {
                                writeln!(
                                    s,
                                    "]\n\t[Sequence] (original: {original}, shrunk: {})",
                                    sequence.len()
                                )
                                .unwrap();
                                for ex in sequence {
                                    writeln!(s, "{ex}").unwrap();
                                }
                            }
                        }
                    } else {
                        s.push(']');
                    }
                } else if !self.invariant_failures.is_empty() {
                    // Render every broken invariant uniformly. Show the function name on the
                    // `[FAIL: ...]` line when there is >1 failure or the failure isn't the
                    // anchor (the anchor's name is already on the trailing summary).
                    let multi = self.invariant_failures.len() > 1;
                    for (i, failure) in self.invariant_failures.iter().enumerate() {
                        if i > 0 {
                            s.push('\n');
                        }
                        let is_anchor =
                            matches!(failure, InvariantFailure::Predicate { is_anchor: true, .. });
                        let name_suffix = if multi || !is_anchor {
                            format!(" {}", failure.name())
                        } else {
                            String::new()
                        };
                        if let Some(CounterExample::Sequence(original, sequence)) =
                            failure.counterexample()
                        {
                            writeln!(
                                s,
                                "[FAIL: {}]{name_suffix}\n\t[Sequence] (original: {original}, shrunk: {})",
                                failure.reason(),
                                sequence.len()
                            )
                            .unwrap();
                            for ex in sequence {
                                writeln!(s, "{ex}").unwrap();
                            }
                        } else {
                            write!(s, "[FAIL: {}]{name_suffix}", failure.reason()).unwrap();
                        }
                    }
                }

                let rollup_rendered =
                    self.write_invariant_rollup(&mut s, user_facing, is_invariant_failure);
                let show_predicate_header = if user_facing { !rollup_rendered } else { true };
                self.write_invariant_predicate_results(&mut s, user_facing, show_predicate_header);
                self.write_invariant_persistence_note(&mut s);
                let handler_preceded = if user_facing {
                    rollup_rendered
                        || self.invariant_predicate_results.len() > 1
                        || !self.invariant_failures.is_empty()
                } else {
                    !self.invariant_failures.is_empty()
                        || matches!(self.invariant_count, Some(t) if t > 1)
                };
                self.write_handler_failures(&mut s, user_facing, handler_preceded);

                format!("{}", s.red().wrap())
            }
        }
    }

    fn write_invariant_rollup(
        &self,
        s: &mut String,
        user_facing: bool,
        is_invariant_failure: bool,
    ) -> bool {
        let Some(total) = self.invariant_count else {
            return false;
        };
        if total <= 1 || !is_invariant_failure {
            return false;
        }

        writeln!(
            s,
            "\n{}: {}/{total} invariants broken",
            if user_facing { "Invariant/Property Tests" } else { "Predicates" },
            self.invariant_failures.len()
        )
        .unwrap();
        true
    }

    fn write_invariant_persistence_note(&self, s: &mut String) {
        if self.invariant_failures.len() > 1
            && let Some(dir) = &self.invariant_failure_dir
        {
            writeln!(
                s,
                "{} invariant failure(s) persisted to {} — rerun to shrink",
                self.invariant_failures.len(),
                dir.display()
            )
            .unwrap();
        }
    }

    fn write_handler_failures(&self, s: &mut String, user_facing: bool, preceded: bool) {
        if self.invariant_handler_failures.is_empty() {
            return;
        }

        let prefix = if preceded { "\n" } else { "" };
        writeln!(
            s,
            "{prefix}{}: {} assertion bug(s) found",
            if user_facing { "Assertion Tests" } else { "Handler assertions" },
            self.invariant_handler_failures.len()
        )
        .unwrap();
        for failure in &self.invariant_handler_failures {
            if let Some(CounterExample::Sequence(original, sequence)) = failure.counterexample() {
                writeln!(
                    s,
                    "[FAIL: {}] {}\n\t[Sequence] (original: {original}, shrunk: {})",
                    failure.reason(),
                    failure.name(),
                    sequence.len()
                )
                .unwrap();
                for ex in sequence {
                    writeln!(s, "{ex}").unwrap();
                }
            } else {
                writeln!(s, "[FAIL: {}] {}", failure.reason(), failure.name()).unwrap();
            }
        }
    }

    /// Appends the invariant/property summary for multi-predicate campaigns.
    fn write_invariant_predicate_results(
        &self,
        s: &mut String,
        user_facing: bool,
        show_header: bool,
    ) {
        if self.invariant_predicate_results.len() <= 1 {
            return;
        }

        if show_header {
            s.push('\n');
            s.push_str(if user_facing { "Invariant/Property Tests" } else { "Predicates" });
            s.push_str(":\n");
        }

        for predicate in &self.invariant_predicate_results {
            match predicate.status {
                TestStatus::Success => {
                    writeln!(s, "[PASS] {}", predicate.name).unwrap();
                }
                TestStatus::Failure => {
                    let reason = predicate.reason.as_deref().unwrap_or_default();
                    writeln!(s, "[FAIL: {reason}] {}", predicate.name).unwrap();
                }
                TestStatus::Skipped => {
                    if let Some(reason) = &predicate.reason {
                        writeln!(s, "[SKIP: {reason}] {}", predicate.name).unwrap();
                    } else {
                        writeln!(s, "[SKIP] {}", predicate.name).unwrap();
                    }
                }
            }
        }
    }
}

macro_rules! extend {
    ($a:expr, $b:expr, $trace_kind:expr) => {
        $a.logs.extend($b.logs);
        $a.labels.extend($b.labels);
        $a.traces.extend($b.traces.map(|traces| ($trace_kind, traces)));
        $a.merge_coverages($b.line_coverage);
    };
}

impl TestResult {
    /// Creates a new test result starting from test setup results.
    pub fn new(setup: &TestSetup) -> Self {
        Self {
            labels: setup.labels.clone(),
            logs: setup.logs.clone(),
            traces: setup.traces.clone(),
            line_coverage: setup.coverage.clone(),
            ..Default::default()
        }
    }

    /// Creates a failed test result with given reason.
    pub fn fail(reason: String) -> Self {
        Self { status: TestStatus::Failure, reason: Some(reason), ..Default::default() }
    }

    /// Creates a test setup result.
    pub fn setup_result(setup: TestSetup) -> Self {
        let TestSetup {
            address: _,
            fuzz_fixtures: _,
            logs,
            labels,
            traces,
            coverage,
            deployed_libs: _,
            reason,
            skipped,
            deployment_failure: _,
        } = setup;
        Self {
            status: if skipped { TestStatus::Skipped } else { TestStatus::Failure },
            reason,
            logs,
            traces,
            line_coverage: coverage,
            labels,
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

        self.status = match success {
            true => TestStatus::Success,
            false => TestStatus::Failure,
        };
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
    pub fn fuzz_result(&mut self, result: FuzzTestResult) {
        self.kind = TestKind::Fuzz {
            median_gas: result.median_gas(false),
            mean_gas: result.mean_gas(false),
            first_case: result.first_case,
            runs: result.gas_by_case.len(),
            failed_corpus_replays: result.failed_corpus_replays,
        };

        // Record logs, labels, traces and merge coverages.
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

    /// Returns the skipped result for invariant test.
    pub fn invariant_skip(&mut self, reason: SkipReason) {
        self.invariant_skip_with_predicates(reason, Vec::new());
    }

    /// Returns the skipped result for invariant campaign with per-predicate outcomes.
    pub fn invariant_skip_with_predicates(
        &mut self,
        reason: SkipReason,
        invariant_predicate_results: Vec<InvariantPredicateResult>,
    ) {
        self.kind = TestKind::Invariant {
            runs: 1,
            calls: 1,
            reverts: 1,
            metrics: HashMap::default(),
            failed_corpus_replays: 0,
            optimization_best_value: None,
        };
        self.status = TestStatus::Skipped;
        self.reason = reason.0;
        self.invariant_count =
            (invariant_predicate_results.len() > 1).then_some(invariant_predicate_results.len());
        self.invariant_predicate_results = invariant_predicate_results;
    }

    /// Returns the fail result for replayed invariant test.
    pub fn invariant_replay_fail(
        &mut self,
        replayed_entirely: bool,
        invariant_name: &String,
        replay_reason: Option<String>,
        call_sequence: Vec<BaseCounterExample>,
    ) {
        self.kind = TestKind::Invariant {
            runs: 1,
            calls: 1,
            reverts: 1,
            metrics: HashMap::default(),
            failed_corpus_replays: 0,
            optimization_best_value: None,
        };
        self.status = TestStatus::Failure;
        self.reason = replay_reason.or_else(|| {
            if replayed_entirely {
                Some(format!("{invariant_name} replay failure"))
            } else {
                Some(format!("{invariant_name} persisted failure revert"))
            }
        });
        self.counterexample = Some(CounterExample::Sequence(call_sequence.len(), call_sequence));
    }

    /// Returns the fail result for invariant test setup.
    pub fn invariant_setup_fail(&mut self, e: Report) {
        self.kind = TestKind::Invariant {
            runs: 0,
            calls: 0,
            reverts: 0,
            metrics: HashMap::default(),
            failed_corpus_replays: 0,
            optimization_best_value: None,
        };
        self.status = TestStatus::Failure;
        self.reason = Some(format!("failed to set up invariant testing environment: {e}"));
    }

    /// Returns the invariant test result.
    #[expect(clippy::too_many_arguments)]
    pub fn invariant_result(
        &mut self,
        gas_report_traces: Vec<Vec<CallTraceArena>>,
        success: bool,
        invariant_failures: Vec<InvariantFailure>,
        invariant_predicate_results: Vec<InvariantPredicateResult>,
        invariant_failure_dir: Option<std::path::PathBuf>,
        invariant_count: Option<usize>,
        invariant_handler_failures: Vec<InvariantFailure>,
        counterexample: Option<CounterExample>,
        cases: Vec<FuzzedCases>,
        reverts: usize,
        metrics: Map<String, InvariantMetrics>,
        failed_corpus_replays: usize,
        optimization_best_value: Option<I256>,
    ) {
        self.kind = TestKind::Invariant {
            runs: cases.len(),
            calls: cases.iter().map(|sequence| sequence.cases().len()).sum(),
            reverts,
            metrics,
            failed_corpus_replays,
            optimization_best_value,
        };
        // For optimization mode (Some value), always succeed. For check mode (None), use success.
        self.status = if optimization_best_value.is_some() || success {
            TestStatus::Success
        } else {
            TestStatus::Failure
        };
        self.invariant_failures = invariant_failures;
        self.invariant_predicate_results = invariant_predicate_results;
        self.invariant_failure_dir = invariant_failure_dir;
        self.invariant_count = invariant_count;
        self.invariant_handler_failures = invariant_handler_failures;
        // `counterexample` is only used by the renderer for optimization mode (the "best
        // sequence" rendered on success). Invariant check-mode failures live entirely in
        // `invariant_failures`; `reason`/`counterexample` stay `None` for invariant tests.
        self.counterexample = counterexample;
        self.gas_report_traces = gas_report_traces;
    }

    /// Returns the result for a table test. Merges table test execution results (logs, labeled
    /// addresses, traces and coverages) in initial setup results.
    pub fn table_result(&mut self, result: FuzzTestResult) {
        self.kind = TestKind::Table {
            median_gas: result.median_gas(false),
            mean_gas: result.mean_gas(false),
            runs: result.gas_by_case.len(),
        };

        // Record logs, labels, traces and merge coverages.
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

    /// Returns `true` if this is the result of a fuzz test
    pub const fn is_fuzz(&self) -> bool {
        matches!(self.kind, TestKind::Fuzz { .. })
    }

    /// Formats the test result into a string (for printing).
    pub fn short_result(&self, name: &str) -> String {
        if self.status.is_skipped() && self.invariant_predicate_results.len() > 1 {
            return self
                .invariant_predicate_results
                .iter()
                .map(|predicate| {
                    let mut s = String::from("[SKIP");
                    if let Some(reason) = &predicate.reason {
                        write!(s, ": {reason}").unwrap();
                    }
                    s.push(']');
                    format!("{} {}() {}", s.yellow(), predicate.name, self.kind.report())
                })
                .join("\n");
        }
        format!("{} {name} {}", self.render_status_block(true), self.kind.report())
    }

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

    fn passed_count(&self) -> usize {
        usize::from(self.status.is_success())
    }

    fn skipped_count(&self) -> usize {
        let skipped = self.skipped_predicate_count();
        if skipped == 0 && self.status.is_skipped() { 1 } else { skipped }
    }

    fn failed_count(&self) -> usize {
        usize::from(self.status.is_failure())
    }

    fn skipped_predicate_count(&self) -> usize {
        self.invariant_predicate_results
            .iter()
            .filter(|predicate| predicate.status.is_skipped())
            .count()
    }

    /// Merges the given raw call result into `self`.
    pub fn extend<FEN: FoundryEvmNetwork>(&mut self, call_result: RawCallResult<FEN>) {
        extend!(self, call_result, TraceKind::Execution);
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
        metrics: Map<String, InvariantMetrics>,
        failed_corpus_replays: usize,
        /// For optimization mode (int256 return): the best value achieved. None = check mode.
        optimization_best_value: Option<I256>,
    },
    Table {
        runs: usize,
        mean_gas: u64,
        median_gas: u64,
    },
}

impl fmt::Display for TestKindReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit { gas } => {
                write!(f, "(gas: {gas})")
            }
            Self::Fuzz { runs, mean_gas, median_gas, failed_corpus_replays } => {
                if *failed_corpus_replays != 0 {
                    write!(
                        f,
                        "(runs: {runs}, μ: {mean_gas}, ~: {median_gas}, failed corpus replays: {failed_corpus_replays})"
                    )
                } else {
                    write!(f, "(runs: {runs}, μ: {mean_gas}, ~: {median_gas})")
                }
            }
            Self::Invariant {
                runs,
                calls,
                reverts,
                metrics: _,
                failed_corpus_replays,
                optimization_best_value,
            } => {
                // If optimization_best_value is Some, this is optimization mode.
                if let Some(best_value) = optimization_best_value {
                    write!(f, "(best: {best_value}, runs: {runs}, calls: {calls})")
                } else if *failed_corpus_replays != 0 {
                    write!(
                        f,
                        "(runs: {runs}, calls: {calls}, reverts: {reverts}, failed corpus replays: {failed_corpus_replays})"
                    )
                } else {
                    write!(f, "(runs: {runs}, calls: {calls}, reverts: {reverts})")
                }
            }
            Self::Table { runs, mean_gas, median_gas } => {
                write!(f, "(runs: {runs}, μ: {mean_gas}, ~: {median_gas})")
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
            Self::Invariant { .. } => 0,
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
        metrics: Map<String, InvariantMetrics>,
        failed_corpus_replays: usize,
        /// For optimization mode (int256 return): the best value achieved. None = check mode.
        optimization_best_value: Option<I256>,
    },
    /// A table test.
    Table { runs: usize, mean_gas: u64, median_gas: u64 },
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

    /// The gas consumed by this test
    pub fn report(&self) -> TestKindReport {
        match self {
            Self::Unit { gas } => TestKindReport::Unit { gas: *gas },
            Self::Fuzz { first_case: _, runs, mean_gas, median_gas, failed_corpus_replays } => {
                TestKindReport::Fuzz {
                    runs: *runs,
                    mean_gas: *mean_gas,
                    median_gas: *median_gas,
                    failed_corpus_replays: *failed_corpus_replays,
                }
            }
            Self::Invariant {
                runs,
                calls,
                reverts,
                metrics: _,
                failed_corpus_replays,
                optimization_best_value,
            } => TestKindReport::Invariant {
                runs: *runs,
                calls: *calls,
                reverts: *reverts,
                metrics: HashMap::default(),
                failed_corpus_replays: *failed_corpus_replays,
                optimization_best_value: *optimization_best_value,
            },
            Self::Table { runs, mean_gas, median_gas } => {
                TestKindReport::Table { runs: *runs, mean_gas: *mean_gas, median_gas: *median_gas }
            }
        }
    }
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
    /// Coverage info during setup.
    pub coverage: Option<HitMaps>,
    /// Addresses of external libraries deployed during setup.
    pub deployed_libs: Vec<Address>,

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
