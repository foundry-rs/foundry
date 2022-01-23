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
use std::{collections::BTreeMap, fmt, time::Instant};

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

        // deploy an instance of the contract inside the runner in the EVM
        let output =
            executor.deploy(self.sender, self.code.clone(), 0u32.into()).expect("couldn't deploy");
        executor.set_balance(output.retdata, self.evm_opts.initial_balance);
        Ok((output.retdata, executor, output.logs))
    }

    /// Runs all tests for a contract whose names match the provided regular expression
    pub fn run_tests(
        &mut self,
        filter: &impl TestFilter,
        fuzzer: Option<TestRunner>,
    ) -> Result<BTreeMap<String, TestResult>> {
        tracing::info!("starting tests");
        let start = Instant::now();
        let needs_setup = self.contract.functions().any(|func| func.name == "setUp");

        let setup = self.setup(needs_setup)?;
        if setup.setup_failed {
            // The setup failed, so we return a single test result for `setUp`
            return Ok([(
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
            .into())
        }

        // Collect valid test functions
        let tests: Vec<_> = self
            .contract
            .functions()
            .into_iter()
            .filter(|func| func.name.starts_with("test") && filter.matches_test(&func.name))
            .map(|func| (func, func.name.starts_with("testFail")))
            .collect();

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

        if !test_results.is_empty() {
            let successful = test_results.iter().filter(|(_, tst)| tst.success).count();
            tracing::info!(
                duration = ?Instant::now().duration_since(start),
                "done. {}/{} successful",
                successful,
                test_results.len()
            );
        }
        Ok(test_results)
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
        // the expected result depends on the function name
        // DAppTools' ds-test will not revert inside its `assertEq`-like functions
        // which allows to test multiple assertions in 1 test function while also
        // preserving logs.
        let should_fail = func.name.starts_with("testFail");
        tracing::debug!(func = ?func.signature(), should_fail, "unit-testing");

        let (address, mut evm, init_logs) = self.new_sputnik_evm()?;

        let errors_abi = self.execution_info.as_ref().map(|(_, _, errors)| errors);
        let errors_abi = if let Some(ref abi) = errors_abi { abi } else { self.contract };

        let mut logs = init_logs;

        let mut traces: Option<Vec<CallTraceArena>> = None;
        let mut identified_contracts: Option<BTreeMap<Address, (String, Abi)>> = None;

        // clear out the deployment trace
        evm.reset_traces();

        // call the setup function in each test to reset the test's state.
        if setup {
            tracing::trace!("setting up");
            let setup_logs = match evm.setup(address) {
                Ok((_reason, setup_logs)) => setup_logs,
                Err(e) => {
                    // if tracing is enabled, just return it as a failed test
                    // otherwise abort
                    if evm.tracing_enabled() {
                        self.update_traces(
                            &mut traces,
                            &mut identified_contracts,
                            known_contracts,
                            setup,
                            &mut evm,
                        );
                    }

                    return Ok(TestResult {
                        success: false,
                        reason: Some("Setup failed: ".to_string() + &e.to_string()),
                        gas_used: 0,
                        counterexample: None,
                        logs,
                        kind: TestKind::Standard(0),
                        traces,
                        identified_contracts,
                        debug_calls: if evm.state().debug_enabled {
                            Some(evm.debug_calls())
                        } else {
                            None
                        },
                        labeled_addresses: evm.state().labels.clone(),
                    })
                }
            };
            logs.extend_from_slice(&setup_logs);
        }

        let (status, reason, gas_used, logs) = match evm.call::<(), _, _>(
            self.sender,
            address,
            func.clone(),
            (),
            0.into(),
            Some(errors_abi),
        ) {
            Ok(output) => {
                logs.extend(output.logs.clone());
                (output.status, None, output.gas, output.logs)
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
            duration = ?Instant::now().duration_since(start),
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
            duration = ?Instant::now().duration_since(start),
            success = %result.success
        );

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
                        self.update_traces(
                            &mut traces,
                            &mut identified_contracts,
                            known_contracts,
                            setup,
                            &mut evm,
                        );
                    }
                    return Ok(TestResult {
                        success: false,
                        reason: Some("Setup failed: ".to_string() + &e.to_string()),
                        gas_used: 0,
                        counterexample: None,
                        logs: vec![],
                        kind: TestKind::Fuzz(FuzzedCases::new(vec![])),
                        traces,
                        identified_contracts,
                        debug_calls: if evm.state().debug_enabled {
                            Some(evm.debug_calls())
                        } else {
                            None
                        },
                        labeled_addresses: evm.state().labels.clone(),
                    })
                }
            }
        }

        let mut logs = init_logs;

        let prev = evm.set_tracing_enabled(false);

        // instantiate the fuzzed evm in line
        let evm = FuzzedExecutor::new(&mut evm, runner, self.sender);
        let FuzzTestResult { cases, test_error } =
            evm.fuzz(func, address, should_fail, Some(self.contract));

        let evm = evm.into_inner();
        if let Some(ref error) = test_error {
            // we want traces for a failed fuzz
            if let TestError::Fail(_reason, bytes) = &error.test_error {
                if prev {
                    let _ = evm.set_tracing_enabled(true);
                }
                let output = evm.call_raw(self.sender, address, bytes.clone(), 0.into(), false)?;
                if is_fail(evm, output.status) {
                    logs.extend(output.logs);
                    // add reverted logs
                    logs.extend(evm.all_logs());
                } else {
                    logs.extend(output.logs);
                }
                self.update_traces(
                    &mut traces,
                    &mut identified_contracts,
                    known_contracts,
                    setup,
                    evm,
                );
            }
        }

        let success = test_error.is_none();
        let mut counterexample = None;
        let mut reason = None;
        if let Some(err) = test_error {
            match err.test_error {
                TestError::Fail(_, value) => {
                    // skip the function selector when decoding
                    let args = func.decode_input(&value.as_ref()[4..])?;
                    let counter = CounterExample { calldata: value.clone(), args };
                    counterexample = Some(counter);
                    tracing::info!("Found minimal failing case: {}", hex::encode(&value));
                }
                result => panic!("Unexpected test result: {:?}", result),
            }
            if !err.revert_reason.is_empty() {
                reason = Some(err.revert_reason);
            }
        }

        let duration = Instant::now().duration_since(start);
        tracing::debug!(?duration, %success);

        // from that call?
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
}
