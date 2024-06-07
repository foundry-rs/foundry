//! The Forge test runner.

use crate::{
    fuzz::{invariant::BasicTxDetails, BaseCounterExample},
    multi_runner::{is_matching_test, TestContract},
    progress::{start_fuzz_progress, TestsProgress},
    result::{SuiteResult, TestKind, TestResult, TestSetup, TestStatus},
    TestFilter, TestOptions,
};
use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::Function;
use alloy_primitives::{address, Address, Bytes, U256};
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
        invariant::{
            check_sequence, replay_error, replay_run, InvariantExecutor, InvariantFuzzError,
            InvariantFuzzTestResult,
        },
        CallResult, EvmError, ExecutionErr, Executor, RawCallResult,
    },
    fuzz::{
        fixture_name,
        invariant::{CallDetails, InvariantContract},
        CounterExample, FuzzFixtures,
    },
    traces::{load_contracts, TraceKind},
};
use proptest::test_runner::TestRunner;
use rayon::prelude::*;
use std::{
    borrow::Cow,
    cmp::min,
    collections::{BTreeMap, HashMap},
    time::Instant,
};

/// When running tests, we deploy all external libraries present in the project. To avoid additional
/// libraries affecting nonces of senders used in tests, we are using separate address to
/// predeploy libraries.
///
/// `address(uint160(uint256(keccak256("foundry library deployer"))))`
pub const LIBRARY_DEPLOYER: Address = address!("1F95D37F27EA0dEA9C252FC09D5A6eaA97647353");

/// A type that executes all tests of a contract
#[derive(Clone, Debug)]
pub struct ContractRunner<'a> {
    pub name: &'a str,
    /// The data of the contract being ran.
    pub contract: &'a TestContract,
    /// The libraries that need to be deployed before the contract.
    pub libs_to_deploy: &'a Vec<Bytes>,
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
    /// Overall test run progress.
    progress: Option<&'a TestsProgress>,
}

impl<'a> ContractRunner<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &'a str,
        executor: Executor,
        contract: &'a TestContract,
        libs_to_deploy: &'a Vec<Bytes>,
        initial_balance: U256,
        sender: Option<Address>,
        revert_decoder: &'a RevertDecoder,
        debug: bool,
        progress: Option<&'a TestsProgress>,
    ) -> Self {
        Self {
            name,
            executor,
            contract,
            libs_to_deploy,
            initial_balance,
            sender: sender.unwrap_or_default(),
            revert_decoder,
            debug,
            progress,
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
        self.executor.set_balance(LIBRARY_DEPLOYER, U256::MAX)?;

        let mut logs = Vec::new();
        let mut traces = Vec::with_capacity(self.libs_to_deploy.len());
        for code in self.libs_to_deploy.iter() {
            match self.executor.deploy(
                LIBRARY_DEPLOYER,
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

        // Reset `self.sender`s, `CALLER`s and `LIBRARY_DEPLOYER`'s balance to the initial balance.
        self.executor.set_balance(self.sender, self.initial_balance)?;
        self.executor.set_balance(CALLER, self.initial_balance)?;
        self.executor.set_balance(LIBRARY_DEPLOYER, self.initial_balance)?;

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

            TestSetup {
                address,
                logs,
                traces,
                labeled_addresses,
                reason,
                coverage,
                fuzz_fixtures: self.fuzz_fixtures(address),
            }
        } else {
            TestSetup::success(
                address,
                logs,
                traces,
                Default::default(),
                None,
                self.fuzz_fixtures(address),
            )
        };

        Ok(setup)
    }

    /// Collect fixtures from test contract.
    ///
    /// Fixtures can be defined:
    /// - as storage arrays in test contract, prefixed with `fixture`
    /// - as functions prefixed with `fixture` and followed by parameter name to be fuzzed
    ///
    /// Storage array fixtures:
    /// `uint256[] public fixture_amount = [1, 2, 3];`
    /// define an array of uint256 values to be used for fuzzing `amount` named parameter in scope
    /// of the current test.
    ///
    /// Function fixtures:
    /// `function fixture_owner() public returns (address[] memory){}`
    /// returns an array of addresses to be used for fuzzing `owner` named parameter in scope of the
    /// current test.
    fn fuzz_fixtures(&mut self, address: Address) -> FuzzFixtures {
        let mut fixtures = HashMap::new();
        self.contract.abi.functions().filter(|func| func.is_fixture()).for_each(|func| {
            if func.inputs.is_empty() {
                // Read fixtures declared as functions.
                if let Ok(CallResult { raw: _, decoded_result }) =
                    self.executor.call(CALLER, address, func, &[], U256::ZERO, None)
                {
                    fixtures.insert(fixture_name(func.name.clone()), decoded_result);
                }
            } else {
                // For reading fixtures from storage arrays we collect values by calling the
                // function with incremented indexes until there's an error.
                let mut vals = Vec::new();
                let mut index = 0;
                loop {
                    if let Ok(CallResult { raw: _, decoded_result }) = self.executor.call(
                        CALLER,
                        address,
                        func,
                        &[DynSolValue::Uint(U256::from(index), 256)],
                        U256::ZERO,
                        None,
                    ) {
                        vals.push(decoded_result);
                    } else {
                        // No result returned for this index, we reached the end of storage
                        // array or the function is not a valid fixture.
                        break;
                    }
                    index += 1;
                }
                fixtures.insert(fixture_name(func.name.clone()), DynSolValue::Array(vals));
            };
        });

        FuzzFixtures::new(fixtures)
    }

    /// Runs all tests for a contract whose names match the provided regular expression
    pub fn run_tests(
        mut self,
        filter: &dyn TestFilter,
        test_options: &TestOptions,
        known_contracts: ContractsByArtifact,
        handle: &tokio::runtime::Handle,
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
            has_invariants.then(|| load_contracts(setup.traces.clone(), &known_contracts));
        let test_results = functions
            .par_iter()
            .map(|&func| {
                let _guard = handle.enter();

                let sig = func.signature();
                let span = debug_span!("test", name = tracing::field::Empty).entered();
                if !span.is_disabled() {
                    if enabled!(tracing::Level::TRACE) {
                        span.record("name", &sig);
                    } else {
                        span.record("name", &func.name);
                    }
                }

                let setup = setup.clone();
                let should_fail = func.is_test_fail();
                let res = if func.is_invariant_test() {
                    let runner = test_options.invariant_runner(self.name, &func.name);
                    let invariant_config = test_options.invariant_config(self.name, &func.name);

                    self.run_invariant_test(
                        runner,
                        setup,
                        invariant_config.clone(),
                        func,
                        &known_contracts,
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
        let suite_result = SuiteResult::new(duration, test_results, warnings);
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
    #[instrument(level = "debug", name = "normal", skip_all)]
    pub fn run_test(&self, func: &Function, should_fail: bool, setup: TestSetup) -> TestResult {
        let TestSetup {
            address, mut logs, mut traces, mut labeled_addresses, mut coverage, ..
        } = setup;

        // Run unit test
        let mut executor = self.executor.clone();
        let start: Instant = Instant::now();
        let (raw_call_result, reason) = match executor.execute_test(
            self.sender,
            address,
            func,
            &[],
            U256::ZERO,
            Some(self.revert_decoder),
        ) {
            Ok(res) => (res.raw, None),
            Err(EvmError::Execution(err)) => (err.raw, Some(err.reason)),
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
        } = raw_call_result;

        let breakpoints = cheatcodes.map(|c| c.breakpoints).unwrap_or_default();
        let debug_arena = debug;
        traces.extend(execution_trace.map(|traces| (TraceKind::Execution, traces)));
        labeled_addresses.extend(new_labels);
        logs.extend(execution_logs);
        coverage = merge_coverages(coverage, execution_coverage);

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

    #[instrument(level = "debug", name = "invariant", skip_all)]
    pub fn run_invariant_test(
        &self,
        runner: TestRunner,
        setup: TestSetup,
        invariant_config: InvariantConfig,
        func: &Function,
        known_contracts: &ContractsByArtifact,
        identified_contracts: &ContractsByAddress,
    ) -> TestResult {
        trace!(target: "forge::test::fuzz", "executing invariant test for {:?}", func.name);
        let TestSetup { address, logs, traces, labeled_addresses, coverage, fuzz_fixtures, .. } =
            setup;

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
            invariant_config.clone(),
            identified_contracts,
            known_contracts,
        );
        let invariant_contract =
            InvariantContract { address, invariant_function: func, abi: &self.contract.abi };

        let mut logs = logs.clone();
        let mut traces = traces.clone();
        let mut coverage = coverage.clone();

        let failure_dir = invariant_config.clone().failure_dir(self.name);
        let failure_file = failure_dir.join(invariant_contract.invariant_function.clone().name);

        // Try to replay recorded failure if any.
        if let Ok(call_sequence) =
            foundry_common::fs::read_json_file::<Vec<BaseCounterExample>>(failure_file.as_path())
        {
            // Create calls from failed sequence and check if invariant still broken.
            let txes = call_sequence
                .clone()
                .into_iter()
                .map(|seq| BasicTxDetails {
                    sender: seq.sender.unwrap_or_default(),
                    call_details: CallDetails {
                        target: seq.addr.unwrap_or_default(),
                        calldata: seq.calldata,
                    },
                })
                .collect::<Vec<BasicTxDetails>>();
            if let Ok((success, replayed_entirely)) = check_sequence(
                self.executor.clone(),
                &txes,
                (0..min(txes.len(), invariant_config.depth as usize)).collect(),
                invariant_contract.address,
                invariant_contract.invariant_function.selector().to_vec().into(),
                invariant_config.fail_on_revert,
            ) {
                if !success {
                    // If sequence still fails then replay error to collect traces and
                    // exit without executing new runs.
                    let _ = replay_run(
                        &invariant_contract,
                        self.executor.clone(),
                        known_contracts,
                        identified_contracts.clone(),
                        &mut logs,
                        &mut traces,
                        &mut coverage,
                        txes,
                    );
                    return TestResult {
                        status: TestStatus::Failure,
                        reason: if replayed_entirely {
                            Some(format!(
                                "{} replay failure",
                                invariant_contract.invariant_function.name
                            ))
                        } else {
                            Some(format!(
                                "{} persisted failure revert",
                                invariant_contract.invariant_function.name
                            ))
                        },
                        decoded_logs: decode_console_logs(&logs),
                        traces,
                        coverage,
                        counterexample: Some(CounterExample::Sequence(call_sequence)),
                        kind: TestKind::Invariant { runs: 1, calls: 1, reverts: 1 },
                        duration: start.elapsed(),
                        ..Default::default()
                    }
                }
            }
        }

        let progress =
            start_fuzz_progress(self.progress, self.name, &func.name, invariant_config.runs);
        let InvariantFuzzTestResult { error, cases, reverts, last_run_inputs, gas_report_traces } =
            match evm.invariant_fuzz(invariant_contract.clone(), &fuzz_fixtures, progress.as_ref())
            {
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
        let success = error.is_none();
        let reason = error.as_ref().and_then(|err| err.revert_reason());

        match error {
            // If invariants were broken, replay the error to collect logs and traces
            Some(error) => match error {
                InvariantFuzzError::BrokenInvariant(case_data) |
                InvariantFuzzError::Revert(case_data) => {
                    // Replay error to create counterexample and to collect logs, traces and
                    // coverage.
                    match replay_error(
                        &case_data,
                        &invariant_contract,
                        self.executor.clone(),
                        known_contracts,
                        identified_contracts.clone(),
                        &mut logs,
                        &mut traces,
                        &mut coverage,
                        progress.as_ref(),
                    ) {
                        Ok(call_sequence) => {
                            if !call_sequence.is_empty() {
                                // Persist error in invariant failure dir.
                                if let Err(err) = foundry_common::fs::create_dir_all(failure_dir) {
                                    error!(%err, "Failed to create invariant failure dir");
                                } else if let Err(err) = foundry_common::fs::write_json_file(
                                    failure_file.as_path(),
                                    &call_sequence,
                                ) {
                                    error!(%err, "Failed to record call sequence");
                                }
                                counterexample = Some(CounterExample::Sequence(call_sequence))
                            }
                        }
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

    #[instrument(level = "debug", name = "fuzz", skip_all)]
    pub fn run_fuzz_test(
        &self,
        func: &Function,
        should_fail: bool,
        runner: TestRunner,
        setup: TestSetup,
        fuzz_config: FuzzConfig,
    ) -> TestResult {
        let TestSetup {
            address,
            mut logs,
            mut traces,
            mut labeled_addresses,
            mut coverage,
            fuzz_fixtures,
            ..
        } = setup;

        // Run fuzz test
        let progress = start_fuzz_progress(self.progress, self.name, &func.name, fuzz_config.runs);
        let start = Instant::now();
        let fuzzed_executor = FuzzedExecutor::new(
            self.executor.clone(),
            runner.clone(),
            self.sender,
            fuzz_config.clone(),
        );
        let result = fuzzed_executor.fuzz(
            func,
            &fuzz_fixtures,
            address,
            should_fail,
            self.revert_decoder,
            progress.as_ref(),
        );

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
            let debug_result =
                FuzzedExecutor::new(debug_executor, runner, self.sender, fuzz_config).single_fuzz(
                    address,
                    should_fail,
                    calldata,
                );

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
        (Some(old_coverage), Some(other)) => Some(old_coverage.merged(other)),
        (None, Some(other)) => Some(other),
        (Some(old_coverage), None) => Some(old_coverage),
        (None, None) => None,
    }
}
