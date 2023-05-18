use crate::{
    result::{SuiteResult, TestKind, TestResult, TestSetup},
    TestFilter, TestOptions,
};
use ethers::{
    abi::{Abi, Function},
    types::{Address, Bytes, U256},
};
use eyre::{Result, WrapErr};
use foundry_common::{
    contracts::{ContractsByAddress, ContractsByArtifact},
    TestFunctionExt,
};
use foundry_config::{FuzzConfig, InvariantConfig};
use foundry_evm::{
    decode::decode_console_logs,
    executor::{CallResult, DeployResult, EvmError, ExecutionErr, Executor},
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
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{
    collections::{BTreeMap, HashMap},
    time::Instant,
};
use tracing::{error, trace};

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
    pub fn setup(&mut self, setup: bool) -> Result<TestSetup> {
        trace!(?setup, "Setting test contract");

        // We max out their balance so that they can deploy and make calls.
        self.executor.set_balance(self.sender, U256::MAX)?;
        self.executor.set_balance(CALLER, U256::MAX)?;

        // We set the nonce of the deployer accounts to 1 to get the same addresses as DappTools
        self.executor.set_nonce(self.sender, 1)?;

        // Deploy libraries
        let mut traces = Vec::with_capacity(self.predeploy_libs.len());
        for code in self.predeploy_libs.iter() {
            match self.executor.deploy(self.sender, code.0.clone(), 0u32.into(), self.errors) {
                Ok(DeployResult { traces: tmp_traces, .. }) => {
                    if let Some(tmp_traces) = tmp_traces {
                        traces.push((TraceKind::Deployment, tmp_traces));
                    }
                }
                Err(EvmError::Execution(err)) => {
                    let ExecutionErr { reason, traces, logs, labels, .. } = *err;
                    // If we failed to call the constructor, force the tracekind to be setup so
                    // a trace is shown.
                    let traces =
                        traces.map(|traces| vec![(TraceKind::Setup, traces)]).unwrap_or_default();

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
            Err(EvmError::Execution(err)) => {
                let ExecutionErr { reason, traces, logs, labels, .. } = *err;
                let traces =
                    traces.map(|traces| vec![(TraceKind::Setup, traces)]).unwrap_or_default();

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

        // Now we set the contracts initial balance, and we also reset `self.sender`s and `CALLER`s
        // balance to the initial balance we want
        self.executor.set_balance(address, self.initial_balance)?;
        self.executor.set_balance(self.sender, self.initial_balance)?;
        self.executor.set_balance(CALLER, self.initial_balance)?;

        self.executor.deploy_create2_deployer()?;

        // Optionally call the `setUp` function
        let setup = if setup {
            trace!("setting up");
            let (setup_failed, setup_logs, setup_traces, labeled_addresses, reason) =
                match self.executor.setup(None, address) {
                    Ok(CallResult { traces, labels, logs, .. }) => {
                        trace!(contract=?address, "successfully setUp test");
                        (false, logs, traces, labels, None)
                    }
                    Err(EvmError::Execution(err)) => {
                        let ExecutionErr { traces, labels, logs, reason, .. } = *err;
                        error!(reason=?reason, contract= ?address, "setUp failed");
                        (true, logs, traces, labels, Some(format!("Setup failed: {reason}")))
                    }
                    Err(err) => {
                        error!(reason=?err, contract= ?address, "setUp failed");
                        (
                            true,
                            Vec::new(),
                            None,
                            BTreeMap::new(),
                            Some(format!("Setup failed: {}", &err.to_string())),
                        )
                    }
                };
            traces.extend(setup_traces.map(|traces| (TraceKind::Setup, traces)).into_iter());
            logs.extend(setup_logs);

            TestSetup { address, logs, traces, labeled_addresses, setup_failed, reason }
        } else {
            TestSetup { address, logs, traces, ..Default::default() }
        };

        Ok(setup)
    }

    /// Runs all tests for a contract whose names match the provided regular expression
    pub fn run_tests(
        mut self,
        filter: &impl TestFilter,
        test_options: TestOptions,
        known_contracts: Option<&ContractsByArtifact>,
    ) -> Result<SuiteResult> {
        tracing::info!("starting tests");
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
            return Ok(SuiteResult::new(
                start.elapsed(),
                [(
                    "setUp()".to_string(),
                    TestResult {
                        success: false,
                        reason: Some("Multiple setUp functions".to_string()),
                        counterexample: None,
                        logs: vec![],
                        decoded_logs: vec![],
                        kind: TestKind::Standard(0),
                        traces: vec![],
                        coverage: None,
                        labeled_addresses: BTreeMap::new(),
                        // TODO-f: get the breakpoints here
                        breakpoints: Default::default(),
                    },
                )]
                .into(),
                warnings,
            ))
        }

        let has_invariants = self.contract.functions().any(|func| func.name.is_invariant_test());

        // Invariant testing requires tracing to figure out what contracts were created.
        let original_tracing = self.executor.inspector_config().tracing;
        if has_invariants && needs_setup {
            self.executor.set_tracing(true);
        }

        let setup = self.setup(needs_setup)?;
        self.executor.set_tracing(original_tracing);

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
                        decoded_logs: decode_console_logs(&setup.logs),
                        logs: setup.logs,
                        kind: TestKind::Standard(0),
                        traces: setup.traces,
                        coverage: None,
                        labeled_addresses: setup.labeled_addresses,
                        breakpoints: Default::default(),
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
            .filter(|func| func.is_test() && filter.matches_test(func.signature()))
            .collect();

        let mut test_results = BTreeMap::new();
        if !tests.is_empty() {
            test_results.extend(
                tests
                    .par_iter()
                    .flat_map(|func| {
                        if func.is_fuzz_test() {
                            let fn_name = &func.name;
                            let runner = test_options.fuzz_runner(self.name, fn_name);
                            let fuzz_config = test_options.fuzz_config(self.name, fn_name);

                            self.run_fuzz_test(
                                func,
                                runner,
                                setup.clone(),
                                *fuzz_config,
                            )
                        } else {
                            self.clone().run_test(func, setup.clone())
                        }
                        .map(|result| Ok((func.signature(), result)))
                    })
                    .collect::<Result<BTreeMap<_, _>>>()?,
            );
        }

        if has_invariants {
            let identified_contracts = load_contracts(setup.traces.clone(), known_contracts);

            let functions: Vec<&Function> = self
                .contract
                .functions()
                .filter(|func| {
                    func.name.is_invariant_test() && filter.matches_test(func.signature())
                })
                .collect();

            let mut results: Vec<TestResult> = vec![];

            for func in functions.iter() {
                let fn_name = &func.name;
                let runner = test_options.invariant_runner(self.name, fn_name);
                let invariant_config = test_options.invariant_config(self.name, fn_name);

                let invariant_results = self.run_invariant_test(
                    runner,
                    setup.clone(),
                    *invariant_config,
                    vec![func],
                    known_contracts,
                    identified_contracts.clone(),
                )?;

                for test_result in invariant_results {
                    results.push(test_result);
                }
            }

            results.into_iter().zip(functions.iter()).for_each(|(result, function)| {
                match result.kind {
                    TestKind::Invariant { .. } => {
                        test_results.insert(function.signature(), result);
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

    /// Runs a single test
    ///
    /// Calls the given functions and returns the `TestResult`.
    ///
    /// State modifications are not committed to the evm database but discarded after the call,
    /// similar to `eth_call`.
    #[tracing::instrument(name = "test", skip_all, fields(name = %func.signature()))]
    pub fn run_test(
        mut self,
        func: &Function,
        setup: TestSetup,
    ) -> Result<TestResult> {
        let TestSetup { address, mut logs, mut traces, mut labeled_addresses, .. } = setup;

        // Run unit test
        let start = Instant::now();
        let (
            reverted,
            reason,
            gas,
            stipend,
            execution_traces,
            coverage,
            state_changeset,
            breakpoints,
        ) = match self.executor.execute_test::<(), _, _>(
            self.sender,
            address,
            func.clone(),
            (),
            0.into(),
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
                breakpoints,
                ..
            }) => {
                labeled_addresses.extend(new_labels);
                logs.extend(execution_logs);
                (
                    reverted,
                    None,
                    gas,
                    stipend,
                    execution_trace,
                    coverage,
                    state_changeset,
                    breakpoints,
                )
            }
            Err(EvmError::Execution(err)) => {
                let ExecutionErr {
                    reverted,
                    reason,
                    gas_used: gas,
                    stipend,
                    logs: execution_logs,
                    traces: execution_trace,
                    labels: new_labels,
                    state_changeset,
                    ..
                } = *err;
                labeled_addresses.extend(new_labels);
                logs.extend(execution_logs);
                (
                    reverted,
                    Some(reason),
                    gas,
                    stipend,
                    execution_trace,
                    None,
                    state_changeset,
                    HashMap::new(),
                )
            }
            Err(err) => {
                error!(?err);
                return Err(err.into())
            }
        };
        traces.extend(execution_traces.map(|traces| (TraceKind::Execution, traces)).into_iter());

        let success = self.executor.is_success(
            setup.address,
            reverted,
            state_changeset.expect("we should have a state changeset"),
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
            decoded_logs: decode_console_logs(&logs),
            logs,
            kind: TestKind::Standard(gas.overflowing_sub(stipend).0),
            traces,
            coverage,
            labeled_addresses,
            breakpoints,
        })
    }

    #[tracing::instrument(name = "invariant-test", skip_all)]
    pub fn run_invariant_test(
        &mut self,
        runner: TestRunner,
        setup: TestSetup,
        invariant_config: InvariantConfig,
        functions: Vec<&Function>,
        known_contracts: Option<&ContractsByArtifact>,
        identified_contracts: ContractsByAddress,
    ) -> Result<Vec<TestResult>> {
        trace!(target: "forge::test::fuzz", "executing invariant test with invariant functions {:?}",  functions.iter().map(|f|&f.name).collect::<Vec<_>>());
        let empty = ContractsByArtifact::default();
        let project_contracts = known_contracts.unwrap_or(&empty);
        let TestSetup { address, logs, traces, labeled_addresses, .. } = setup;

        let mut evm = InvariantExecutor::new(
            &mut self.executor,
            runner,
            invariant_config,
            &identified_contracts,
            project_contracts,
        );

        let invariant_contract =
            InvariantContract { address, invariant_functions: functions, abi: self.contract };

        if let Some(InvariantFuzzTestResult { invariants, cases, reverts, mut last_call_results }) =
            evm.invariant_fuzz(invariant_contract)?
        {
            let results = invariants
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
                        Some(
                            error @ InvariantFuzzError { test_error: TestError::Fail(_, _), .. },
                        ) => {
                            counterexample = error.replay(
                                self.executor.clone(),
                                known_contracts,
                                identified_contracts.clone(),
                                &mut logs,
                                &mut traces,
                            )?;

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

                    Ok(TestResult {
                        success,
                        reason,
                        counterexample,
                        decoded_logs: decode_console_logs(&logs),
                        logs,
                        kind,
                        coverage: None, // todo?
                        traces,
                        labeled_addresses: labeled_addresses.clone(),
                        breakpoints: Default::default(),
                    })
                })
                .collect::<Result<Vec<TestResult>>>()
                .wrap_err("Failed to replay counter examples")?;

            Ok(results)
        } else {
            Ok(vec![])
        }
    }

    #[tracing::instrument(name = "fuzz-test", skip_all, fields(name = %func.signature()))]
    pub fn run_fuzz_test(
        &self,
        func: &Function,
        runner: TestRunner,
        setup: TestSetup,
        fuzz_config: FuzzConfig,
    ) -> Result<TestResult> {
        let TestSetup { address, mut logs, mut traces, mut labeled_addresses, .. } = setup;

        // Run fuzz test
        let start = Instant::now();
        let mut result = FuzzedExecutor::new(&self.executor, runner, self.sender, fuzz_config)
            .fuzz(func, address, self.errors)
            .wrap_err("Failed to run fuzz test")?;

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
        tracing::debug!(
            duration = ?start.elapsed(),
            success = %result.success
        );

        Ok(TestResult {
            success: result.success,
            reason: result.reason,
            counterexample: result.counterexample,
            decoded_logs: decode_console_logs(&logs),
            logs,
            kind,
            traces,
            coverage: result.coverage,
            labeled_addresses,
            breakpoints: Default::default(),
        })
    }
}
