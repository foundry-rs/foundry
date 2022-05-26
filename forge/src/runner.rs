use crate::TestFilter;
use ethers::{
    abi::{Abi, Function},
    prelude::ArtifactId,
    types::{Address, Bytes, Log, U256},
};
use eyre::Result;
use foundry_evm::{
    executor::{CallResult, DatabaseRef, DeployResult, EvmError, Executor},
    fuzz::{
        invariant::{InvariantExecutor, InvariantFuzzTestResult},
        CounterExample, FuzzedCases, FuzzedExecutor,
    },
    trace::{load_contracts, CallTraceArena, TraceKind},
    CALLER,
};
use proptest::test_runner::{TestError, TestRunner};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt,
    time::{Duration, Instant},
};

/// Metadata on how to run fuzz/invariant tests
#[derive(Clone, Copy)]
pub struct TestOptions {
    /// Whether fuzz tests should be run
    pub include_fuzz_tests: bool,
    /// The number of calls executed to attempt to break invariants
    pub invariant_depth: u32,
    /// Fails the invariant fuzzing if a reversion occurs
    pub invariant_fail_on_revert: bool,
}

/// Results and duration for a set of tests included in the same test contract
#[derive(Clone, Serialize)]
pub struct SuiteResult {
    /// Total duration of the test run for this block of tests
    pub duration: Duration,
    /// Individual test results. `test method name -> TestResult`
    pub test_results: BTreeMap<String, TestResult>,
    // Warnings
    pub warnings: Vec<String>,
}

impl SuiteResult {
    pub fn new(
        duration: Duration,
        test_results: BTreeMap<String, TestResult>,
        warnings: Vec<String>,
    ) -> Self {
        Self { duration, test_results, warnings }
    }

    pub fn is_empty(&self) -> bool {
        self.test_results.is_empty()
    }

    pub fn len(&self) -> usize {
        self.test_results.len()
    }
}

/// The result of an executed solidity test
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestResult {
    /// Whether the test case was successful. This means that the transaction executed
    /// properly, or that there was a revert and that the test was expected to fail
    /// (prefixed with `testFail`)
    pub success: bool,

    /// If there was a revert, this field will be populated. Note that the test can
    /// still be successful (i.e self.success == true) when it's expected to fail.
    pub reason: Option<String>,

    /// Minimal reproduction test case for failing fuzz tests
    pub counterexample: Option<CounterExample>,

    /// Minimal reproduction sequence for failing invariant test
    pub counterexample_sequence: Option<Vec<CounterExample>>,

    /// Any captured & parsed as strings logs along the test's execution which should
    /// be printed to the user.
    #[serde(skip)]
    pub logs: Vec<Log>,

    /// What kind of test this was
    pub kind: TestKind,

    /// Traces
    pub traces: Vec<(TraceKind, CallTraceArena)>,

    /// Labeled addresses
    pub labeled_addresses: BTreeMap<Address, String>,
}

impl TestResult {
    /// Returns `true` if this is the result of a fuzz test
    pub fn is_fuzz(&self) -> bool {
        matches!(self.kind, TestKind::Fuzz(_))
    }
}

/// Used gas by a test
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum TestKindGas {
    Standard(u64),
    Fuzz { runs: usize, mean: u64, median: u64 },
    Invariant { runs: usize, calls: usize, reverts: usize, mean: u64, median: u64 },
}

impl fmt::Display for TestKindGas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestKindGas::Standard(gas) => {
                write!(f, "(gas: {})", gas)
            }
            TestKindGas::Fuzz { runs, mean, median } => {
                write!(f, "(runs: {}, μ: {}, ~: {})", runs, mean, median)
            }
            TestKindGas::Invariant { runs, calls, reverts, mean, median } => {
                write!(
                    f,
                    "(runs: {}, calls: {}, reverts: {}, μ: {}, ~: {})",
                    runs, calls, reverts, mean, median
                )
            }
        }
    }
}

impl TestKindGas {
    /// Returns the main gas value to compare against
    pub fn gas(&self) -> u64 {
        match self {
            TestKindGas::Standard(gas) => *gas,
            // We use the median for comparisons
            TestKindGas::Fuzz { median, .. } => *median,
            TestKindGas::Invariant { median, .. } => *median,
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
    Fuzz(FuzzedCases),
    /// Invariant
    Invariant(Vec<FuzzedCases>, usize),
}

impl TestKind {
    /// The gas consumed by this test
    pub fn gas_used(&self) -> TestKindGas {
        match self {
            TestKind::Standard(gas) => TestKindGas::Standard(*gas),
            TestKind::Fuzz(fuzzed) => TestKindGas::Fuzz {
                runs: fuzzed.cases().len(),
                median: fuzzed.median_gas(false),
                mean: fuzzed.mean_gas(false),
            },
            TestKind::Invariant(fuzzed, reverts) => TestKindGas::Invariant {
                runs: fuzzed.len(),
                calls: fuzzed.iter().map(|sequence| sequence.cases().len()).sum(),
                // todo
                median: fuzzed.get(0).unwrap().median_gas(false),
                mean: fuzzed.get(0).unwrap().mean_gas(false),
                reverts: *reverts,
            },
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
    pub traces: Vec<(TraceKind, CallTraceArena)>,
    /// Addresses labeled during setup
    pub labeled_addresses: BTreeMap<Address, String>,
    /// Whether the setup failed
    pub setup_failed: bool,
    /// The reason the setup failed
    pub reason: Option<String>,
}

pub struct ContractRunner<'a, DB: DatabaseRef + Clone> {
    /// The executor used by the runner.
    pub executor: Executor<DB>,

    /// Library contracts to be deployed before the test contract
    pub predeploy_libs: &'a [Bytes],
    /// The deployed contract's code
    pub code: Bytes,
    /// The test contract's ABI
    pub contract: &'a Abi,
    /// All known errors, used to decode reverts
    pub errors: Option<&'a Abi>,

    /// The initial balance of the test contract
    pub initial_balance: U256,
    /// The address which will be used as the `from` field in all EVM calls
    pub sender: Address,
}

impl<'a, DB: DatabaseRef + Clone> ContractRunner<'a, DB> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        executor: Executor<DB>,
        contract: &'a Abi,
        code: Bytes,
        initial_balance: U256,
        sender: Option<Address>,
        errors: Option<&'a Abi>,
        predeploy_libs: &'a [Bytes],
    ) -> Self {
        Self {
            executor,
            contract,
            code,
            initial_balance,
            sender: sender.unwrap_or_default(),
            errors,
            predeploy_libs,
        }
    }
}

impl<'a, DB: DatabaseRef + Send + Sync + Clone> ContractRunner<'a, DB> {
    /// Deploys the test contract inside the runner from the sending account, and optionally runs
    /// the `setUp` function on the test contract.
    pub fn setup(&mut self, setup: bool) -> Result<TestSetup> {
        // We max out their balance so that they can deploy and make calls.
        self.executor.set_balance(self.sender, U256::MAX);
        self.executor.set_balance(*CALLER, U256::MAX);

        // We set the nonce of the deployer accounts to 1 to get the same addresses as DappTools
        self.executor.set_nonce(self.sender, 1);

        // Deploy libraries
        let mut traces: Vec<(TraceKind, CallTraceArena)> = vec![];
        for code in self.predeploy_libs.iter() {
            match self.executor.deploy(self.sender, code.0.clone(), 0u32.into(), self.errors) {
                Ok(DeployResult { traces: tmp_traces, .. }) => {
                    if let Some(tmp_traces) = tmp_traces {
                        traces.push((TraceKind::Deployment, tmp_traces));
                    }
                }
                Err(EvmError::Execution { reason, traces, logs, labels, .. }) => {
                    // If we failed to call the constructor, force the tracekind to be setup so
                    // a trace is shown.
                    let traces = if let Some(traces) = traces {
                        vec![(TraceKind::Setup, traces)]
                    } else {
                        vec![]
                    };

                    return Ok(TestSetup {
                        address: Address::zero(),
                        logs,
                        traces,
                        labeled_addresses: labels,
                        setup_failed: true,
                        reason: Some(reason),
                    })
                }
                e => eyre::bail!("Unrecoverable error: {:?}", e),
            }
        }

        // Deploy an instance of the contract
        let DeployResult { address, mut logs, traces: constructor_traces, .. } = match self
            .executor
            .deploy(self.sender, self.code.0.clone(), 0u32.into(), self.errors)
        {
            Ok(d) => d,
            Err(EvmError::Execution { reason, traces, logs, labels, .. }) => {
                let traces = if let Some(traces) = traces {
                    vec![(TraceKind::Setup, traces)]
                } else {
                    vec![]
                };

                return Ok(TestSetup {
                    address: Address::zero(),
                    logs,
                    traces,
                    labeled_addresses: labels,
                    setup_failed: true,
                    reason: Some(reason),
                })
            }
            e => eyre::bail!("Unrecoverable error: {:?}", e),
        };

        traces.extend(constructor_traces.map(|traces| (TraceKind::Deployment, traces)).into_iter());

        // Now we set the contracts initial balance, and we also reset `self.sender`s balance to
        // the initial balance we want
        self.executor.set_balance(address, self.initial_balance);
        self.executor.set_balance(self.sender, self.initial_balance);

        // Optionally call the `setUp` function
        Ok(if setup {
            tracing::trace!("setting up");
            let (setup_failed, setup_logs, setup_traces, labeled_addresses, reason) = match self
                .executor
                .setup(address)
            {
                Ok(CallResult { traces, labels, logs, .. }) => (false, logs, traces, labels, None),
                Err(EvmError::Execution { traces, labels, logs, reason, .. }) => {
                    (true, logs, traces, labels, Some(format!("Setup failed: {reason}")))
                }
                Err(e) => (
                    true,
                    Vec::new(),
                    None,
                    BTreeMap::new(),
                    Some(format!("Setup failed: {}", &e.to_string())),
                ),
            };
            traces.extend(setup_traces.map(|traces| (TraceKind::Setup, traces)).into_iter());
            logs.extend_from_slice(&setup_logs);

            TestSetup { address, logs, traces, labeled_addresses, setup_failed, reason }
        } else {
            TestSetup { address, logs, traces, ..Default::default() }
        })
    }

    /// Runs all tests for a contract whose names match the provided regular expression
    pub fn run_tests(
        &mut self,
        filter: &impl TestFilter,
        fuzzer: Option<TestRunner>,
        test_options: TestOptions,
        known_contracts: Option<&BTreeMap<ArtifactId, (Abi, Vec<u8>)>>,
    ) -> Result<SuiteResult> {
        tracing::info!("starting tests");
        let start = Instant::now();
        let mut warnings = Vec::new();

        let setup_fns: Vec<_> =
            self.contract.functions().filter(|func| func.name.to_lowercase() == "setup").collect();

        let needs_setup = setup_fns.len() == 1 && setup_fns[0].name == "setUp";

        // There is a single miss-cased `setUp` function, so we add a warning
        for setup_fn in setup_fns.iter() {
            if setup_fn.name != "setUp" {
                warnings.push(format!(
                    "Found invalid setup function \"{}\" did you mean \"setUp()\"?",
                    setup_fn.signature()
                ));
            }
        }

        // There are multiple setUp function, so we return a single test result for `setUp`
        if setup_fns.len() > 1 {
            return Ok(SuiteResult::new(
                start.elapsed(),
                [(
                    "setUp()".to_string(),
                    TestResult {
                        success: false,
                        reason: Some("Multiple setUp functions".to_string()),
                        counterexample: None,
                        counterexample_sequence: None,
                        logs: vec![],
                        kind: TestKind::Standard(0),
                        traces: vec![],
                        labeled_addresses: BTreeMap::new(),
                    },
                )]
                .into(),
                warnings,
            ))
        }

        let has_invariants =
            self.contract.functions().into_iter().any(|func| func.name.starts_with("invariant"));

        if has_invariants && needs_setup {
            // invariant testing requires tracing to figure
            //   out what contracts were created
            self.executor.set_tracing(true);
        }

        let setup = self.setup(needs_setup)?;
        if setup.setup_failed {
            // The setup failed, so we return a single test result for `setUp`
            return Ok(SuiteResult::new(
                start.elapsed(),
                [(
                    "setUp()".to_string(),
                    TestResult {
                        success: false,
                        reason: setup.reason,
                        counterexample: None,
                        counterexample_sequence: None,
                        logs: setup.logs,
                        kind: TestKind::Standard(0),
                        traces: setup.traces,
                        labeled_addresses: setup.labeled_addresses,
                    },
                )]
                .into(),
                warnings,
            ))
        }

        // Collect valid test functions
        let tests: Vec<_> = self
            .contract
            .functions()
            .into_iter()
            .filter(|func| {
                func.name.starts_with("test") &&
                    filter.matches_test(func.signature()) &&
                    (test_options.include_fuzz_tests || func.inputs.is_empty())
            })
            .map(|func| (func, func.name.starts_with("testFail")))
            .collect();

        let mut test_results = tests
            .par_iter()
            .filter_map(|(func, should_fail)| {
                let result = if func.inputs.is_empty() {
                    Some(self.run_test(func, *should_fail, setup.clone()))
                } else {
                    fuzzer.as_ref().map(|fuzzer| {
                        self.run_fuzz_test(func, *should_fail, fuzzer.clone(), setup.clone())
                    })
                };

                result.map(|result| Ok((func.signature(), result?)))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;

        if has_invariants && test_options.include_fuzz_tests && fuzzer.is_some() {
            let identified_contracts = load_contracts(setup.traces.clone(), known_contracts);
            let functions: Vec<&Function> = self
                .contract
                .functions()
                .into_iter()
                .filter(|func| {
                    func.name.starts_with("invariant") && filter.matches_test(func.signature())
                })
                .collect();

            let results = self.run_invariant_test(
                fuzzer.expect("no fuzzer"),
                setup,
                test_options,
                functions.clone(),
                known_contracts,
                identified_contracts,
            )?;

            results.into_iter().zip(functions.iter()).for_each(|(result, function)| {
                match result.kind {
                    TestKind::Invariant(ref _cases, _) => {
                        test_results.insert(function.name.clone(), result);
                    }
                    _ => unreachable!(),
                }
            });
        }

        let duration = start.elapsed();
        if !test_results.is_empty() {
            let successful = test_results.iter().filter(|(_, tst)| tst.success).count();
            tracing::info!(
                duration = ?duration,
                "done. {}/{} successful",
                successful,
                test_results.len()
            );
        }

        Ok(SuiteResult::new(duration, test_results, warnings))
    }

    #[tracing::instrument(name = "test", skip_all, fields(name = %func.signature(), %should_fail))]
    pub fn run_test(
        &self,
        func: &Function,
        should_fail: bool,
        setup: TestSetup,
    ) -> Result<TestResult> {
        let TestSetup { address, mut logs, mut traces, mut labeled_addresses, .. } = setup;

        // Run unit test
        let start = Instant::now();
        let (reverted, reason, gas, stipend, execution_traces, state_changeset) = match self
            .executor
            .call::<(), _, _>(self.sender, address, func.clone(), (), 0.into(), self.errors)
        {
            Ok(CallResult {
                reverted,
                gas,
                stipend,
                logs: execution_logs,
                traces: execution_trace,
                labels: new_labels,
                state_changeset,
                ..
            }) => {
                labeled_addresses.extend(new_labels);
                logs.extend(execution_logs);
                (reverted, None, gas, stipend, execution_trace, state_changeset)
            }
            Err(EvmError::Execution {
                reverted,
                reason,
                gas,
                stipend,
                logs: execution_logs,
                traces: execution_trace,
                labels: new_labels,
                state_changeset,
                ..
            }) => {
                labeled_addresses.extend(new_labels);
                logs.extend(execution_logs);
                (reverted, Some(reason), gas, stipend, execution_trace, state_changeset)
            }
            Err(err) => {
                tracing::error!(?err);
                return Err(err.into())
            }
        };
        traces.extend(execution_traces.map(|traces| (TraceKind::Execution, traces)).into_iter());

        let success = self.executor.is_success(
            setup.address,
            reverted,
            state_changeset.expect("we should have a state changeset"),
            should_fail,
        );

        // Record test execution time
        tracing::debug!(
            duration = ?start.elapsed(),
            %success,
            %gas
        );

        Ok(TestResult {
            success,
            reason,
            counterexample: None,
            counterexample_sequence: None,
            logs,
            kind: TestKind::Standard(gas.overflowing_sub(stipend).0),
            traces,
            labeled_addresses,
        })
    }

    #[tracing::instrument(name = "invariant-test", skip_all)]
    pub fn run_invariant_test(
        &mut self,
        runner: TestRunner,
        setup: TestSetup,
        test_options: TestOptions,
        functions: Vec<&Function>,
        known_contracts: Option<&BTreeMap<ArtifactId, (Abi, Vec<u8>)>>,
        identified_contracts: BTreeMap<Address, (String, Abi)>,
    ) -> Result<Vec<TestResult>> {
        let empty = BTreeMap::new();
        let project_contracts = known_contracts.unwrap_or(&empty);
        let TestSetup { address, logs, traces, labeled_addresses, .. } = setup;

        let start = Instant::now();
        let prev_db = self.executor.db.clone();
        let mut evm = InvariantExecutor::new(
            &mut self.executor,
            runner,
            self.sender,
            &identified_contracts,
            project_contracts,
        );

        if let Some(InvariantFuzzTestResult { invariants, cases, reverts }) = evm.invariant_fuzz(
            functions,
            address,
            self.contract,
            test_options.invariant_depth as usize,
            test_options.invariant_fail_on_revert,
        )? {
            let results = invariants
                .iter()
                .map(|(_k, test_error)| {
                    let mut counterexample_sequence = vec![];
                    let mut logs = logs.clone();
                    let mut traces = traces.clone();

                    if let Some(ref error) = test_error {
                        // we want traces for a failed fuzz
                        let mut ided_contracts = identified_contracts.clone();
                        if let TestError::Fail(_reason, vec_addr_bytes) = &error.test_error {
                            // Reset DB state
                            self.executor.db = prev_db.clone();
                            self.executor.set_tracing(true);

                            for (sender, (addr, bytes)) in vec_addr_bytes.iter() {
                                let call_result = self
                                    .executor
                                    .call_raw_committing(*sender, *addr, bytes.0.clone(), 0.into())
                                    .expect("bad call to evm");

                                logs.extend(call_result.logs);
                                traces.push((
                                    TraceKind::Execution,
                                    call_result.traces.clone().unwrap(),
                                ));

                                // In case the call created more.
                                ided_contracts.extend(load_contracts(
                                    vec![(TraceKind::Execution, call_result.traces.unwrap())],
                                    known_contracts,
                                ));
                                counterexample_sequence.push(CounterExample::create(
                                    *sender,
                                    *addr,
                                    bytes,
                                    &ided_contracts,
                                ));

                                if let Some(func) = &error.func {
                                    let error_call_result = self
                                        .executor
                                        .call_raw(self.sender, error.addr, func.0.clone(), 0.into())
                                        .expect("bad call to evm");

                                    if error_call_result.reverted {
                                        logs.extend(error_call_result.logs);
                                        traces.push((
                                            TraceKind::Execution,
                                            error_call_result.traces.unwrap(),
                                        ));
                                        break
                                    }
                                }
                            }
                        }
                    }

                    let success = test_error.is_none();
                    let mut reason = None;

                    if let Some(err) = test_error {
                        if !err.revert_reason.is_empty() {
                            reason = Some(err.revert_reason.clone());
                        }
                    }

                    let duration = Instant::now().duration_since(start);
                    tracing::debug!(?duration, %success);

                    let sequence = if !counterexample_sequence.is_empty() {
                        Some(counterexample_sequence)
                    } else {
                        None
                    };

                    TestResult {
                        success,
                        reason,
                        counterexample: None,
                        counterexample_sequence: sequence,
                        logs,
                        kind: TestKind::Invariant(cases.clone(), reverts),
                        traces,
                        labeled_addresses: labeled_addresses.clone(),
                    }
                })
                .collect();

            // Final clean-up
            self.executor.db = prev_db;

            Ok(results)
        } else {
            Ok(vec![])
        }
    }

    #[tracing::instrument(name = "fuzz-test", skip_all, fields(name = %func.signature(), %should_fail))]
    pub fn run_fuzz_test(
        &self,
        func: &Function,
        should_fail: bool,
        runner: TestRunner,
        setup: TestSetup,
    ) -> Result<TestResult> {
        let TestSetup { address, mut logs, mut traces, mut labeled_addresses, .. } = setup;

        // Run fuzz test
        let start = Instant::now();
        let mut result = FuzzedExecutor::new(&self.executor, runner, self.sender).fuzz(
            func,
            address,
            should_fail,
            self.errors,
        );

        // Record logs, labels and traces
        logs.append(&mut result.logs);
        labeled_addresses.append(&mut result.labeled_addresses);
        traces.extend(result.traces.map(|traces| (TraceKind::Execution, traces)).into_iter());

        // Record test execution time
        tracing::debug!(
            duration = ?start.elapsed(),
            success = %result.success
        );

        Ok(TestResult {
            success: result.success,
            reason: result.reason,
            counterexample: result.counterexample,
            counterexample_sequence: None,
            logs,
            kind: TestKind::Fuzz(result.cases),
            traces,
            labeled_addresses,
        })
    }
}
