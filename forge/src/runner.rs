use crate::{
    result::{SuiteResult, TestKind, TestResult, TestSetup},
    TestFilter, TestOptions,
};
use ethers::{
    abi::{Abi, Function},
    prelude::ArtifactId,
    types::{Address, Bytes, U256},
};
use eyre::Result;
use foundry_common::TestFunctionExt;
use foundry_evm::{
    executor::{CallResult, DeployResult, EvmError, Executor},
    fuzz::{
        invariant::{
            InvariantContract, InvariantExecutor, InvariantFuzzTestResult, InvariantTestOptions,
        },
        FuzzedExecutor,
    },
    trace::{load_contracts, TraceKind},
    CALLER,
};
use proptest::test_runner::{RngAlgorithm, TestError, TestRng, TestRunner};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{collections::BTreeMap, time::Instant};
use tracing::{error, trace};

/// A type that executes all tests of a contract
#[derive(Debug, Clone)]
pub struct ContractRunner<'a> {
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
        executor: Executor,
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

impl<'a> ContractRunner<'a> {
    /// Deploys the test contract inside the runner from the sending account, and optionally runs
    /// the `setUp` function on the test contract.
    pub fn setup(&mut self, setup: bool) -> Result<TestSetup> {
        trace!(?setup, "Setting test contract");

        // We max out their balance so that they can deploy and make calls.
        self.executor.set_balance(self.sender, U256::MAX);
        self.executor.set_balance(CALLER, U256::MAX);

        // We set the nonce of the deployer accounts to 1 to get the same addresses as DappTools
        self.executor.set_nonce(self.sender, 1);

        // Deploy libraries
        let mut traces = Vec::with_capacity(self.predeploy_libs.len());
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
            Err(EvmError::Execution { reason, traces, logs, labels, .. }) => {
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
        self.executor.set_balance(address, self.initial_balance);
        self.executor.set_balance(self.sender, self.initial_balance);
        self.executor.set_balance(CALLER, self.initial_balance);

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
                    Err(EvmError::Execution { traces, labels, logs, reason, .. }) => {
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
        known_contracts: Option<&BTreeMap<ArtifactId, (Abi, Vec<u8>)>>,
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
                        kind: TestKind::Standard(0),
                        traces: vec![],
                        coverage: None,
                        labeled_addresses: BTreeMap::new(),
                    },
                )]
                .into(),
                warnings,
            ))
        }

        let has_invariants =
            self.contract.functions().into_iter().any(|func| func.name.is_invariant_test());

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
                        logs: setup.logs,
                        kind: TestKind::Standard(0),
                        traces: setup.traces,
                        coverage: None,
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
            .filter(|func| func.is_test() && filter.matches_test(func.signature()))
            .map(|func| (func, func.is_test_fail()))
            .collect();

        let mut test_results = BTreeMap::new();
        if !tests.is_empty() {
            // TODO: Add Options to modify the persistence
            let cfg = proptest::test_runner::Config {
                failure_persistence: None,
                cases: test_options.fuzz_runs,
                max_local_rejects: test_options.fuzz_max_local_rejects,
                max_global_rejects: test_options.fuzz_max_global_rejects,
                ..Default::default()
            };

            let fuzzer = if let Some(ref fuzz_seed) = test_options.fuzz_seed {
                let mut bytes: [u8; 32] = [0; 32];
                fuzz_seed.to_big_endian(&mut bytes);
                trace!(target: "forge::test", "executing test command");
                let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &bytes);
                proptest::test_runner::TestRunner::new_with_rng(cfg, rng)
            } else {
                proptest::test_runner::TestRunner::new(cfg)
            };

            test_results.extend(
                tests
                    .par_iter()
                    .flat_map(|(func, should_fail)| {
                        let result = if func.is_fuzz_test() {
                            self.run_fuzz_test(func, *should_fail, fuzzer.clone(), setup.clone())
                        } else {
                            self.clone().run_test(func, *should_fail, setup.clone())
                        };

                        result.map(|result| Ok((func.signature(), result)))
                    })
                    .collect::<Result<BTreeMap<_, _>>>()?,
            );
        }

        if has_invariants {
            // TODO: Add Options to modify the persistence
            let cfg = proptest::test_runner::Config {
                failure_persistence: None,
                cases: test_options.invariant_runs,
                max_local_rejects: test_options.fuzz_max_local_rejects,
                max_global_rejects: test_options.fuzz_max_global_rejects,
                ..Default::default()
            };

            let fuzzer = if let Some(ref fuzz_seed) = test_options.fuzz_seed {
                let mut bytes: [u8; 32] = [0; 32];
                fuzz_seed.to_big_endian(&mut bytes);
                trace!(target: "forge::test", "executing invariant test command");
                let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &bytes);
                proptest::test_runner::TestRunner::new_with_rng(cfg, rng)
            } else {
                proptest::test_runner::TestRunner::new(cfg)
            };

            let identified_contracts = load_contracts(setup.traces.clone(), known_contracts);
            let functions: Vec<&Function> = self
                .contract
                .functions()
                .into_iter()
                .filter(|func| {
                    func.name.is_invariant_test() && filter.matches_test(func.signature())
                })
                .collect();

            let results = self.run_invariant_test(
                fuzzer,
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

    /// Runs a single test
    ///
    /// Calls the given functions and returns the `TestResult`.
    ///
    /// State modifications are not committed to the evm database but discarded after the call,
    /// similar to `eth_call`.
    #[tracing::instrument(name = "test", skip_all, fields(name = %func.signature(), %should_fail))]
    pub fn run_test(
        mut self,
        func: &Function,
        should_fail: bool,
        setup: TestSetup,
    ) -> Result<TestResult> {
        let TestSetup { address, mut logs, mut traces, mut labeled_addresses, .. } = setup;

        // Run unit test
        let start = Instant::now();
        let (reverted, reason, gas, stipend, execution_traces, coverage, state_changeset) =
            match self.executor.execute_test::<(), _, _>(
                self.sender,
                address,
                func.clone(),
                (),
                0.into(),
                self.errors,
            ) {
                Ok(CallResult {
                    reverted,
                    gas,
                    stipend,
                    logs: execution_logs,
                    traces: execution_trace,
                    coverage,
                    labels: new_labels,
                    state_changeset,
                    ..
                }) => {
                    labeled_addresses.extend(new_labels);
                    logs.extend(execution_logs);
                    (reverted, None, gas, stipend, execution_trace, coverage, state_changeset)
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
                    (reverted, Some(reason), gas, stipend, execution_trace, None, state_changeset)
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
            coverage,
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

        let mut evm = InvariantExecutor::new(
            &mut self.executor,
            runner,
            &identified_contracts,
            project_contracts,
        );

        let invariant_contract =
            InvariantContract { address, invariant_functions: functions, abi: self.contract };

        if let Some(InvariantFuzzTestResult { invariants, cases, reverts }) = evm.invariant_fuzz(
            invariant_contract,
            InvariantTestOptions {
                depth: test_options.invariant_depth,
                fail_on_revert: test_options.invariant_fail_on_revert,
                call_override: test_options.invariant_call_override,
            },
        )? {
            let results = invariants
                .iter()
                .map(|(_, test_error)| {
                    let mut counterexample = None;
                    let mut logs = logs.clone();
                    let mut traces = traces.clone();

                    if let Some(ref error) = test_error {
                        if let TestError::Fail(_, _) = &error.test_error {
                            counterexample = error.replay(
                                self.executor.clone(),
                                known_contracts,
                                identified_contracts.clone(),
                                &mut logs,
                                &mut traces,
                            );
                        }
                    }

                    TestResult {
                        success: test_error.is_none(),
                        reason: test_error.as_ref().and_then(|err| {
                            (!err.revert_reason.is_empty()).then(|| err.revert_reason.clone())
                        }),
                        counterexample,
                        logs,
                        kind: TestKind::Invariant(cases.clone(), reverts),
                        coverage: None, // todo?
                        traces,
                        labeled_addresses: labeled_addresses.clone(),
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
            // TODO: Maybe support coverage for fuzz tests
            coverage: None,
            labeled_addresses,
        })
    }
}
