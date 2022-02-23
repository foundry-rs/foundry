use crate::{
    executor::{
        fuzz::{FuzzError, FuzzTestResult, FuzzedCases, FuzzedExecutor},
        CallResult, EvmError, Executor, RawCallResult,
    },
    TestFilter,
};
use rayon::iter::ParallelIterator;
use revm::db::DatabaseRef;

use ethers::{
    abi::{Abi, Event, Function, RawLog, Token},
    types::{Address, Bytes, H256, U256},
};

use eyre::Result;
use std::{collections::BTreeMap, fmt, time::Instant};

use proptest::test_runner::{TestError, TestRunner};
use rayon::iter::IntoParallelRefIterator;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CounterExample {
    pub calldata: Bytes,

    // Token does not implement Serde (lol), so we just serialize the calldata
    #[serde(skip)]
    pub args: Vec<Token>,
}

impl fmt::Display for CounterExample {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let args = foundry_utils::format_tokens(&self.args).collect::<Vec<_>>().join(", ");
        write!(f, "calldata=0x{}, args=[{}]", hex::encode(&self.calldata), args)
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

    /// The gas used during execution.
    ///
    /// If this is the result of a fuzz test (`TestKind::Fuzz`), then this is the median of all
    /// successful cases
    // TODO: The gas usage is both in TestKind and here. We should dedupe.
    pub gas_used: u64,

    /// Minimal reproduction test case for failing fuzz tests
    pub counterexample: Option<CounterExample>,

    /// Any captured & parsed as strings logs along the test's execution which should
    /// be printed to the user.
    #[serde(skip)]
    pub logs: Vec<RawLog>,

    /// What kind of test this was
    pub kind: TestKind,

    /// Traces
    // TODO
    //pub traces: Option<Vec<CallTraceArena>>,
    traces: Option<Vec<()>>,

    /// Identified contracts
    pub identified_contracts: Option<BTreeMap<Address, (String, Abi)>>,

    /// Debug Steps
    // TODO
    #[serde(skip)]
    //pub debug_calls: Option<Vec<DebugArena>>,
    pub debug_calls: Option<Vec<()>>,

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
                median: fuzzed.median_gas(),
                mean: fuzzed.mean_gas(),
            },
        }
    }
}

/// Type complexity wrapper around execution info
type MaybeExecutionInfo<'a> =
    Option<(&'a BTreeMap<[u8; 4], Function>, &'a BTreeMap<H256, Event>, &'a Abi)>;

// TODO: Get rid of known contracts, execution info and so on once we rewrite tracing, since we are
// moving all the decoding/display logic to the CLI. Traces and logs returned from the runner (and
// consequently the multi runner) are in a raw (but digestible) format.
pub struct ContractRunner<'a, DB: DatabaseRef> {
    /// The executor used by the runner.
    pub executor: Executor<DB>,

    // Contract deployment options
    /// The deployed contract's ABI
    pub contract: &'a Abi,
    /// The deployed contract's code
    // This is cheap to clone due to [`bytes::Bytes`], so OK to own
    pub code: Bytes,
    /// The initial balance of the test contract
    pub initial_balance: U256,
    /// The address which will be used as the `from` field in all EVM calls
    pub sender: Address,

    /// Contract execution info, (functions, events, errors)
    pub execution_info: MaybeExecutionInfo<'a>,
    /// library contracts to be deployed before this contract
    pub predeploy_libs: &'a [Bytes],
}

impl<'a, DB: DatabaseRef> ContractRunner<'a, DB> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        executor: Executor<DB>,
        contract: &'a Abi,
        code: Bytes,
        initial_balance: U256,
        sender: Option<Address>,
        execution_info: MaybeExecutionInfo<'a>,
        predeploy_libs: &'a [Bytes],
    ) -> Self {
        Self {
            executor,
            contract,
            code,
            initial_balance,
            sender: sender.unwrap_or_default(),
            execution_info,
            predeploy_libs,
        }
    }
}

impl<'a, DB: DatabaseRef + Clone + Send + Sync> ContractRunner<'a, DB> {
    /// Deploys the test contract inside the runner from the sending account, and optionally runs
    /// the `setUp` function on the test contract.
    pub fn deploy(&mut self, setup: bool) -> Result<(Address, Vec<RawLog>, bool, Option<String>)> {
        // We set the nonce of the deployer accounts to 1 to get the same addresses as DappTools
        self.executor.set_nonce(self.sender, 1);

        // Deploy libraries
        self.predeploy_libs.iter().for_each(|code| {
            self.executor
                .deploy(Address::zero(), code.0.clone(), 0u32.into())
                .expect("couldn't deploy library");
        });

        // Deploy an instance of the contract
        let (addr, _, _, mut logs) = self
            .executor
            .deploy(self.sender, self.code.0.clone(), 0u32.into())
            .expect("couldn't deploy");
        self.executor.set_balance(addr, self.initial_balance);

        // Optionally call the `setUp` function
        if setup {
            tracing::trace!("setting up");
            let (setup_failed, setup_logs, reason) = match self.executor.setup(addr) {
                Ok((_, logs)) => (false, logs, None),
                Err(EvmError::Execution { logs, reason, .. }) => {
                    (true, logs, Some(format!("Setup failed: {}", reason)))
                }
                Err(e) => (true, Vec::new(), Some(format!("Setup failed: {}", &e.to_string()))),
            };
            logs.extend_from_slice(&setup_logs);
            Ok((addr, logs, setup_failed, reason))
        } else {
            Ok((addr, logs, false, None))
        }
    }

    /// Runs all tests for a contract whose names match the provided regular expression
    pub fn run_tests(
        &mut self,
        filter: &impl TestFilter,
        fuzzer: Option<TestRunner>,
        known_contracts: Option<&BTreeMap<String, (Abi, Vec<u8>)>>,
    ) -> Result<BTreeMap<String, TestResult>> {
        tracing::info!("starting tests");
        let start = Instant::now();
        let needs_setup = self.contract.functions().any(|func| func.name == "setUp");
        let (unit_tests, fuzz_tests): (Vec<_>, Vec<_>) = self
            .contract
            .functions()
            .into_iter()
            .filter(|func| func.name.starts_with("test"))
            .filter(|func| filter.matches_test(&func.name))
            .partition(|func| func.inputs.is_empty());

        let (addr, init_logs, setup_failed, reason) = self.deploy(needs_setup)?;
        if setup_failed {
            // The setup failed, so we return a single test result for `setUp`
            return Ok([(
                "setUp()".to_string(),
                TestResult {
                    success: false,
                    reason,
                    gas_used: 0,
                    counterexample: None,
                    logs: init_logs,
                    kind: TestKind::Standard(0),
                    traces: None,
                    identified_contracts: None,
                    debug_calls: None,
                    labeled_addresses: Default::default(),
                },
            )]
            .into())
        }

        // Run all unit tests
        let mut test_results = unit_tests
            .par_iter()
            .map(|func| {
                let result = self.run_test(func, known_contracts, addr, init_logs.clone())?;
                Ok((func.signature(), result))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;

        if let Some(fuzzer) = fuzzer {
            let fuzz_results = fuzz_tests
                .par_iter()
                .filter(|func| !func.inputs.is_empty())
                .map(|func| {
                    let result = self.run_fuzz_test(
                        func,
                        fuzzer.clone(),
                        known_contracts,
                        addr,
                        init_logs.clone(),
                    )?;
                    Ok((func.signature(), result))
                })
                .collect::<Result<BTreeMap<_, _>>>()?;
            test_results.extend(fuzz_results);
        }

        if !test_results.is_empty() {
            let successful = test_results.iter().filter(|(_, tst)| tst.success).count();
            let duration = Instant::now().duration_since(start);
            tracing::info!(?duration, "done. {}/{} successful", successful, test_results.len());
        }
        Ok(test_results)
    }

    #[tracing::instrument(name = "test", skip_all, fields(name = %func.signature()))]
    pub fn run_test(
        &self,
        func: &Function,
        _known_contracts: Option<&BTreeMap<String, (Abi, Vec<u8>)>>,
        address: Address,
        mut logs: Vec<RawLog>,
    ) -> Result<TestResult> {
        let start = Instant::now();
        // The expected result depends on the function name.
        // TODO: Dedupe (`TestDescriptor`?)
        let should_fail = func.name.starts_with("testFail");
        tracing::debug!(func = ?func.signature(), should_fail, "unit-testing");

        let errors_abi = self.execution_info.as_ref().map(|(_, _, errors)| errors);
        let errors_abi = if let Some(ref abi) = errors_abi { abi } else { self.contract };

        let identified_contracts: Option<BTreeMap<Address, (String, Abi)>> = None;

        let (status, reason, gas_used, logs, state_changeset) = match self
            .executor
            .call::<(), _, _>(self.sender, address, func.clone(), (), 0.into(), Some(errors_abi))
        {
            Ok(CallResult {
                status, gas: gas_used, logs: execution_logs, state_changeset, ..
            }) => {
                logs.extend(execution_logs);
                (status, None, gas_used, logs, state_changeset)
            }
            Err(err) => match err {
                EvmError::Execution {
                    status,
                    reason,
                    gas_used,
                    logs: execution_logs,
                    state_changeset,
                } => {
                    logs.extend(execution_logs);
                    (status, Some(reason), gas_used, logs, state_changeset)
                }
                err => {
                    tracing::error!(?err);
                    return Err(err.into())
                }
            },
        };

        // DSTest will not revert inside its `assertEq`-like functions
        // which allows to test multiple assertions in 1 test function while also
        // preserving logs - instead it sets `failed` to `true` which we must check.
        let success = self.executor.is_success(
            address,
            status,
            state_changeset.expect("we should have a state changeset"),
            should_fail,
        );
        let duration = Instant::now().duration_since(start);
        tracing::debug!(?duration, %success, %gas_used);

        Ok(TestResult {
            success,
            reason,
            gas_used,
            counterexample: None,
            logs,
            kind: TestKind::Standard(gas_used),
            traces: None,
            identified_contracts,
            debug_calls: None,
            labeled_addresses: Default::default(),
        })
    }

    #[tracing::instrument(name = "fuzz-test", skip_all, fields(name = %func.signature()))]
    pub fn run_fuzz_test(
        &self,
        func: &Function,
        runner: TestRunner,
        _known_contracts: Option<&BTreeMap<String, (Abi, Vec<u8>)>>,
        address: Address,
        mut logs: Vec<RawLog>,
    ) -> Result<TestResult> {
        // We do not trace in fuzz tests as it is a big performance hit
        let start = Instant::now();
        // TODO: Dedupe (`TestDescriptor`?)
        let should_fail = func.name.starts_with("testFail");

        let identified_contracts: Option<BTreeMap<Address, (String, Abi)>> = None;

        // Wrap the executor in a fuzzed version
        // TODO: When tracing is ported, we should disable it here.
        let executor = FuzzedExecutor::new(&self.executor, runner, self.sender);
        let FuzzTestResult { cases, test_error } =
            executor.fuzz(func, address, should_fail, Some(self.contract));

        // Rerun the failed fuzz case to get more information like traces and logs
        if let Some(FuzzError { test_error: TestError::Fail(_, ref calldata), .. }) = test_error {
            // TODO: When tracing is ported, we should re-enable it here to get traces.
            let RawCallResult { logs: execution_logs, .. } =
                self.executor.call_raw(self.sender, address, calldata.0.clone(), 0.into())?;
            logs.extend(execution_logs);
        }

        let (success, counterexample, reason) = match test_error {
            Some(err) => {
                let (counterexample, reason) = match err.test_error {
                    TestError::Abort(r) if r == "Too many global rejects".into() => {
                        (None, Some(r.message().to_string()))
                    }
                    TestError::Fail(_, calldata) => {
                        // Skip the function selector when decoding
                        let args = func.decode_input(&calldata.as_ref()[4..])?;

                        (Some(CounterExample { calldata, args }), None)
                    }
                    e => panic!("Unexpected test error: {:?}", e),
                };

                if !err.revert_reason.is_empty() {
                    (false, counterexample, Some(err.revert_reason))
                } else {
                    (false, counterexample, reason)
                }
            }
            _ => (true, None, None),
        };

        let duration = Instant::now().duration_since(start);
        tracing::debug!(?duration, %success);

        Ok(TestResult {
            success,
            reason,
            gas_used: cases.median_gas(),
            counterexample,
            logs,
            kind: TestKind::Fuzz(cases),
            traces: Default::default(),
            identified_contracts,
            debug_calls: None,
            labeled_addresses: Default::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::test_helpers::{test_executor, Filter, COMPILED, EVM_OPTS};

    use super::*;
    use proptest::test_runner::Config as FuzzConfig;
    use revm::db::EmptyDB;

    pub fn runner<'a>(
        abi: &'a Abi,
        code: ethers::prelude::Bytes,
        libs: &'a mut Vec<ethers::prelude::Bytes>,
    ) -> ContractRunner<'a, EmptyDB> {
        ContractRunner::new(
            test_executor(),
            abi,
            code,
            (&*EVM_OPTS).initial_balance,
            None,
            None,
            libs,
        )
    }

    #[test]
    fn test_function_overriding() {
        let compiled = COMPILED.find("GreeterTest").expect("could not find contract");

        let (_, code, _) = compiled.into_parts_or_default();
        let mut libs = vec![];
        let mut runner = runner(compiled.abi.as_ref().unwrap(), code, &mut libs);

        let mut cfg = FuzzConfig::default();
        cfg.failure_persistence = None;
        let fuzzer = TestRunner::new(cfg);
        let results =
            runner.run_tests(&Filter::new("testGreeting", ".*"), Some(fuzzer), None).unwrap();
        assert!(results["testGreeting()"].success);
        assert!(results["testGreeting(string)"].success);
        assert!(results["testGreeting(string,string)"].success);
    }

    #[test]
    fn test_fuzzing_counterexamples() {
        let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
        let (_, code, _) = compiled.into_parts_or_default();
        let mut libs = vec![];
        let mut runner = runner(compiled.abi.as_ref().unwrap(), code, &mut libs);

        let mut cfg = FuzzConfig::default();
        cfg.failure_persistence = None;
        let fuzzer = TestRunner::new(cfg);
        let results =
            runner.run_tests(&Filter::new("testFuzz.*", ".*"), Some(fuzzer), None).unwrap();
        for (_, res) in results {
            assert!(!res.success);
            assert!(res.counterexample.is_some());
        }
    }

    #[test]
    fn test_fuzzing_ok() {
        let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
        let (_, code, _) = compiled.into_parts_or_default();
        let mut libs = vec![];
        let mut runner = runner(compiled.abi.as_ref().unwrap(), code, &mut libs);

        let mut cfg = FuzzConfig::default();
        cfg.failure_persistence = None;
        let fuzzer = TestRunner::new(cfg);
        let res =
            runner.run_tests(&Filter::new("testStringFuzz.*", ".*"), Some(fuzzer), None).unwrap();
        assert_eq!(res.len(), 1);
        assert!(res["testStringFuzz(string)"].success);
        assert!(res["testStringFuzz(string)"].counterexample.is_none());
    }

    #[test]
    fn test_fuzz_shrinking() {
        let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
        let (_, code, _) = compiled.into_parts_or_default();
        let mut libs = vec![];
        let mut runner = runner(compiled.abi.as_ref().unwrap(), code, &mut libs);

        let mut cfg = FuzzConfig::default();
        cfg.failure_persistence = None;
        let fuzzer = TestRunner::new(cfg);
        let res =
            runner.run_tests(&Filter::new("testShrinking.*", ".*"), Some(fuzzer), None).unwrap();
        assert_eq!(res.len(), 1);

        let res = res["testShrinking(uint256,uint256)"].clone();
        assert!(!res.success);

        // get the counterexample with shrinking enabled by default
        let counterexample = res.counterexample.unwrap();
        let product_with_shrinking: u64 =
                // casting to u64 here is safe because the shrunk result is always gonna be small
                // enough to fit in a u64, whereas as seen below, that's not possible without
                // shrinking
                counterexample.args.into_iter().map(|x| x.into_uint().unwrap().as_u64()).product();

        let mut cfg = FuzzConfig::default();
        cfg.failure_persistence = None;
        // we reduce the shrinking iters and observe a larger result
        cfg.max_shrink_iters = 5;
        let fuzzer = TestRunner::new(cfg);
        let res =
            runner.run_tests(&Filter::new("testShrinking.*", ".*"), Some(fuzzer), None).unwrap();
        assert_eq!(res.len(), 1);

        let res = res["testShrinking(uint256,uint256)"].clone();
        assert!(!res.success);

        // get the non-shrunk result
        let counterexample = res.counterexample.unwrap();
        let args =
            counterexample.args.into_iter().map(|x| x.into_uint().unwrap()).collect::<Vec<_>>();
        let product_without_shrinking = args[0].saturating_mul(args[1]);
        assert!(product_without_shrinking > product_with_shrinking.into());
    }
}
