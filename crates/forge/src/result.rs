//! Test outcomes.

use crate::{
    fuzz::{BaseCounterExample, BasicTxDetails},
    gas_report::GasReport,
};
use alloy_primitives::{
    Address, Bytes, I256, Log, Selector, U256,
    map::{AddressHashMap, HashMap},
};
use eyre::Report;
use foundry_common::{ContractsByArtifact, get_contract_name, get_file_name, shell};
use foundry_config::{SymbolicConfig, SymbolicExplorationOrder, SymbolicStorageLayout};
use foundry_evm::{
    core::{Breakpoints, evm::FoundryEvmNetwork},
    coverage::HitMaps,
    decode::SkipReason,
    executors::{RawCallResult, invariant::InvariantMetrics},
    fuzz::{CallDetails, CounterExample, FuzzCase, FuzzFixtures, FuzzTestResult},
    traces::{CallTraceArena, CallTraceDecoder, TraceKind, Traces},
};
use foundry_evm_symbolic::{PortfolioDiagnostics, SymbolicStats, SymbolicStopReason};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap as Map},
    fmt::{self, Write},
    time::Duration,
};
use yansi::Paint;

pub(crate) fn invariant_campaign_display_name(contract_name: &str) -> String {
    format!("{contract_name} invariants")
}

const INVARIANT_CAMPAIGN_FALLBACK_NAME: &str = "Invariant campaign";
const SYMBOLIC_RESULT_SCHEMA_VERSION: u32 = 1;
pub const SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA: &str = "foundry:symbolic.counterexample@v1";
pub const SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA_VERSION: u32 = 1;

const fn symbolic_result_schema_version() -> u32 {
    SYMBOLIC_RESULT_SCHEMA_VERSION
}

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

    /// Returns merged symbolic solver portfolio diagnostics across all tests in this outcome.
    pub fn symbolic_portfolio_diagnostics(&self) -> Option<PortfolioDiagnostics> {
        let mut diagnostics = PortfolioDiagnostics::default();
        for (_, result) in self.tests() {
            if let Some(result_diagnostics) = &result.symbolic_portfolio_diagnostics {
                diagnostics.merge(result_diagnostics);
            }
        }
        (!diagnostics.is_empty()).then_some(diagnostics)
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

    /// Returns `true` if any invariant test failed.
    pub fn has_invariant_failures(&self) -> bool {
        self.failures().any(|(_, t)| t.kind.is_invariant())
    }

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
    //
    // Exit-code policy: under `--machine` we honor the agent contract
    // ([`ExitCode::TestFailure`]); legacy invocations preserve the
    // historical exit-1 contract that scripts and CIs already depend on.
    pub fn ensure_ok(&self, silent: bool) -> eyre::Result<()> {
        let outcome = self;
        let failures = outcome.failures().count();
        if outcome.allow_failure || failures == 0 {
            return Ok(());
        }

        if shell::is_quiet() || silent {
            std::process::exit(test_failure_exit_code());
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
                sh_println!("{}", result.short_result_with_suite(name, suite_name))?;
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
            if let Some(invariant_workers) = outcome.invariant_workers_hint() {
                sh_println!(
                    "Invariant workers: {} (use {} to reproduce)",
                    invariant_workers,
                    format!("`--invariant-workers {invariant_workers}`").cyan()
                )?;
            }
        }

        std::process::exit(test_failure_exit_code());
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

/// Process exit code emitted when at least one test failed.
fn test_failure_exit_code() -> i32 {
    if foundry_cli::is_machine() { foundry_cli::ExitCode::TestFailure.to_i32() } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYMBOLIC_RESULT_SCHEMA_JSON: &str =
        include_str!("../../evm/symbolic/assets/symbolic-result.schema.json");
    const SYMBOLIC_COUNTEREXAMPLE_SCHEMA_JSON: &str =
        include_str!("../../evm/symbolic/assets/symbolic-counterexample.schema.json");

    fn schema_defs(schema: &serde_json::Value) -> &serde_json::Map<String, serde_json::Value> {
        schema["$defs"].as_object().expect("schema $defs object")
    }

    fn assert_counterexample_schema_refs_resolve_offline() {
        let counterexample_schema: serde_json::Value =
            serde_json::from_str(SYMBOLIC_COUNTEREXAMPLE_SCHEMA_JSON).unwrap();
        let result_schema: serde_json::Value =
            serde_json::from_str(SYMBOLIC_RESULT_SCHEMA_JSON).unwrap();
        let result_defs = schema_defs(&result_schema);
        let counterexample_defs = schema_defs(&counterexample_schema);

        fn visit_refs(
            value: &serde_json::Value,
            result_defs: &serde_json::Map<String, serde_json::Value>,
            counterexample_defs: &serde_json::Map<String, serde_json::Value>,
        ) {
            match value {
                serde_json::Value::Object(map) => {
                    if let Some(reference) = map.get("$ref").and_then(serde_json::Value::as_str) {
                        if let Some(name) = reference.strip_prefix(
                            "https://foundry-rs.github.io/schemas/symbolic-result.v1.schema.json#/$defs/",
                        ) {
                            assert!(result_defs.contains_key(name), "unresolved ref {reference}");
                        } else if let Some(name) = reference.strip_prefix("#/$defs/") {
                            assert!(
                                counterexample_defs.contains_key(name),
                                "unresolved ref {reference}"
                            );
                        } else {
                            panic!("unexpected schema ref {reference}");
                        }
                    }
                    for child in map.values() {
                        visit_refs(child, result_defs, counterexample_defs);
                    }
                }
                serde_json::Value::Array(values) => {
                    for child in values {
                        visit_refs(child, result_defs, counterexample_defs);
                    }
                }
                _ => {}
            }
        }

        visit_refs(&counterexample_schema, result_defs, counterexample_defs);
    }

    fn assert_counterexample_artifact_shape(value: &serde_json::Value) {
        assert_counterexample_schema_refs_resolve_offline();
        let object = value.as_object().expect("artifact object");
        for key in [
            "schema_version",
            "schema",
            "kind",
            "test",
            "replay",
            "replay_semantics",
            "bounds",
            "solver",
            "assumptions",
            "call_trace",
            "calls",
        ] {
            assert!(object.contains_key(key), "missing required artifact key {key}");
        }
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["schema"], SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA);
        assert!(matches!(value["kind"].as_str(), Some("single_call" | "sequence")));
        assert!(value["replay_semantics"].is_object());
        assert!(!value["calls"].as_array().expect("calls array").is_empty());
        for call in value["calls"].as_array().unwrap() {
            let call = call.as_object().expect("call object");
            for key in [
                "warp",
                "roll",
                "sender",
                "target",
                "calldata",
                "value",
                "contract_name",
                "function_name",
                "signature",
                "args",
                "raw_args",
            ] {
                assert!(call.contains_key(key), "missing required call key {key}");
            }
            for key in ["warp", "roll", "value"] {
                let Some(encoded) = call[key].as_str() else { continue };
                let Some(hex) = encoded.strip_prefix("0x") else {
                    panic!("{key} must be 0x-prefixed hex quantity: {encoded}");
                };
                assert!(
                    hex == "0" || !hex.starts_with('0'),
                    "{key} must be compact hex quantity without leading zeros: {encoded}"
                );
                assert!(
                    hex.bytes().all(|byte| byte.is_ascii_hexdigit()),
                    "{key} must be hex quantity: {encoded}"
                );
            }
        }
    }

    fn outcome_with_failed_invariant_workers(workers: &[usize]) -> TestOutcome {
        let test_results = workers
            .iter()
            .enumerate()
            .map(|(idx, workers)| {
                (
                    format!("invariant{idx}()"),
                    TestResult {
                        status: TestStatus::Failure,
                        kind: TestKind::Invariant {
                            runs: 0,
                            calls: 0,
                            reverts: 0,
                            workers: *workers,
                            metrics: Map::new(),
                            failed_corpus_replays: 0,
                            optimization_best_value: None,
                        },
                        ..Default::default()
                    },
                )
            })
            .collect();
        TestOutcome::new(
            None,
            BTreeMap::from([(
                "suite".to_string(),
                SuiteResult::new(Duration::ZERO, test_results, Vec::new()),
            )]),
            false,
            None,
        )
    }

    #[test]
    fn invariant_workers_hint_requires_matching_parallel_worker_counts() {
        assert_eq!(
            outcome_with_failed_invariant_workers(&[3, 3]).invariant_workers_hint(),
            Some(3)
        );
        assert_eq!(outcome_with_failed_invariant_workers(&[2, 3]).invariant_workers_hint(), None);
        assert_eq!(outcome_with_failed_invariant_workers(&[1]).invariant_workers_hint(), None);
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

    #[test]
    fn symbolic_counterexample_artifact_serializes_sequence_calls() {
        let symbolic = SymbolicResult::pass(&SymbolicConfig::default(), SymbolicStats::default());
        let call = SymbolicCounterexampleCall {
            warp: Some(U256::from(12)),
            roll: Some(U256::from(3)),
            sender: Address::ZERO,
            target: Address::ZERO,
            calldata: Bytes::from_static(&[0x12, 0x34, 0x56, 0x78]),
            value: Some(U256::from(9)),
            contract_name: Some("Target".to_string()),
            function_name: Some("step".to_string()),
            signature: Some("step()".to_string()),
            args: Some(String::new()),
            raw_args: Some(String::new()),
        };
        let artifact = SymbolicCounterexampleArtifact::new(
            SymbolicCounterexampleArtifactKind::Sequence,
            SymbolicCounterexampleTestIdentity {
                contract: "InvariantTest".to_string(),
                test: "invariant_counter()".to_string(),
            },
            &symbolic,
            SymbolicCounterexampleReplaySemantics { fail_on_revert: false },
            vec![call.clone(), call],
        );

        let value = serde_json::to_value(artifact).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["schema"], SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA);
        assert_eq!(value["kind"], "sequence");
        assert_eq!(value["replay_semantics"]["fail_on_revert"], false);
        assert_eq!(value["calls"].as_array().unwrap().len(), 2);
        assert_eq!(value["calls"][0]["calldata"], "0x12345678");
        assert_eq!(value["calls"][0]["warp"], "0xc");
        assert_eq!(value["calls"][0]["roll"], "0x3");
        assert_eq!(value["calls"][0]["value"], "0x9");
        assert_counterexample_artifact_shape(&value);
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
        /// Durable replay artifact for this counterexample, when one was written.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact: Option<SymbolicArtifactRef>,
        /// Path where the counterexample was persisted for re-running and shrinking.
        persisted_path: std::path::PathBuf,
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
    /// Durable counterexample artifact, when one was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<SymbolicArtifactRef>,
}

impl SymbolicResult {
    /// Creates a symbolic pass result.
    pub fn pass(config: &SymbolicConfig, stats: SymbolicStats) -> Self {
        Self::new(
            SymbolicResultStatus::Pass,
            config,
            stats,
            None,
            SymbolicReplayMetadata::not_required(),
            SymbolicCallTrace::none(),
            None,
        )
    }

    /// Creates a symbolic counterexample result that concrete replay confirmed.
    pub fn fail_counterexample(
        config: &SymbolicConfig,
        stats: SymbolicStats,
        call_trace: SymbolicCallTrace,
        counterexample: SymbolicCounterexample,
    ) -> Self {
        Self::new(
            SymbolicResultStatus::FailCounterexample,
            config,
            stats,
            None,
            SymbolicReplayMetadata::confirmed(),
            call_trace,
            Some(counterexample),
        )
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
        Self::new(
            SymbolicResultStatus::Incomplete,
            config,
            stats,
            Some(SymbolicIncomplete::new(kind, reason)),
            replay,
            call_trace,
            counterexample,
        )
    }

    fn new(
        status: SymbolicResultStatus,
        config: &SymbolicConfig,
        stats: SymbolicStats,
        incomplete: Option<SymbolicIncomplete>,
        replay: SymbolicReplayMetadata,
        call_trace: SymbolicCallTrace,
        counterexample: Option<SymbolicCounterexample>,
    ) -> Self {
        Self {
            schema_version: SYMBOLIC_RESULT_SCHEMA_VERSION,
            status,
            incomplete,
            bounds: SymbolicBounds::from_config(config),
            solver: SymbolicSolverMetadata::from_config_and_stats(config, stats),
            assumptions: SymbolicAssumption::default_assumptions(),
            call_trace,
            replay,
            counterexample,
            artifact: None,
        }
    }

    /// Attaches a durable replay artifact reference to this symbolic result.
    pub fn with_artifact(mut self, artifact: SymbolicArtifactRef) -> Self {
        self.artifact = Some(artifact);
        self
    }
}

/// Reference to a durable symbolic counterexample artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolicArtifactRef {
    /// Artifact schema id.
    pub schema: String,
    /// Path to the artifact file.
    pub path: std::path::PathBuf,
}

impl SymbolicArtifactRef {
    /// Creates a reference to a symbolic counterexample artifact.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { schema: SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA.to_string(), path: path.into() }
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
        Self { kind: symbolic_stop_reason_kind(kind).to_string(), reason: reason.into() }
    }
}

const fn symbolic_stop_reason_kind(kind: SymbolicStopReason) -> &'static str {
    match kind {
        SymbolicStopReason::Stuck => "stuck",
        SymbolicStopReason::RevertAll => "revert_all",
        SymbolicStopReason::Timeout => "timeout",
        SymbolicStopReason::Error => "error",
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
    pub stats: SymbolicSolverStats,
}

impl SymbolicSolverMetadata {
    fn from_config_and_stats(config: &SymbolicConfig, stats: SymbolicStats) -> Self {
        Self {
            name: config.solver.clone(),
            command: config.solver_command.clone(),
            portfolio: config.solver_portfolio.clone(),
            stats: SymbolicSolverStats::from(stats),
        }
    }
}

/// Symbolic engine and solver counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolicSolverStats {
    /// Number of explored symbolic paths.
    pub paths: usize,
    /// Number of normalized solver queries issued during the run.
    pub solver_queries: usize,
    /// Number of queries sent to the SMT backend after local fast paths.
    pub smt_queries: usize,
    /// Number of satisfiability checks requested by the executor.
    pub sat_queries: usize,
    /// Number of concrete model requests requested by the executor.
    pub model_queries: usize,
    /// Number of satisfiability checks served from the normalized cache.
    pub sat_cache_hits: usize,
    /// Number of model requests served from the normalized model cache.
    pub model_cache_hits: usize,
    /// Number of satisfiable witnesses produced by local hard-arithmetic search.
    pub heuristic_witnesses: usize,
    /// Wall-clock time spent waiting on backend solver subprocesses, in milliseconds.
    pub solver_time_ms: u64,
}

impl From<SymbolicStats> for SymbolicSolverStats {
    fn from(stats: SymbolicStats) -> Self {
        Self {
            paths: stats.paths,
            solver_queries: stats.solver_queries,
            smt_queries: stats.smt_queries,
            sat_queries: stats.sat_queries,
            model_queries: stats.model_queries,
            sat_cache_hits: stats.sat_cache_hits,
            model_cache_hits: stats.model_cache_hits,
            heuristic_witnesses: stats.heuristic_witnesses,
            solver_time_ms: stats.solver_time_ms,
        }
    }
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
        if !available {
            return Self::none();
        }

        Self {
            available: true,
            source: Some("test_result.traces".to_string()),
            format: Some("foundry_call_trace_arena".to_string()),
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
            calls,
        }
    }
}

/// Concrete replay semantics captured when a symbolic artifact is confirmed.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
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

/// Test identity for a symbolic counterexample artifact.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolicCounterexampleTestIdentity {
    /// Contract identifier as reported by Forge.
    pub contract: String,
    /// Test function signature.
    pub test: String,
}

/// One concrete call in a symbolic counterexample artifact.
#[derive(Clone, Debug, Serialize, Deserialize)]
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

    /// Durable replay artifact for the top-level counterexample, when one was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterexample_artifact: Option<SymbolicArtifactRef>,

    /// All durable replay artifacts produced for this test result, normalized for JSON consumers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub counterexample_artifacts: Vec<SymbolicArtifactRef>,

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
        f.write_str(&self.render_status_block(false, None))
    }
}

impl TestResult {
    /// Adds a durable replay artifact to the normalized list and legacy top-level field.
    pub fn add_counterexample_artifact(&mut self, artifact: SymbolicArtifactRef) {
        if !self.counterexample_artifacts.contains(&artifact) {
            self.counterexample_artifacts.push(artifact.clone());
        }
        if self.counterexample_artifact.is_none() {
            self.counterexample_artifact = Some(artifact);
        }
    }

    fn render_status_block(
        &self,
        user_facing: bool,
        invariant_campaign_name: Option<&str>,
    ) -> String {
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
                self.write_invariant_predicate_results(
                    &mut s,
                    user_facing,
                    true,
                    invariant_campaign_name,
                );
                format!("{}", s.green().wrap())
            }
            TestStatus::Skipped => {
                let mut s = String::from("[SKIP");
                if let Some(reason) = &self.reason {
                    write!(s, ": {reason}").unwrap();
                }
                s.push(']');
                self.write_invariant_predicate_results(
                    &mut s,
                    user_facing,
                    true,
                    invariant_campaign_name,
                );
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
                    // Contract-level campaigns identify the broken predicate even when only one
                    // predicate failed. Preserve the compact legacy shape only for the anchor of a
                    // single-predicate run.
                    let multi = self.invariant_failures.len() > 1;
                    let is_campaign = self.invariant_count.is_some();
                    for (i, failure) in self.invariant_failures.iter().enumerate() {
                        if i > 0 {
                            s.push('\n');
                        }
                        let is_anchor =
                            matches!(failure, InvariantFailure::Predicate { is_anchor: true, .. });
                        let name_suffix = if is_campaign || multi || !is_anchor {
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

                let rollup_rendered = self.write_invariant_rollup(
                    &mut s,
                    user_facing,
                    is_invariant_failure,
                    invariant_campaign_name,
                );
                let show_predicate_header = if user_facing { !rollup_rendered } else { true };
                self.write_invariant_predicate_results(
                    &mut s,
                    user_facing,
                    show_predicate_header,
                    invariant_campaign_name,
                );
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
        invariant_campaign_name: Option<&str>,
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
            if user_facing {
                invariant_campaign_name.unwrap_or(INVARIANT_CAMPAIGN_FALLBACK_NAME)
            } else {
                "Predicates"
            },
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
        invariant_campaign_name: Option<&str>,
    ) {
        if self.invariant_predicate_results.len() <= 1 {
            return;
        }

        if show_header {
            s.push('\n');
            s.push_str(if user_facing {
                invariant_campaign_name.unwrap_or(INVARIANT_CAMPAIGN_FALLBACK_NAME)
            } else {
                "Predicates"
            });
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
        $a.debug_bytecodes.extend($b.debug_bytecodes);
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
            debug_bytecodes: setup.debug_bytecodes.clone(),
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
            debug_bytecodes,
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
            debug_bytecodes,
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
            workers: default_invariant_workers(),
            metrics: HashMap::default(),
            failed_corpus_replays: 0,
            optimization_best_value: None,
        };
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
        replayed_entirely: bool,
        invariant_name: &String,
        replay_reason: Option<String>,
        call_sequence: Vec<BaseCounterExample>,
    ) {
        self.kind = TestKind::Invariant {
            runs: 1,
            calls: 1,
            reverts: 1,
            workers: default_invariant_workers(),
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
            workers: default_invariant_workers(),
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
        runs: usize,
        calls: usize,
        reverts: usize,
        metrics: Map<String, InvariantMetrics>,
        failed_corpus_replays: usize,
        workers: usize,
        optimization_best_value: Option<I256>,
    ) {
        self.kind = TestKind::Invariant {
            runs,
            calls,
            reverts,
            workers: workers.max(1),
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
        let artifacts = self
            .invariant_failures
            .iter()
            .chain(&self.invariant_handler_failures)
            .filter_map(InvariantFailure::artifact)
            .cloned()
            .collect::<Vec<_>>();
        for artifact in artifacts {
            self.add_counterexample_artifact(artifact);
        }
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

    /// Returns the result for a symbolic test.
    pub fn symbolic_result(
        &mut self,
        status: TestStatus,
        reason: Option<String>,
        counterexample: Option<CounterExample>,
        symbolic: SymbolicResult,
    ) {
        let stats = symbolic.solver.stats;
        self.kind = TestKind::Symbolic {
            paths: stats.paths,
            solver_queries: stats.solver_queries,
            smt_queries: stats.smt_queries,
            sat_queries: stats.sat_queries,
            model_queries: stats.model_queries,
            sat_cache_hits: stats.sat_cache_hits,
            model_cache_hits: stats.model_cache_hits,
            heuristic_witnesses: stats.heuristic_witnesses,
            solver_time_ms: stats.solver_time_ms,
        };
        self.status = status;
        self.reason = reason;
        self.counterexample = counterexample;
        if let Some(artifact) = symbolic.artifact.clone() {
            self.add_counterexample_artifact(artifact);
        }
        self.symbolic = Some(symbolic);
        self.duration = Duration::default();
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

    /// Returns `true` if this is the result of a fuzz test
    pub const fn is_fuzz(&self) -> bool {
        matches!(self.kind, TestKind::Fuzz { .. })
    }

    /// Formats the test result into a string (for printing).
    pub fn short_result(&self, name: &str) -> String {
        self.short_result_with_campaign_name(name, None)
    }

    pub(crate) fn short_result_with_suite(&self, name: &str, suite_name: &str) -> String {
        self.short_result_with_campaign_name(name, Some(get_contract_name(suite_name)))
    }

    fn short_result_with_campaign_name(&self, name: &str, contract_name: Option<&str>) -> String {
        let is_invariant_campaign = self.is_invariant_campaign();
        let name = if is_invariant_campaign {
            contract_name
                .map(invariant_campaign_display_name)
                .map(Cow::Owned)
                .unwrap_or(Cow::Borrowed(INVARIANT_CAMPAIGN_FALLBACK_NAME))
        } else {
            Cow::Borrowed(name)
        };
        let status = self.render_status_block(true, is_invariant_campaign.then_some(name.as_ref()));
        format!("{status} {name} {}", self.kind.report())
    }

    const fn is_invariant_campaign(&self) -> bool {
        self.kind.is_invariant() && self.invariant_count.is_some()
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
    Symbolic {
        paths: usize,
        solver_queries: usize,
        smt_queries: usize,
        sat_queries: usize,
        model_queries: usize,
        sat_cache_hits: usize,
        model_cache_hits: usize,
        heuristic_witnesses: usize,
        solver_time_ms: u64,
    },
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
            Self::Symbolic {
                paths,
                solver_queries,
                smt_queries,
                sat_queries,
                model_queries,
                sat_cache_hits,
                model_cache_hits,
                heuristic_witnesses,
                solver_time_ms,
            } => {
                write!(
                    f,
                    "(paths: {paths}, queries: {solver_queries}, smt: {smt_queries}, sat: {sat_queries} ({sat_cache_hits} cached), models: {model_queries} ({model_cache_hits} cached), hard-arith: {heuristic_witnesses}, solver: {solver_time_ms}ms)"
                )
            }
            Self::Replay { corpus_entries, showmap_files, skipped_entries } => {
                if *skipped_entries != 0 {
                    write!(
                        f,
                        "(replay: {corpus_entries} entries, {showmap_files} files, {skipped_entries} skipped)"
                    )
                } else {
                    write!(f, "(replay: {corpus_entries} entries, {showmap_files} files)")
                }
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
    Symbolic {
        paths: usize,
        solver_queries: usize,
        #[serde(default)]
        smt_queries: usize,
        #[serde(default)]
        sat_queries: usize,
        #[serde(default)]
        model_queries: usize,
        #[serde(default)]
        sat_cache_hits: usize,
        #[serde(default)]
        model_cache_hits: usize,
        #[serde(default)]
        heuristic_witnesses: usize,
        #[serde(default)]
        solver_time_ms: u64,
    },
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

    /// Actual invariant campaign worker count, if this is an invariant test.
    pub const fn invariant_workers(&self) -> Option<usize> {
        match self {
            Self::Invariant { workers, .. } => Some(*workers),
            _ => None,
        }
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
                workers: _,
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
            Self::Symbolic {
                paths,
                solver_queries,
                smt_queries,
                sat_queries,
                model_queries,
                sat_cache_hits,
                model_cache_hits,
                heuristic_witnesses,
                solver_time_ms,
            } => TestKindReport::Symbolic {
                paths: *paths,
                solver_queries: *solver_queries,
                smt_queries: *smt_queries,
                sat_queries: *sat_queries,
                model_queries: *model_queries,
                sat_cache_hits: *sat_cache_hits,
                model_cache_hits: *model_cache_hits,
                heuristic_witnesses: *heuristic_witnesses,
                solver_time_ms: *solver_time_ms,
            },
            Self::Replay { corpus_entries, showmap_files, skipped_entries } => {
                TestKindReport::Replay {
                    corpus_entries: *corpus_entries,
                    showmap_files: *showmap_files,
                    skipped_entries: *skipped_entries,
                }
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
