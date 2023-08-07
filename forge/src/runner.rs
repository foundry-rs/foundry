use crate::{
    result::{SuiteResult, TestKind, TestResult, TestSetup, TestStatus},
    TestFilter, TestOptions,
};
use ethers::{
    abi::{Abi, Function},
    types::{Address, Bytes, U256},
};
use eyre::Result;
use foundry_common::{
    contracts::{ContractsByAddress, ContractsByArtifact},
    TestFunctionExt,
};
use foundry_config::{FuzzConfig, InvariantConfig};
use foundry_evm::{
    decode::decode_console_logs,
    executor::{CallResult, EvmError, ExecutionErr, Executor},
    fuzz::{
        invariant::{
            InvariantContract, InvariantExecutor, InvariantFuzzError, InvariantFuzzTestResult,
        },
        FuzzedExecutor,
    },
    trace::{load_contracts, TraceKind},
    CALLER,
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
            match self.executor.deploy(self.sender, code.0.clone(), 0u32.into(), self.errors) {
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
        let address = match self.executor.deploy(
            self.sender,
            self.code.0.clone(),
            0u32.into(),
            self.errors,
        ) {
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
        let original_tracing = self.executor.inspector_config().tracing;
        if has_invariants && needs_setup {
            self.executor.set_tracing(true);
        }

        let setup = self.setup(needs_setup);
        self.executor.set_tracing(original_tracing);

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
                        breakpoints: Default::default(),
                        debug: Default::default(),
                    },
                )]
                .into(),
                warnings,
            )
        }

        let mut test_results = self
            .contract
            .functions
            .par_iter()
            .flat_map(|(_, f)| f)
            .filter(|&func| func.is_test() && filter.matches_test(func.signature()))
            .map(|func| {
                let should_fail = func.is_test_fail();
                let res = if func.is_fuzz_test() {
                    let runner = test_options.fuzz_runner(self.name, &func.name);
                    let fuzz_config = test_options.fuzz_config(self.name, &func.name);
                    self.run_fuzz_test(func, should_fail, runner, setup.clone(), *fuzz_config)
                } else {
                    self.clone().run_test(func, should_fail, setup.clone())
                };
                (func.signature(), res)
            })
            .collect::<BTreeMap<_, _>>();

        if has_invariants {
            let identified_contracts = load_contracts(setup.traces.clone(), known_contracts);

            // TODO: par_iter ?
            let functions = self
                .contract
                .functions()
                .filter(|&func| func.is_invariant_test() && filter.matches_test(func.signature()));
            for func in functions {
                let runner = test_options.invariant_runner(self.name, &func.name);
                let invariant_config = test_options.invariant_config(self.name, &func.name);
                let results = self.run_invariant_test(
                    runner,
                    setup.clone(),
                    *invariant_config,
                    vec![func],
                    known_contracts,
                    identified_contracts.clone(),
                );
                for result in results {
                    test_results.insert(func.signature(), result);
                }
            }
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
    pub fn run_test(mut self, func: &Function, should_fail: bool, setup: TestSetup) -> TestResult {
        let TestSetup { address, mut logs, mut traces, mut labeled_addresses, .. } = setup;

        // Run unit test
        let start = Instant::now();
        let mut debug_arena = None;
        let (reverted, reason, gas, stipend, coverage, state_changeset, breakpoints) = match self
            .executor
            .execute_test::<(), _, _>(self.sender, address, func.clone(), (), 0.into(), self.errors)
        {
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

        let success = self.executor.is_success(
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
        &mut self,
        runner: TestRunner,
        setup: TestSetup,
        invariant_config: InvariantConfig,
        functions: Vec<&Function>,
        known_contracts: Option<&ContractsByArtifact>,
        identified_contracts: ContractsByAddress,
    ) -> Vec<TestResult> {
        trace!(target: "forge::test::fuzz", "executing invariant test with invariant functions {:?}",  functions.iter().map(|f|&f.name).collect::<Vec<_>>());
        let empty = ContractsByArtifact::default();
        let project_contracts = known_contracts.unwrap_or(&empty);
        let TestSetup { address, logs, traces, labeled_addresses, .. } = setup;

        // First, run the test normally to see if it needs to be skipped.
        if let Err(EvmError::SkipError) = self.executor.execute_test::<(), _, _>(
            self.sender,
            address,
            functions[0].clone(),
            (),
            0.into(),
            self.errors,
        ) {
            return vec![TestResult {
                status: TestStatus::Skipped,
                reason: None,
                decoded_logs: decode_console_logs(&logs),
                traces,
                labeled_addresses,
                kind: TestKind::Standard(0),
                ..Default::default()
            }]
        };

        let mut evm = InvariantExecutor::new(
            &mut self.executor,
            runner,
            invariant_config,
            &identified_contracts,
            project_contracts,
        );

        let invariant_contract =
            InvariantContract { address, invariant_functions: functions, abi: self.contract };

        let Ok(InvariantFuzzTestResult { invariants, cases, reverts, mut last_call_results }) =
            evm.invariant_fuzz(invariant_contract)
        else {
            return vec![]
        };

        invariants
            .into_iter()
            .map(|(func_name, test_error)| {
                let mut counterexample = None;
                let mut logs = logs.clone();
                let mut traces = traces.clone();

                let success = test_error.is_none();
                let reason = test_error.as_ref().and_then(|err| {
                    (!err.revert_reason.is_empty()).then(|| err.revert_reason.clone())
                });

                match test_error {
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
                    // If invariants ran successfully, collect last call logs and traces
                    _ => {
                        if let Some(last_call_result) = last_call_results
                            .as_mut()
                            .and_then(|call_results| call_results.remove(&func_name))
                        {
                            logs.extend(last_call_result.logs);

                            if let Some(last_call_traces) = last_call_result.traces {
                                traces.push((TraceKind::Execution, last_call_traces));
                            }
                        }
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
                    // TODO
                    debug: Default::default(),
                    breakpoints: Default::default(),
                }
            })
            .collect()
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
        let mut result = FuzzedExecutor::new(&self.executor, runner, self.sender, fuzz_config)
            .fuzz(func, address, should_fail, self.errors);

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
                debug: result.debug,
                breakpoints: result.breakpoints,
                ..Default::default()
            }
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
            debug: result.debug,
            breakpoints: result.breakpoints,
        }
    }
}
