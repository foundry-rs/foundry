use crate::TestFilter;
use ethers::{
    abi::{Abi, Function, RawLog},
    types::{Address, Bytes, U256},
};
use eyre::Result;
use foundry_evm::{
    executor::{CallResult, DatabaseRef, DeployResult, EvmError, Executor},
    fuzz::{CounterExample, FuzzedCases, FuzzedExecutor},
    trace::{CallTraceArena, TraceKind},
    CALLER,
};
use proptest::test_runner::TestRunner;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt,
    time::{Duration, Instant},
};

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

    /// Any captured & parsed as strings logs along the test's execution which should
    /// be printed to the user.
    #[serde(skip)]
    pub logs: Vec<RawLog>,

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
    Invariant(String, FuzzedCases),
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
            TestKind::Invariant(_s, fuzzed) => TestKindGas::Fuzz {
                runs: fuzzed.cases().len(),
                median: fuzzed.median_gas(),
                mean: fuzzed.mean_gas(),
            },
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TestSetup {
    /// The address at which the test contract was deployed
    pub address: Address,
    /// The logs emitted during setup
    pub logs: Vec<RawLog>,
    /// Call traces of the setup
    pub traces: Vec<(TraceKind, CallTraceArena)>,
    /// Addresses labeled during setup
    pub labeled_addresses: BTreeMap<Address, String>,
    /// Whether the setup failed
    pub setup_failed: bool,
    /// The reason the setup failed
    pub reason: Option<String>,
}

pub struct ContractRunner<'a, DB: DatabaseRef> {
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

impl<'a, DB: DatabaseRef> ContractRunner<'a, DB> {
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

impl<'a, DB: DatabaseRef + Send + Sync> ContractRunner<'a, DB> {
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
        include_fuzz_tests: bool,
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
                    (include_fuzz_tests || func.inputs.is_empty())
            })
            .map(|func| (func, func.name.starts_with("testFail")))
            .collect();

        println!("{:?}", test_fns);
        let has_invar_fns =
            self.contract.functions().into_iter().any(|func| func.name.starts_with("invariant"));


        let test_results = tests
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

        let mut map = if let Some(ref fuzzer) = fuzzer {
            let fuzz_tests = test_fns
                .par_iter()
                .filter(|func| !func.inputs.is_empty())
                .map(|func| {
                    let result =
                        self.run_fuzz_test(func, needs_setup, fuzzer.clone(), known_contracts)?;
                    Ok((func.signature(), result))
                })
                .collect::<Result<BTreeMap<_, _>>>()?;

            let mut map = unit_tests;
            map.extend(fuzz_tests);
            map
        } else {
            unit_tests
        };

        println!("has invar {:?}", has_invar_fns);
        let map = if has_invar_fns {
            if let Some(fuzzer) = fuzzer {
                let results =
                    self.run_invariant_test(needs_setup, fuzzer.clone(), known_contracts)?;
                results.into_iter().for_each(|result| match result.kind {
                    TestKind::Invariant(ref name, ref _cases) => {
                        map.insert(name.to_string(), result);
                    }
                    _ => unreachable!(),
                });
                map
            } else {
                map
            }
        } else {
            map
        };

        if !map.is_empty() {
            let successful = map.iter().filter(|(_, tst)| tst.success).count();
            let duration = Instant::now().duration_since(start);
            tracing::info!(?duration, "done. {}/{} successful", successful, map.len());
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
            logs,
            kind: TestKind::Standard(gas.overflowing_sub(stipend).0),
            traces,
            labeled_addresses,
        })
    }

    #[tracing::instrument(name = "invariant-test", skip_all)]
    pub fn run_invariant_test(
        &self,
        setup: bool,
        runner: TestRunner,
        known_contracts: Option<&BTreeMap<String, (Abi, Vec<u8>)>>,
    ) -> Result<Vec<TestResult>> {
        println!("running invariant tests for contract");
        // do not trace in fuzztests, as it's a big performance hit
        let start = Instant::now();

        let (address, mut evm, init_logs) = self.new_sputnik_evm(true)?;

        let mut traces: Vec<CallTraceArena> = Vec::new();
        let identified_contracts: RefCell<BTreeMap<Address, (String, Abi)>> =
            RefCell::new(BTreeMap::new());

        for trace in evm.traces().iter() {
            trace.update_identified(
                0,
                known_contracts.expect("traces enabled but no identified_contracts"),
                &mut identified_contracts.borrow_mut(),
                &evm,
            );
        }

        // clear out the deployment trace
        evm.reset_traces();

        // call the setup function in each test to reset the test's state.
        if setup {
            tracing::trace!("setting up");
            match evm.setup(address) {
                Ok((_reason, _setup_logs)) => {}
                Err(e) => {
                    // if tracing is enabled, just return it as a failed test
                    // otherwise abort
                    if evm.tracing_enabled() {
                        self.update_traces_ref(
                            &mut traces,
                            &mut identified_contracts.borrow_mut(),
                            known_contracts,
                            setup,
                            false,
                            &mut evm,
                        );
                    }
                    return Ok(vec![TestResult {
                        success: false,
                        reason: Some("Setup failed: ".to_string() + &e.to_string()),
                        gas_used: 0,
                        counterexample: None,
                        logs: vec![],
                        kind: TestKind::Fuzz(FuzzedCases::new(vec![])),
                        traces: Some(traces),
                        identified_contracts: Some(identified_contracts.into_inner()),
                        debug_calls: if evm.state().debug_enabled {
                            Some(evm.debug_calls())
                        } else {
                            None
                        },
                        labeled_addresses: evm.state().labels.clone(),
                    }])
                }
            }
        }

        self.update_traces_ref(
            &mut traces,
            &mut identified_contracts.borrow_mut(),
            known_contracts,
            true,
            false,
            &mut evm,
        );

        let mut logs = init_logs;

        let prev = evm.set_tracing_enabled(false);

        // instantiate the fuzzed evm in line
        let ident = identified_contracts.clone();
        let ident = ident.borrow();
        let evm = InvariantExecutor::new(&mut evm, runner, self.sender, &ident);
        if let Some(InvariantFuzzTestResult { invariants, cases }) =
            evm.invariant_fuzz(address, Some(self.contract))
        {
            let evm = evm.into_inner();

            let _duration = Instant::now().duration_since(start);
            // tracing::debug!(?duration, %success);

            let results = invariants
                .iter()
                .map(|(k, test_error)| {
                    if let Some(ref error) = test_error {
                        // we want traces for a failed fuzz
                        if let TestError::Fail(_reason, vec_addr_bytes) = &error.test_error {
                            if prev {
                                let _ = evm.set_tracing_enabled(true);
                            }
                            for (addr, bytes) in vec_addr_bytes.iter() {
                                println!("rerunning fails {:?} {:?}", addr, hex::encode(bytes));
                                let (_retdata, status, _gas, execution_logs) = evm
                                    .call_raw(self.sender, *addr, bytes.clone(), 0.into(), false)
                                    .expect("bad call to evm");

                                if is_fail(evm, status) {
                                    logs.extend(execution_logs);
                                    // add reverted logs
                                    logs.extend(evm.all_logs());
                                } else {
                                    logs.extend(execution_logs);
                                }
                                self.update_traces_ref(
                                    &mut traces,
                                    &mut identified_contracts.borrow_mut(),
                                    known_contracts,
                                    false,
                                    true,
                                    evm,
                                );

                                let (_retdata, status, _gas, execution_logs) = evm
                                    .call_raw(
                                        self.sender,
                                        error.addr,
                                        error.func.clone(),
                                        0.into(),
                                        false,
                                    )
                                    .expect("bad call to evm");
                                if is_fail(evm, status) {
                                    logs.extend(execution_logs);
                                    // add reverted logs
                                    logs.extend(evm.all_logs());
                                    self.update_traces_ref(
                                        &mut traces,
                                        &mut identified_contracts.borrow_mut(),
                                        known_contracts,
                                        false,
                                        true,
                                        evm,
                                    );
                                    break
                                } else {
                                    logs.extend(execution_logs);
                                    self.update_traces_ref(
                                        &mut traces,
                                        &mut identified_contracts.borrow_mut(),
                                        known_contracts,
                                        false,
                                        true,
                                        evm,
                                    );
                                }
                            }
                        }
                    }

                    let success = test_error.is_none();
                    let mut counterexample = None;
                    let mut reason = None;
                    if let Some(err) = test_error {
                        match &err.test_error {
                            TestError::Fail(_, vec_addr_bytes) => {
                                let addr = vec_addr_bytes[0].0;
                                let value = &vec_addr_bytes[0].1;
                                let ident = identified_contracts.borrow();
                                let abi =
                                    &ident.get(&addr).expect("Couldnt call unknown contract").1;
                                let func = abi
                                    .functions()
                                    .find(|f| f.short_signature() == value.as_ref()[0..4])
                                    .expect("Couldnt find function");
                                // skip the function selector when decoding
                                let args = func
                                    .decode_input(&value.as_ref()[4..])
                                    .expect("Unable to decode input");
                                let counter = CounterExample {
                                    addr: Some(addr),
                                    calldata: value.clone(),
                                    args,
                                };
                                counterexample = Some(counter);
                                tracing::info!(
                                    "Found minimal failing case: {}",
                                    hex::encode(&value)
                                );
                            }
                            result => panic!("Unexpected test result: {:?}", result),
                        }
                        if !err.revert_reason.is_empty() {
                            reason = Some(err.revert_reason.clone());
                        }
                    }

                    let duration = Instant::now().duration_since(start);
                    tracing::debug!(?duration, %success);

                    // from that call?
                    TestResult {
                        success,
                        reason,
                        gas_used: cases.median_gas(),
                        counterexample,
                        logs: logs.clone(),
                        kind: TestKind::Invariant(k.to_string(), cases.clone()),
                        traces: Some(traces.clone()),
                        identified_contracts: Some(identified_contracts.clone().into_inner()),
                        debug_calls: if evm.state().debug_enabled {
                            Some(evm.debug_calls())
                        } else {
                            None
                        },
                        labeled_addresses: evm.state().labels.clone(),
                    }
                })
                .collect();
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
            logs,
            kind: TestKind::Fuzz(result.cases),
            traces,
            labeled_addresses,
        })
    }

    fn update_traces_ref<S: Clone, E: Evm<S>>(
        &self,
        traces: &mut Vec<CallTraceArena>,
        identified_contracts: &mut BTreeMap<Address, (String, Abi)>,
        known_contracts: Option<&BTreeMap<String, (Abi, Vec<u8>)>>,
        setup: bool,
        skip: bool,
        evm: &mut E,
    ) {
        let evm_traces = evm.traces();
        if !evm_traces.is_empty() && evm.tracing_enabled() {
            let mut ident = identified_contracts.clone();
            // create an iter over the traces
            let mut trace_iter = evm_traces.into_iter();
            if setup {
                // grab the setup trace if it exists
                let setup = trace_iter.next().expect("no setup trace");
                setup.update_identified(
                    0,
                    known_contracts.expect("traces enabled but no identified_contracts"),
                    &mut ident,
                    evm,
                );
                traces.push(setup);
            }
            // grab the test trace
            while let Some(test_trace) = trace_iter.next() {
                test_trace.update_identified(
                    0,
                    known_contracts.expect("traces enabled but no identified_contracts"),
                    &mut ident,
                    evm,
                );

                if test_trace.arena[0].trace.addr != Address::zero() {
                    traces.push(test_trace);
                }
            }

            // pass back the identified contracts and traces
            *identified_contracts = ident;
        }
        evm.reset_traces();
    }
}