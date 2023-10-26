use crate::{
    result::{SuiteResult, TestKind, TestResult, TestSetup, TestStatus},
    TestFilter, TestOptions,
};
use alloy_json_abi::{Function, JsonAbi as Abi};
use alloy_primitives::{Address, Bytes, U256};
use eyre::Result;
use foundry_common::{
    contracts::{ContractsByAddress, ContractsByArtifact},
    TestFunctionExt,
};
use foundry_config::{FuzzConfig, InvariantConfig};
use foundry_evm::{
    decode::decode_console_logs,
    executors::{
        fuzz::{CaseOutcome, CounterExampleOutcome, FuzzOutcome, FuzzedExecutor},
        invariant::{replay_run, InvariantExecutor, InvariantFuzzError, InvariantFuzzTestResult},
        CallResult, EvmError, ExecutionErr, Executor, CALLER,
    },
    fuzz::{invariant::InvariantContract, CounterExample},
    traces::{load_contracts, TraceKind},
};
use proptest::test_runner::{TestError, TestRunner};
use rayon::prelude::*;
use std::{
    collections::{BTreeMap, HashMap},
    time::Instant,
};

/// A type that executes all tests of a contract
#[derive(Debug, Clone)]
pub struct ContractRunner<'a> {
    pub name: &'a str,
    /// The executor used by the runner.
    pub executor: Executor,
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
    /// Should generate debug traces
    pub debug: bool,
}

impl<'a> ContractRunner<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &'a str,
        executor: Executor,
        contract: &'a Abi,
        code: Bytes,
        initial_balance: U256,
        sender: Option<Address>,
        errors: Option<&'a Abi>,
        predeploy_libs: &'a [Bytes],
        debug: bool,
    ) -> Self {
        Self {
            name,
            executor,
            contract,
            code,
            initial_balance,
            sender: sender.unwrap_or_default(),
            errors,
            predeploy_libs,
            debug,
        }
    }
}

impl<'a> ContractRunner<'a> {
    /// Deploys the test contract inside the runner from the sending account, and optionally runs
    /// the `setUp` function on the test contract.
    pub fn setup(&mut self, setup: bool) -> TestSetup {
        match self._setup(setup) {
            Ok(setup) => setup,
            Err(err) => TestSetup::failed(err.to_string()),
        }
    }

    fn _setup(&mut self, setup: bool) -> Result<TestSetup> {
        trace!(?setup, "Setting test contract");

        // We max out their balance so that they can deploy and make calls.
        self.executor.set_balance(self.sender, U256::MAX)?;
        self.executor.set_balance(CALLER, U256::MAX)?;

        // We set the nonce of the deployer accounts to 1 to get the same addresses as DappTools
        self.executor.set_nonce(self.sender, 1)?;

        // Deploy libraries
        let mut logs = Vec::new();
        let mut traces = Vec::with_capacity(self.predeploy_libs.len());
        for code in self.predeploy_libs.iter() {
            match self.executor.deploy(self.sender, code.clone(), U256::ZERO, self.errors) {
                Ok(d) => {
                    logs.extend(d.logs);
                    traces.extend(d.traces.map(|traces| (TraceKind::Deployment, traces)));
                }
                Err(e) => {
                    return Ok(TestSetup::from_evm_error_with(e, logs, traces, Default::default()))
                }
            }
        }

        // Deploy the test contract
        let address =
            match self.executor.deploy(self.sender, self.code.clone(), U256::ZERO, self.errors) {
                Ok(d) => {
                    logs.extend(d.logs);
                    traces.extend(d.traces.map(|traces| (TraceKind::Deployment, traces)));
                    d.address
                }
                Err(e) => {
                    return Ok(TestSetup::from_evm_error_with(e, logs, traces, Default::default()))
                }
            };

        // Now we set the contracts initial balance, and we also reset `self.sender`s and `CALLER`s
        // balance to the initial balance we want
        self.executor.set_balance(address, self.initial_balance)?;
        self.executor.set_balance(self.sender, self.initial_balance)?;
        self.executor.set_balance(CALLER, self.initial_balance)?;

        self.executor.deploy_create2_deployer()?;

        // Optionally call the `setUp` function
        let setup = if setup {
            trace!("setting up");
            let (setup_logs, setup_traces, labeled_addresses, reason) =
                match self.executor.setup(None, address) {
                    Ok(CallResult { traces, labels, logs, .. }) => {
                        trace!(contract = ?address, "successfully setUp test");
                        (logs, traces, labels, None)
                    }
                    Err(EvmError::Execution(err)) => {
                        let ExecutionErr { traces, labels, logs, reason, .. } = *err;
                        error!(reason = ?reason, contract = ?address, "setUp failed");
                        (logs, traces, labels, Some(format!("Setup failed: {reason}")))
                    }
                    Err(err) => {
                        error!(reason=?err, contract= ?address, "setUp failed");
                        (Vec::new(), None, BTreeMap::new(), Some(format!("Setup failed: {err}")))
                    }
                };
            traces.extend(setup_traces.map(|traces| (TraceKind::Setup, traces)));
            logs.extend(setup_logs);

            TestSetup { address, logs, traces, labeled_addresses, reason }
        } else {
            TestSetup::success(address, logs, traces, Default::default())
        };

        Ok(setup)
    }

    /// Runs all tests for a contract whose names match the provided regular expression
    pub fn run_tests(
        mut self,
        filter: impl TestFilter,
        test_options: TestOptions,
        known_contracts: Option<&ContractsByArtifact>,
    ) -> SuiteResult {
        info!("starting tests");
        let start = Instant::now();
        let mut warnings = Vec::new();

        let setup_fns: Vec<_> =
            self.contract.functions().filter(|func| func.name.is_setup()).collect();

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
            return SuiteResult::new(
                start.elapsed(),
                [("setUp()".to_string(), TestResult::fail("Multiple setUp functions".to_string()))]
                    .into(),
                warnings,
            )
        }

        let has_invariants = self.contract.functions().any(|func| func.is_invariant_test());

        // Invariant testing requires tracing to figure out what contracts were created.
        let tmp_tracing = self.executor.inspector.tracer.is_none() && has_invariants && needs_setup;
        if tmp_tracing {
            self.executor.set_tracing(true);
        }
        let setup = self.setup(needs_setup);
        if tmp_tracing {
            self.executor.set_tracing(false);
        }

        if setup.reason.is_some() {
            // The setup failed, so we return a single test result for `setUp`
            return SuiteResult::new(
                start.elapsed(),
                [(
                    "setUp()".to_string(),
                    TestResult {
                        status: TestStatus::Failure,
                        reason: setup.reason,
                        counterexample: None,
                        decoded_logs: decode_console_logs(&setup.logs),
                        logs: setup.logs,
                        kind: TestKind::Standard(0),
                        traces: setup.traces,
                        coverage: None,
                        labeled_addresses: setup.labeled_addresses,
                        ..Default::default()
                    },
                )]
                .into(),
                warnings,
            )
        }

        let functions: Vec<_> = self.contract.functions().collect();
        let mut test_results = functions
            .par_iter()
            .filter(|&&func| func.is_test() && filter.matches_test(func.signature()))
            .map(|&func| {
                let should_fail = func.is_test_fail();
                let res = if func.is_fuzz_test() {
                    let runner = test_options.fuzz_runner(self.name, &func.name);
                    let fuzz_config = test_options.fuzz_config(self.name, &func.name);
                    self.run_fuzz_test(func, should_fail, runner, setup.clone(), *fuzz_config)
                } else {
                    self.run_test(func, should_fail, setup.clone())
                };
                (func.signature(), res)
            })
            .collect::<BTreeMap<_, _>>();

        if has_invariants {
            let identified_contracts = load_contracts(setup.traces.clone(), known_contracts);
            let results: Vec<_> = functions
                .par_iter()
                .filter(|&&func| func.is_invariant_test() && filter.matches_test(func.signature()))
                .map(|&func| {
                    let runner = test_options.invariant_runner(self.name, &func.name);
                    let invariant_config = test_options.invariant_config(self.name, &func.name);
                    let res = self.run_invariant_test(
                        runner,
                        setup.clone(),
                        *invariant_config,
                        func,
                        known_contracts,
                        &identified_contracts,
                    );
                    (func.signature(), res)
                })
                .collect();
            test_results.extend(results);
        }

        let duration = start.elapsed();
        if !test_results.is_empty() {
            let successful =
                test_results.iter().filter(|(_, tst)| tst.status == TestStatus::Success).count();
            info!(
                duration = ?duration,
                "done. {}/{} successful",
                successful,
                test_results.len()
            );
        }

        SuiteResult::new(duration, test_results, warnings)
    }

    /// Runs a single test
    ///
    /// Calls the given functions and returns the `TestResult`.
    ///
    /// State modifications are not committed to the evm database but discarded after the call,
    /// similar to `eth_call`.
    #[instrument(name = "test", skip_all, fields(name = %func.signature(), %should_fail))]
    pub fn run_test(&self, func: &Function, should_fail: bool, setup: TestSetup) -> TestResult {
        let TestSetup { address, mut logs, mut traces, mut labeled_addresses, .. } = setup;

        // Run unit test
        let mut executor = self.executor.clone();
        let start = Instant::now();
        let debug_arena;
        let (reverted, reason, gas, stipend, coverage, state_changeset, breakpoints) =
            match executor.execute_test::<_, _>(
                self.sender,
                address,
                func.clone(),
                vec![],
                U256::ZERO,
                self.errors,
            ) {
                Ok(CallResult {
                    reverted,
                    gas_used: gas,
                    stipend,
                    logs: execution_logs,
                    traces: execution_trace,
                    coverage,
                    labels: new_labels,
                    state_changeset,
                    debug,
                    breakpoints,
                    ..
                }) => {
                    traces.extend(execution_trace.map(|traces| (TraceKind::Execution, traces)));
                    labeled_addresses.extend(new_labels);
                    logs.extend(execution_logs);
                    debug_arena = debug;
                    (reverted, None, gas, stipend, coverage, state_changeset, breakpoints)
                }
                Err(EvmError::Execution(err)) => {
                    traces.extend(err.traces.map(|traces| (TraceKind::Execution, traces)));
                    labeled_addresses.extend(err.labels);
                    logs.extend(err.logs);
                    debug_arena = err.debug;
                    (
                        err.reverted,
                        Some(err.reason),
                        err.gas_used,
                        err.stipend,
                        None,
                        err.state_changeset,
                        HashMap::new(),
                    )
                }
                Err(EvmError::SkipError) => {
                    return TestResult {
                        status: TestStatus::Skipped,
                        reason: None,
                        decoded_logs: decode_console_logs(&logs),
                        traces,
                        labeled_addresses,
                        kind: TestKind::Standard(0),
                        ..Default::default()
                    }
                }
                Err(err) => {
                    return TestResult {
                        status: TestStatus::Failure,
                        reason: Some(err.to_string()),
                        decoded_logs: decode_console_logs(&logs),
                        traces,
                        labeled_addresses,
                        kind: TestKind::Standard(0),
                        ..Default::default()
                    }
                }
            };

        let success = executor.is_success(
            setup.address,
            reverted,
            state_changeset.expect("we should have a state changeset"),
            should_fail,
        );

        // Record test execution time
        debug!(
            duration = ?start.elapsed(),
            %success,
            %gas
        );

        TestResult {
            status: match success {
                true => TestStatus::Success,
                false => TestStatus::Failure,
            },
            reason,
            counterexample: None,
            decoded_logs: decode_console_logs(&logs),
            logs,
            kind: TestKind::Standard(gas.overflowing_sub(stipend).0),
            traces,
            coverage,
            labeled_addresses,
            debug: debug_arena,
            breakpoints,
        }
    }

    #[instrument(name = "invariant-test", skip_all)]
    pub fn run_invariant_test(
        &self,
        runner: TestRunner,
        setup: TestSetup,
        invariant_config: InvariantConfig,
        func: &Function,
        known_contracts: Option<&ContractsByArtifact>,
        identified_contracts: &ContractsByAddress,
    ) -> TestResult {
        trace!(target: "forge::test::fuzz", "executing invariant test for {:?}", func.name);
        let empty = ContractsByArtifact::default();
        let project_contracts = known_contracts.unwrap_or(&empty);
        let TestSetup { address, logs, traces, labeled_addresses, .. } = setup;

        // First, run the test normally to see if it needs to be skipped.
        if let Err(EvmError::SkipError) = self.executor.clone().execute_test::<_, _>(
            self.sender,
            address,
            func.clone(),
            vec![],
            U256::ZERO,
            self.errors,
        ) {
            return TestResult {
                status: TestStatus::Skipped,
                reason: None,
                decoded_logs: decode_console_logs(&logs),
                traces,
                labeled_addresses,
                kind: TestKind::Invariant { runs: 1, calls: 1, reverts: 1 },
                ..Default::default()
            }
        };

        let mut evm = InvariantExecutor::new(
            self.executor.clone(),
            runner,
            invariant_config,
            identified_contracts,
            project_contracts,
        );

        let invariant_contract =
            InvariantContract { address, invariant_function: func, abi: self.contract };

        let InvariantFuzzTestResult { error, cases, reverts, last_run_inputs } = match evm
            .invariant_fuzz(invariant_contract.clone())
        {
            Ok(x) => x,
            Err(e) => {
                return TestResult {
                    status: TestStatus::Failure,
                    reason: Some(format!("Failed to set up invariant testing environment: {e}")),
                    decoded_logs: decode_console_logs(&logs),
                    traces,
                    labeled_addresses,
                    kind: TestKind::Invariant { runs: 0, calls: 0, reverts: 0 },
                    ..Default::default()
                }
            }
        };

        let mut counterexample = None;
        let mut logs = logs.clone();
        let mut traces = traces.clone();
        let success = error.is_none();
        let reason = error
            .as_ref()
            .and_then(|err| (!err.revert_reason.is_empty()).then(|| err.revert_reason.clone()));
        match error {
            // If invariants were broken, replay the error to collect logs and traces
            Some(error @ InvariantFuzzError { test_error: TestError::Fail(_, _), .. }) => {
                match error.replay(
                    self.executor.clone(),
                    known_contracts,
                    identified_contracts.clone(),
                    &mut logs,
                    &mut traces,
                ) {
                    Ok(c) => counterexample = c,
                    Err(err) => {
                        error!(?err, "Failed to replay invariant error")
                    }
                };

                logs.extend(error.logs);

                if let Some(error_traces) = error.traces {
                    traces.push((TraceKind::Execution, error_traces));
                }
            }

            // If invariants ran successfully, replay the last run to collect logs and
            // traces.
            _ => {
                replay_run(
                    &invariant_contract,
                    self.executor.clone(),
                    known_contracts,
                    identified_contracts.clone(),
                    &mut logs,
                    &mut traces,
                    func.clone(),
                    last_run_inputs.clone(),
                );
            }
        }

        let kind = TestKind::Invariant {
            runs: cases.len(),
            calls: cases.iter().map(|sequence| sequence.cases().len()).sum(),
            reverts,
        };

        TestResult {
            status: match success {
                true => TestStatus::Success,
                false => TestStatus::Failure,
            },
            reason,
            counterexample,
            decoded_logs: decode_console_logs(&logs),
            logs,
            kind,
            coverage: None, // TODO ?
            traces,
            labeled_addresses: labeled_addresses.clone(),
            ..Default::default() // TODO collect debug traces on the last run or error
        }
    }

    #[instrument(name = "fuzz-test", skip_all, fields(name = %func.signature(), %should_fail))]
    pub fn run_fuzz_test(
        &self,
        func: &Function,
        should_fail: bool,
        runner: TestRunner,
        setup: TestSetup,
        fuzz_config: FuzzConfig,
    ) -> TestResult {
        let TestSetup { address, mut logs, mut traces, mut labeled_addresses, .. } = setup;

        // Run fuzz test
        let start = Instant::now();
        let fuzzed_executor =
            FuzzedExecutor::new(&self.executor, runner.clone(), self.sender, fuzz_config);
        let state = fuzzed_executor.build_fuzz_state();
        let mut result = fuzzed_executor.fuzz(func, address, should_fail, self.errors);

        let mut debug = Default::default();
        let mut breakpoints = Default::default();

        // Check the last test result and skip the test
        // if it's marked as so.
        if let Some("SKIPPED") = result.reason.as_deref() {
            return TestResult {
                status: TestStatus::Skipped,
                reason: None,
                decoded_logs: decode_console_logs(&logs),
                traces,
                labeled_addresses,
                kind: TestKind::Standard(0),
                debug,
                breakpoints,
                ..Default::default()
            }
        }

        // if should debug
        if self.debug {
            let mut debug_executor = self.executor.clone();
            // turn the debug traces on
            debug_executor.inspector.enable_debugger(true);
            debug_executor.inspector.tracing(true);
            let calldata = if let Some(counterexample) = result.counterexample.as_ref() {
                match counterexample {
                    CounterExample::Single(ce) => ce.calldata.clone(),
                    _ => unimplemented!(),
                }
            } else {
                result.first_case.calldata.clone()
            };
            // rerun the last relevant test with traces
            let debug_result = FuzzedExecutor::new(
                &debug_executor,
                runner,
                self.sender,
                fuzz_config,
            )
            .single_fuzz(&state, address, should_fail, calldata);

            (debug, breakpoints) = match debug_result {
                Ok(fuzz_outcome) => match fuzz_outcome {
                    FuzzOutcome::Case(CaseOutcome { debug, breakpoints, .. }) => {
                        (debug, breakpoints)
                    }
                    FuzzOutcome::CounterExample(CounterExampleOutcome {
                        debug,
                        breakpoints,
                        ..
                    }) => (debug, breakpoints),
                },
                Err(_) => (Default::default(), Default::default()),
            };
        }

        let kind = TestKind::Fuzz {
            median_gas: result.median_gas(false),
            mean_gas: result.mean_gas(false),
            first_case: result.first_case,
            runs: result.gas_by_case.len(),
        };

        // Record logs, labels and traces
        logs.append(&mut result.logs);
        labeled_addresses.append(&mut result.labeled_addresses);
        traces.extend(result.traces.map(|traces| (TraceKind::Execution, traces)));

        // Record test execution time
        debug!(
            duration = ?start.elapsed(),
            success = %result.success
        );

        TestResult {
            status: match result.success {
                true => TestStatus::Success,
                false => TestStatus::Failure,
            },
            reason: result.reason,
            counterexample: result.counterexample,
            decoded_logs: decode_console_logs(&logs),
            logs,
            kind,
            traces,
            coverage: result.coverage,
            labeled_addresses,
            debug,
            breakpoints,
        }
    }
}
