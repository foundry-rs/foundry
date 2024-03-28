//! The Forge test runner.

use crate::{
    multi_runner::{is_matching_test, TestContract},
    result::{SuiteResult, TestKind, TestResult, TestSetup, TestStatus},
    TestFilter, TestOptions,
};
use alloy_json_abi::Function;
use alloy_primitives::{Address, U256};
use eyre::Result;
use foundry_common::{
    contracts::{ContractsByAddress, ContractsByArtifact},
    TestFunctionExt,
};
use foundry_config::{FuzzConfig, InvariantConfig};
use foundry_evm::{
    constants::CALLER,
    coverage::HitMaps,
    decode::{decode_console_logs, RevertDecoder},
    executors::{
        fuzz::{CaseOutcome, CounterExampleOutcome, FuzzOutcome, FuzzedExecutor},
        invariant::{replay_run, InvariantExecutor, InvariantFuzzError, InvariantFuzzTestResult},
        EvmError, ExecutionErr, Executor, RawCallResult,
    },
    fuzz::{invariant::InvariantContract, CounterExample},
    traces::{load_contracts, TraceKind},
};
use proptest::test_runner::TestRunner;
use rayon::prelude::*;
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    time::Instant,
};

/// A type that executes all tests of a contract
#[derive(Clone, Debug)]
pub struct ContractRunner<'a> {
    pub name: &'a str,
    /// The data of the contract being ran.
    pub contract: &'a TestContract,
    /// The executor used by the runner.
    pub executor: Executor,
    /// Revert decoder. Contains all known errors.
    pub revert_decoder: &'a RevertDecoder,
    /// The initial balance of the test contract
    pub initial_balance: U256,
    /// The address which will be used as the `from` field in all EVM calls
    pub sender: Address,
    /// Should generate debug traces
    pub debug: bool,
}

impl<'a> ContractRunner<'a> {
    pub fn new(
        name: &'a str,
        executor: Executor,
        contract: &'a TestContract,
        initial_balance: U256,
        sender: Option<Address>,
        revert_decoder: &'a RevertDecoder,
        debug: bool,
    ) -> Self {
        Self {
            name,
            executor,
            contract,
            initial_balance,
            sender: sender.unwrap_or_default(),
            revert_decoder,
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
        let mut traces = Vec::with_capacity(self.contract.libs_to_deploy.len());
        for code in self.contract.libs_to_deploy.iter() {
            match self.executor.deploy(
                self.sender,
                code.clone(),
                U256::ZERO,
                Some(self.revert_decoder),
            ) {
                Ok(d) => {
                    logs.extend(d.raw.logs);
                    traces.extend(d.raw.traces.map(|traces| (TraceKind::Deployment, traces)));
                }
                Err(e) => {
                    return Ok(TestSetup::from_evm_error_with(e, logs, traces, Default::default()))
                }
            }
        }

        let address = self.sender.create(self.executor.get_nonce(self.sender)?);

        // Set the contracts initial balance before deployment, so it is available during
        // construction
        self.executor.set_balance(address, self.initial_balance)?;

        // Deploy the test contract
        match self.executor.deploy(
            self.sender,
            self.contract.bytecode.clone(),
            U256::ZERO,
            Some(self.revert_decoder),
        ) {
            Ok(d) => {
                logs.extend(d.raw.logs);
                traces.extend(d.raw.traces.map(|traces| (TraceKind::Deployment, traces)));
                d.address
            }
            Err(e) => {
                return Ok(TestSetup::from_evm_error_with(e, logs, traces, Default::default()))
            }
        };

        // Reset `self.sender`s and `CALLER`s balance to the initial balance we want
        self.executor.set_balance(self.sender, self.initial_balance)?;
        self.executor.set_balance(CALLER, self.initial_balance)?;

        self.executor.deploy_create2_deployer()?;

        // Optionally call the `setUp` function
        let setup = if setup {
            trace!("setting up");
            let res = self.executor.setup(None, address, Some(self.revert_decoder));
            let (setup_logs, setup_traces, labeled_addresses, reason, coverage) = match res {
                Ok(RawCallResult { traces, labels, logs, coverage, .. }) => {
                    trace!(contract=%address, "successfully setUp test");
                    (logs, traces, labels, None, coverage)
                }
                Err(EvmError::Execution(err)) => {
                    let ExecutionErr {
                        raw: RawCallResult { traces, labels, logs, coverage, .. },
                        reason,
                    } = *err;
                    (logs, traces, labels, Some(format!("setup failed: {reason}")), coverage)
                }
                Err(err) => {
                    (Vec::new(), None, HashMap::new(), Some(format!("setup failed: {err}")), None)
                }
            };
            traces.extend(setup_traces.map(|traces| (TraceKind::Setup, traces)));
            logs.extend(setup_logs);

            TestSetup { address, logs, traces, labeled_addresses, reason, coverage }
        } else {
            TestSetup::success(address, logs, traces, Default::default(), None)
        };

        Ok(setup)
    }

    /// Runs all tests for a contract whose names match the provided regular expression
    pub fn run_tests(
        mut self,
        filter: &dyn TestFilter,
        test_options: &TestOptions,
        known_contracts: Option<&ContractsByArtifact>,
    ) -> SuiteResult {
        info!("starting tests");
        let start = Instant::now();
        let mut warnings = Vec::new();

        let setup_fns: Vec<_> =
            self.contract.abi.functions().filter(|func| func.name.is_setup()).collect();

        let needs_setup = setup_fns.len() == 1 && setup_fns[0].name == "setUp";

        // There is a single miss-cased `setUp` function, so we add a warning
        for &setup_fn in setup_fns.iter() {
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
                [("setUp()".to_string(), TestResult::fail("multiple setUp functions".to_string()))]
                    .into(),
                warnings,
                self.contract.libraries.clone(),
            )
        }

        let has_invariants = self.contract.abi.functions().any(|func| func.is_invariant_test());

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
                        coverage: setup.coverage,
                        labeled_addresses: setup.labeled_addresses,
                        ..Default::default()
                    },
                )]
                .into(),
                warnings,
                self.contract.libraries.clone(),
            )
        }

        // Filter out functions sequentially since it's very fast and there is no need to do it
        // in parallel.
        let find_timer = Instant::now();
        let functions = self
            .contract
            .abi
            .functions()
            .filter(|func| is_matching_test(func, filter))
            .collect::<Vec<_>>();
        let find_time = find_timer.elapsed();
        debug!(
            "Found {} test functions out of {} in {:?}",
            functions.len(),
            self.contract.abi.functions().count(),
            find_time,
        );

        let identified_contracts =
            has_invariants.then(|| load_contracts(setup.traces.clone(), known_contracts));
        let test_results = functions
            .par_iter()
            .map(|&func| {
                let sig = func.signature();

                let setup = setup.clone();
                let should_fail = func.is_test_fail();
                let res = if func.is_invariant_test() {
                    let runner = test_options.invariant_runner(self.name, &func.name);
                    let invariant_config = test_options.invariant_config(self.name, &func.name);
                    self.run_invariant_test(
                        runner,
                        setup,
                        *invariant_config,
                        func,
                        known_contracts,
                        identified_contracts.as_ref().unwrap(),
                    )
                } else if func.is_fuzz_test() {
                    debug_assert!(func.is_test());
                    let runner = test_options.fuzz_runner(self.name, &func.name);
                    let fuzz_config = test_options.fuzz_config(self.name, &func.name);
                    self.run_fuzz_test(func, should_fail, runner, setup, fuzz_config.clone())
                } else {
                    debug_assert!(func.is_test());
                    self.run_test(func, should_fail, setup)
                };

                (sig, res)
            })
            .collect::<BTreeMap<_, _>>();

        let duration = start.elapsed();
        let suite_result =
            SuiteResult::new(duration, test_results, warnings, self.contract.libraries.clone());
        info!(
            duration=?suite_result.duration,
            "done. {}/{} successful",
            suite_result.passed(),
            suite_result.test_results.len()
        );
        suite_result
    }

    /// Runs a single test
    ///
    /// Calls the given functions and returns the `TestResult`.
    ///
    /// State modifications are not committed to the evm database but discarded after the call,
    /// similar to `eth_call`.
    pub fn run_test(&self, func: &Function, should_fail: bool, setup: TestSetup) -> TestResult {
        let span = info_span!("test", %should_fail);
        if !span.is_disabled() {
            let sig = &func.signature()[..];
            if enabled!(tracing::Level::TRACE) {
                span.record("sig", sig);
            } else {
                span.record("sig", sig.split('(').next().unwrap());
            }
        }
        let _guard = span.enter();

        let TestSetup {
            address, mut logs, mut traces, mut labeled_addresses, mut coverage, ..
        } = setup;

        // Run unit test
        let mut executor = self.executor.clone();
        let start = Instant::now();
        let debug_arena;
        let (reverted, reason, gas, stipend, coverage, state_changeset, breakpoints) =
            match executor.execute_test(
                self.sender,
                address,
                func,
                &[],
                U256::ZERO,
                Some(self.revert_decoder),
            ) {
                Ok(res) => {
                    let RawCallResult {
                        reverted,
                        gas_used: gas,
                        stipend,
                        logs: execution_logs,
                        traces: execution_trace,
                        coverage: execution_coverage,
                        labels: new_labels,
                        state_changeset,
                        debug,
                        cheatcodes,
                        ..
                    } = res.raw;

                    let breakpoints = cheatcodes.map(|c| c.breakpoints).unwrap_or_default();
                    traces.extend(execution_trace.map(|traces| (TraceKind::Execution, traces)));
                    labeled_addresses.extend(new_labels);
                    logs.extend(execution_logs);
                    debug_arena = debug;
                    coverage = merge_coverages(coverage, execution_coverage);

                    (reverted, None, gas, stipend, coverage, state_changeset, breakpoints)
                }
                Err(EvmError::Execution(err)) => {
                    let ExecutionErr { raw, reason } = *err;
                    traces.extend(raw.traces.map(|traces| (TraceKind::Execution, traces)));
                    labeled_addresses.extend(raw.labels);
                    logs.extend(raw.logs);
                    debug_arena = raw.debug;
                    (
                        raw.reverted,
                        Some(reason),
                        raw.gas_used,
                        raw.stipend,
                        None,
                        raw.state_changeset,
                        Default::default(),
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
                        duration: start.elapsed(),
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
                        duration: start.elapsed(),
                        ..Default::default()
                    }
                }
            };

        let success = executor.is_success(
            setup.address,
            reverted,
            Cow::Owned(state_changeset.unwrap()),
            should_fail,
        );

        // Record test execution time
        let duration = start.elapsed();
        trace!(?duration, gas, reverted, should_fail, success);

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
            duration,
            gas_report_traces: Vec::new(),
        }
    }

    #[instrument(name = "invariant_test", skip_all)]
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
        let TestSetup { address, logs, traces, labeled_addresses, coverage, .. } = setup;

        // First, run the test normally to see if it needs to be skipped.
        let start = Instant::now();
        if let Err(EvmError::SkipError) = self.executor.clone().execute_test(
            self.sender,
            address,
            func,
            &[],
            U256::ZERO,
            Some(self.revert_decoder),
        ) {
            return TestResult {
                status: TestStatus::Skipped,
                reason: None,
                decoded_logs: decode_console_logs(&logs),
                traces,
                labeled_addresses,
                kind: TestKind::Invariant { runs: 1, calls: 1, reverts: 1 },
                coverage,
                duration: start.elapsed(),
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
            InvariantContract { address, invariant_function: func, abi: &self.contract.abi };

        let InvariantFuzzTestResult { error, cases, reverts, last_run_inputs, gas_report_traces } =
            match evm.invariant_fuzz(invariant_contract.clone()) {
                Ok(x) => x,
                Err(e) => {
                    return TestResult {
                        status: TestStatus::Failure,
                        reason: Some(format!(
                            "failed to set up invariant testing environment: {e}"
                        )),
                        decoded_logs: decode_console_logs(&logs),
                        traces,
                        labeled_addresses,
                        kind: TestKind::Invariant { runs: 0, calls: 0, reverts: 0 },
                        duration: start.elapsed(),
                        ..Default::default()
                    }
                }
            };

        let mut counterexample = None;
        let mut logs = logs.clone();
        let mut traces = traces.clone();
        let success = error.is_none();
        let reason = error.as_ref().and_then(|err| err.revert_reason());
        let mut coverage = coverage.clone();
        match error {
            // If invariants were broken, replay the error to collect logs and traces
            Some(error) => match error {
                InvariantFuzzError::BrokenInvariant(case_data) |
                InvariantFuzzError::Revert(case_data) => {
                    match case_data.replay(
                        self.executor.clone(),
                        known_contracts,
                        identified_contracts.clone(),
                        &mut logs,
                        &mut traces,
                    ) {
                        Ok(c) => counterexample = c,
                        Err(err) => {
                            error!(%err, "Failed to replay invariant error");
                        }
                    };
                }
                InvariantFuzzError::MaxAssumeRejects(_) => {}
            },

            // If invariants ran successfully, replay the last run to collect logs and
            // traces.
            _ => {
                if let Err(err) = replay_run(
                    &invariant_contract,
                    self.executor.clone(),
                    known_contracts,
                    identified_contracts.clone(),
                    &mut logs,
                    &mut traces,
                    &mut coverage,
                    func.clone(),
                    last_run_inputs.clone(),
                ) {
                    error!(%err, "Failed to replay last invariant run");
                }
            }
        }

        TestResult {
            status: match success {
                true => TestStatus::Success,
                false => TestStatus::Failure,
            },
            reason,
            counterexample,
            decoded_logs: decode_console_logs(&logs),
            logs,
            kind: TestKind::Invariant {
                runs: cases.len(),
                calls: cases.iter().map(|sequence| sequence.cases().len()).sum(),
                reverts,
            },
            coverage,
            traces,
            labeled_addresses: labeled_addresses.clone(),
            duration: start.elapsed(),
            gas_report_traces,
            ..Default::default() // TODO collect debug traces on the last run or error
        }
    }

    #[instrument(name = "fuzz_test", skip_all, fields(name = %func.signature(), %should_fail))]
    pub fn run_fuzz_test(
        &self,
        func: &Function,
        should_fail: bool,
        runner: TestRunner,
        setup: TestSetup,
        fuzz_config: FuzzConfig,
    ) -> TestResult {
        let span = info_span!("fuzz_test", %should_fail);
        if !span.is_disabled() {
            let sig = &func.signature()[..];
            if enabled!(tracing::Level::TRACE) {
                span.record("test", sig);
            } else {
                span.record("test", sig.split('(').next().unwrap());
            }
        }
        let _guard = span.enter();

        let TestSetup {
            address, mut logs, mut traces, mut labeled_addresses, mut coverage, ..
        } = setup;

        // Run fuzz test
        let start = Instant::now();
        let fuzzed_executor = FuzzedExecutor::new(
            self.executor.clone(),
            runner.clone(),
            self.sender,
            fuzz_config.clone(),
        );
        let state = fuzzed_executor.build_fuzz_state();
        let result = fuzzed_executor.fuzz(func, address, should_fail, self.revert_decoder);

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
                coverage,
                duration: start.elapsed(),
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
                debug_executor,
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
        logs.extend(result.logs);
        labeled_addresses.extend(result.labeled_addresses);
        traces.extend(result.traces.map(|traces| (TraceKind::Execution, traces)));
        coverage = merge_coverages(coverage, result.coverage);

        // Record test execution time
        let duration = start.elapsed();
        trace!(?duration, success = %result.success);

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
            coverage,
            labeled_addresses,
            debug,
            breakpoints,
            duration,
            gas_report_traces: result.gas_report_traces.into_iter().map(|t| vec![t]).collect(),
        }
    }
}

/// Utility function to merge coverage options
fn merge_coverages(mut coverage: Option<HitMaps>, other: Option<HitMaps>) -> Option<HitMaps> {
    let old_coverage = std::mem::take(&mut coverage);
    match (old_coverage, other) {
        (Some(old_coverage), Some(other)) => Some(old_coverage.merge(other)),
        (None, Some(other)) => Some(other),
        (Some(old_coverage), None) => Some(old_coverage),
        (None, None) => None,
    }
}
