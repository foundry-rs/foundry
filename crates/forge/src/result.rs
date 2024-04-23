//! Test outcomes.

use alloy_primitives::{Address, Log, U256};
use foundry_common::{
    evm::Breakpoints, get_contract_name, get_file_name, shell, ContractsByArtifact,
};
use foundry_compilers::artifacts::Libraries;
use foundry_evm::{
    coverage::HitMaps,
    debug::DebugArena,
    executors::EvmError,
    fuzz::{CounterExample, FuzzCase, FuzzFixtures},
    inspectors::Context,
    traces::{CallTraceArena, CallTraceDecoder, TraceKind, Traces},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fmt::{self, Write},
    sync::Arc,
    time::Duration,
};
use yansi::Paint;

use crate::gas_report::GasReport;

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
}

impl TestOutcome {
    /// Creates a new test outcome with the given results.
    pub fn new(results: BTreeMap<String, SuiteResult>, allow_failure: bool) -> Self {
        Self { results, allow_failure, last_run_decoder: None, gas_report: None }
    }

    /// Creates a new empty test outcome.
    pub fn empty(allow_failure: bool) -> Self {
        Self::new(BTreeMap::new(), allow_failure)
    }

    /// Returns an iterator over all individual succeeding tests and their names.
    pub fn successes(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.tests().filter(|(_, t)| t.status == TestStatus::Success)
    }

    /// Returns an iterator over all individual skipped tests and their names.
    pub fn skips(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.tests().filter(|(_, t)| t.status == TestStatus::Skipped)
    }

    /// Returns an iterator over all individual failing tests and their names.
    pub fn failures(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.tests().filter(|(_, t)| t.status == TestStatus::Failure)
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
        self.successes().count()
    }

    /// Returns the number of tests that were skipped.
    pub fn skipped(&self) -> usize {
        self.skips().count()
    }

    /// Returns the number of tests that failed.
    pub fn failed(&self) -> usize {
        self.failures().count()
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
    pub fn ensure_ok(&self) -> eyre::Result<()> {
        let outcome = self;
        let failures = outcome.failures().count();
        if outcome.allow_failure || failures == 0 {
            return Ok(());
        }

        if !shell::verbosity().is_normal() {
            // TODO: Avoid process::exit
            std::process::exit(1);
        }

        shell::println("")?;
        shell::println("Failing tests:")?;
        for (suite_name, suite) in outcome.results.iter() {
            let failed = suite.failed();
            if failed == 0 {
                continue;
            }

            let term = if failed > 1 { "tests" } else { "test" };
            shell::println(format!("Encountered {failed} failing {term} in {suite_name}"))?;
            for (name, result) in suite.failures() {
                shell::println(result.short_result(name))?;
            }
            shell::println("")?;
        }
        let successes = outcome.passed();
        shell::println(format!(
            "Encountered a total of {} failing tests, {} tests succeeded",
            failures.to_string().red(),
            successes.to_string().green()
        ))?;

        // TODO: Avoid process::exit
        std::process::exit(1);
    }
}

/// A set of test results for a single test suite, which is all the tests in a single contract.
#[derive(Clone, Debug, Serialize)]
pub struct SuiteResult {
    /// Wall clock time it took to execute all tests in this suite.
    #[serde(with = "humantime_serde")]
    pub duration: Duration,
    /// Individual test results: `test fn signature -> TestResult`.
    pub test_results: BTreeMap<String, TestResult>,
    /// Generated warnings.
    pub warnings: Vec<String>,
    /// Libraries used to link test contract.
    pub libraries: Libraries,
    /// Contracts linked with correct libraries.
    #[serde(skip)]
    pub known_contracts: Arc<ContractsByArtifact>,
}

impl SuiteResult {
    pub fn new(
        duration: Duration,
        test_results: BTreeMap<String, TestResult>,
        warnings: Vec<String>,
        libraries: Libraries,
        known_contracts: Arc<ContractsByArtifact>,
    ) -> Self {
        Self { duration, test_results, warnings, libraries, known_contracts }
    }

    /// Returns an iterator over all individual succeeding tests and their names.
    pub fn successes(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.tests().filter(|(_, t)| t.status == TestStatus::Success)
    }

    /// Returns an iterator over all individual skipped tests and their names.
    pub fn skips(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.tests().filter(|(_, t)| t.status == TestStatus::Skipped)
    }

    /// Returns an iterator over all individual failing tests and their names.
    pub fn failures(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.tests().filter(|(_, t)| t.status == TestStatus::Failure)
    }

    /// Returns the number of tests that passed.
    pub fn passed(&self) -> usize {
        self.successes().count()
    }

    /// Returns the number of tests that were skipped.
    pub fn skipped(&self) -> usize {
        self.skips().count()
    }

    /// Returns the number of tests that failed.
    pub fn failed(&self) -> usize {
        self.failures().count()
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
        self.test_results.len()
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
    pub fn is_success(self) -> bool {
        matches!(self, Self::Success)
    }

    /// Returns `true` if the test failed.
    #[inline]
    pub fn is_failure(self) -> bool {
        matches!(self, Self::Failure)
    }

    /// Returns `true` if the test was skipped.
    #[inline]
    pub fn is_skipped(self) -> bool {
        matches!(self, Self::Skipped)
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

    /// Minimal reproduction test case for failing test
    pub counterexample: Option<CounterExample>,

    /// Any captured & parsed as strings logs along the test's execution which should
    /// be printed to the user.
    pub logs: Vec<Log>,

    /// The decoded DSTest logging events and Hardhat's `console.log` from [logs](Self::logs).
    pub decoded_logs: Vec<String>,

    /// What kind of test this was.
    pub kind: TestKind,

    /// What kind of environment this test was run in.
    #[serde(skip)]
    pub environment: TestEnvironment,

    /// Traces
    #[serde(skip)]
    pub traces: Traces,

    /// Contexts
    #[serde(skip)]
    pub contexts: Vec<Context>,

    /// Additional traces to use for gas report.
    #[serde(skip)]
    pub gas_report_traces: Vec<Vec<CallTraceArena>>,

    /// Raw coverage info
    #[serde(skip)]
    pub coverage: Option<HitMaps>,

    /// Labeled addresses
    pub labeled_addresses: HashMap<Address, String>,

    /// The debug nodes of the call
    pub debug: Option<DebugArena>,

    pub duration: Duration,

    /// pc breakpoint char map
    pub breakpoints: Breakpoints,
}

impl fmt::Display for TestResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            TestStatus::Success => "[PASS]".green().fmt(f),
            TestStatus::Skipped => "[SKIP]".yellow().fmt(f),
            TestStatus::Failure => {
                let mut s = String::from("[FAIL. Reason: ");

                let reason = self.reason.as_deref().unwrap_or("assertion failed");
                s.push_str(reason);

                if let Some(counterexample) = &self.counterexample {
                    match counterexample {
                        CounterExample::Single(ex) => {
                            write!(s, "; counterexample: {ex}]").unwrap();
                        }
                        CounterExample::Sequence(sequence) => {
                            s.push_str("]\n\t[Sequence]\n");
                            for ex in sequence {
                                writeln!(s, "\t\t{ex}").unwrap();
                            }
                        }
                    }
                } else {
                    s.push(']');
                }

                s.red().fmt(f)
            }
        }
    }
}

impl TestResult {
    pub fn fail(reason: String) -> Self {
        Self { status: TestStatus::Failure, reason: Some(reason), ..Default::default() }
    }

    /// Returns `true` if this is the result of a fork test
    pub fn is_fork(&self) -> bool {
        matches!(self.environment, TestEnvironment::Fork { .. })
    }

    /// Returns `true` if this is the result of a fuzz test
    pub fn is_fuzz(&self) -> bool {
        matches!(self.kind, TestKind::Fuzz { .. })
    }

    /// Formats a result into a string (for printing).
    pub fn short_result(&self, name: &str) -> String {
        if self.status == TestStatus::Success {
            format!("{self} {name} {}", self.kind.report())
        } else {
            format!("{self} {name} {}{}", self.environment.report(), self.kind.report())
        }
    }
}

/// Various types of tests
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TestKind {
    /// A standard test that consists of calling the defined solidity function
    ///
    /// Holds the consumed gas
    Standard(u64),
    /// A solidity fuzz test, that stores all test cases
    Fuzz {
        /// we keep this for the debugger
        first_case: FuzzCase,
        runs: usize,
        mean_gas: u64,
        median_gas: u64,
    },
    /// A solidity invariant test, that stores all test cases
    Invariant { runs: usize, calls: usize, reverts: usize },
}

impl Default for TestKind {
    fn default() -> Self {
        Self::Standard(0)
    }
}

impl TestKind {
    /// The gas consumed by this test
    pub fn report(&self) -> TestKindReport {
        match self {
            TestKind::Standard(gas) => TestKindReport::Standard { gas: *gas },
            TestKind::Fuzz { runs, mean_gas, median_gas, .. } => {
                TestKindReport::Fuzz { runs: *runs, mean_gas: *mean_gas, median_gas: *median_gas }
            }
            TestKind::Invariant { runs, calls, reverts } => {
                TestKindReport::Invariant { runs: *runs, calls: *calls, reverts: *reverts }
            }
        }
    }
}

/// Data report by a test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestKindReport {
    Standard { gas: u64 },
    Fuzz { runs: usize, mean_gas: u64, median_gas: u64 },
    Invariant { runs: usize, calls: usize, reverts: usize },
}

impl fmt::Display for TestKindReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestKindReport::Standard { gas } => {
                write!(f, "(gas: {gas})")
            }
            TestKindReport::Fuzz { runs, mean_gas, median_gas } => {
                write!(f, "(runs: {runs}, μ: {mean_gas}, ~: {median_gas})")
            }
            TestKindReport::Invariant { runs, calls, reverts } => {
                write!(f, "(runs: {runs}, calls: {calls}, reverts: {reverts})")
            }
        }
    }
}

impl TestKindReport {
    /// Returns the main gas value to compare against
    pub fn gas(&self) -> u64 {
        match self {
            TestKindReport::Standard { gas } => *gas,
            // We use the median for comparisons
            TestKindReport::Fuzz { median_gas, .. } => *median_gas,
            // We return 0 since it's not applicable
            TestKindReport::Invariant { .. } => 0,
        }
    }
}

/// Various types of test environments
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TestEnvironment {
    /// A standard test environment
    Standard,
    /// A forked test environment
    Fork {
        /// The block number at which the test was executed
        block_number: U256,
    },
}

impl Default for TestEnvironment {
    fn default() -> Self {
        Self::Standard
    }
}

impl TestEnvironment {
    // The environment in which the test was run
    pub fn report(&self) -> TestEnvironmentReport {
        match self {
            TestEnvironment::Standard => TestEnvironmentReport::Standard,
            TestEnvironment::Fork { block_number } => {
                TestEnvironmentReport::Fork { block_number: *block_number }
            }
        }
    }
}

/// Environment report by a test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestEnvironmentReport {
    Standard,
    Fork { block_number: U256 },
}

impl fmt::Display for TestEnvironmentReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestEnvironmentReport::Fork { block_number } => {
                write!(f, "(block: {block_number}) ")
            }
            _ => write!(f, ""),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TestSetup {
    /// The address at which the test contract was deployed
    pub address: Address,
    /// The logs emitted during setup
    pub logs: Vec<Log>,
    /// Call traces of the setup
    pub traces: Traces,
    /// Contexts of the setup
    pub contexts: Vec<Context>,
    /// Addresses labeled during setup
    pub labeled_addresses: HashMap<Address, String>,
    /// The reason the setup failed, if it did
    pub reason: Option<String>,
    /// Coverage info during setup
    pub coverage: Option<HitMaps>,
    /// Defined fuzz test fixtures
    pub fuzz_fixtures: FuzzFixtures,
}

impl TestSetup {
    pub fn from_evm_error_with(
        error: EvmError,
        mut logs: Vec<Log>,
        mut traces: Traces,
        mut contexts: Vec<Context>,
        mut labeled_addresses: HashMap<Address, String>,
    ) -> Self {
        match error {
            EvmError::Execution(err) => {
                // force the tracekind to be setup so a trace is shown.
                traces.extend(err.raw.traces.map(|traces| (TraceKind::Setup, traces)));
                logs.extend(err.raw.logs);
                contexts.extend(err.raw.contexts);
                labeled_addresses.extend(err.raw.labels);
                Self::failed_with(logs, traces, contexts, labeled_addresses, err.reason)
            }
            e => Self::failed_with(
                logs,
                traces,
                contexts,
                labeled_addresses,
                format!("failed to deploy contract: {e}"),
            ),
        }
    }

    pub fn success(
        address: Address,
        logs: Vec<Log>,
        traces: Traces,
        contexts: Vec<Context>,
        labeled_addresses: HashMap<Address, String>,
        coverage: Option<HitMaps>,
        fuzz_fixtures: FuzzFixtures,
    ) -> Self {
        Self {
            address,
            logs,
            traces,
            contexts,
            labeled_addresses,
            reason: None,
            coverage,
            fuzz_fixtures,
        }
    }

    pub fn failed_with(
        logs: Vec<Log>,
        traces: Traces,
        contexts: Vec<Context>,
        labeled_addresses: HashMap<Address, String>,
        reason: String,
    ) -> Self {
        Self {
            address: Address::ZERO,
            logs,
            traces,
            contexts,
            labeled_addresses,
            reason: Some(reason),
            coverage: None,
            fuzz_fixtures: FuzzFixtures::default(),
        }
    }

    pub fn failed(reason: String) -> Self {
        Self { reason: Some(reason), ..Default::default() }
    }
}
