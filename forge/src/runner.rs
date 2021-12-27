use ethers::{
    abi::{Abi, Function, Token},
    types::{Address, Bytes},
};
use evm_adapters::call_tracing::CallTraceArena;

use evm_adapters::{
    fuzz::{FuzzTestResult, FuzzedCases, FuzzedExecutor},
    Evm, EvmError,
};
use eyre::{Context, Result};
use regex::Regex;
use std::{collections::BTreeMap, fmt, marker::PhantomData, time::Instant};

use proptest::test_runner::{TestError, TestRunner};
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
        write!(f, "calldata=0x{}, args={:?}", hex::encode(&self.calldata), self.args)
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
    pub gas_used: u64,

    /// Minimal reproduction test case for failing fuzz tests
    pub counterexample: Option<CounterExample>,

    /// Any captured & parsed as strings logs along the test's execution which should
    /// be printed to the user.
    pub logs: Vec<String>,

    /// What kind of test this was
    pub kind: TestKind,

    /// Traces
    pub traces: Option<Vec<CallTraceArena>>,

    /// Identified contracts
    pub identified_contracts: Option<BTreeMap<Address, (String, Abi)>>,
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
            // we use the median for comparisons
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

pub struct ContractRunner<'a, S, E> {
    /// Mutable reference to the EVM type.
    /// This is a temporary hack to work around the mutability restrictions of
    /// [`proptest::TestRunner::run`] which takes a `Fn` preventing interior mutability. [See also](https://github.com/gakonst/dapptools-rs/pull/44).
    /// Wrapping it like that allows the `test` function to gain mutable access regardless and
    /// since we don't use any parallelized fuzzing yet the `test` function has exclusive access of
    /// the mutable reference over time of its existence.
    pub evm: &'a mut E,
    /// The deployed contract's ABI
    pub contract: &'a Abi,
    /// The deployed contract's address
    pub address: Address,
    /// The address which will be used as the `from` field in all EVM calls
    pub sender: Address,
    /// Any logs emitted in the constructor of the specific contract
    pub init_logs: &'a [String],
    // need to constrain the trait generic
    state: PhantomData<S>,
}

impl<'a, S, E> ContractRunner<'a, S, E> {
    pub fn new(
        evm: &'a mut E,
        contract: &'a Abi,
        address: Address,
        sender: Option<Address>,
        init_logs: &'a [String],
    ) -> Self {
        Self {
            evm,
            contract,
            address,
            init_logs,
            state: PhantomData,
            sender: sender.unwrap_or_default(),
        }
    }
}

impl<'a, S: Clone, E: Evm<S>> ContractRunner<'a, S, E> {
    /// Runs all tests for a contract whose names match the provided regular expression
    pub fn run_tests(
        &mut self,
        regex: &Regex,
        fuzzer: Option<&mut TestRunner>,
        init_state: &S,
        known_contracts: Option<&BTreeMap<String, (Abi, Vec<u8>)>>,
    ) -> Result<BTreeMap<String, TestResult>> {
        tracing::info!("starting tests");
        let start = Instant::now();
        let needs_setup = self.contract.functions().any(|func| func.name == "setUp");
        let test_fns = self
            .contract
            .functions()
            .into_iter()
            .filter(|func| func.name.starts_with("test"))
            .filter(|func| regex.is_match(&func.name))
            .collect::<Vec<_>>();

        // run all unit tests
        let unit_tests = test_fns
            .iter()
            .filter(|func| func.inputs.is_empty())
            .map(|func| {
                // Before each test run executes, ensure we're at our initial state.
                self.evm.reset(init_state.clone());
                let result = self.run_test(func, needs_setup, known_contracts)?;
                Ok((func.signature(), result))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;

        let map = if let Some(fuzzer) = fuzzer {
            let fuzz_tests = test_fns
                .iter()
                .filter(|func| !func.inputs.is_empty())
                .map(|func| {
                    let result = self.run_fuzz_test(func, needs_setup, fuzzer.clone())?;
                    Ok((func.signature(), result))
                })
                .collect::<Result<BTreeMap<_, _>>>()?;

            let mut map = unit_tests;
            map.extend(fuzz_tests);
            map
        } else {
            unit_tests
        };

        if !map.is_empty() {
            let successful = map.iter().filter(|(_, tst)| tst.success).count();
            let duration = Instant::now().duration_since(start);
            tracing::info!(?duration, "done. {}/{} successful", successful, map.len());
        }
        Ok(map)
    }

    #[tracing::instrument(name = "test", skip_all, fields(name = %func.signature()))]
    pub fn run_test(
        &mut self,
        func: &Function,
        setup: bool,
        known_contracts: Option<&BTreeMap<String, (Abi, Vec<u8>)>>,
    ) -> Result<TestResult> {
        let start = Instant::now();
        // the expected result depends on the function name
        // DAppTools' ds-test will not revert inside its `assertEq`-like functions
        // which allows to test multiple assertions in 1 test function while also
        // preserving logs.
        let should_fail = func.name.starts_with("testFail");
        tracing::debug!(func = ?func.signature(), should_fail, "unit-testing");

        let mut logs = self.init_logs.to_vec();

        self.evm.reset_traces();

        // call the setup function in each test to reset the test's state.
        if setup {
            tracing::trace!("setting up");
            let setup_logs = self
                .evm
                .setup(self.address)
                .wrap_err(format!("could not setup during {} test", func.signature()))?
                .1;
            logs.extend_from_slice(&setup_logs);
        }

        let (status, reason, gas_used, logs) = match self.evm.call::<(), _, _>(
            self.sender,
            self.address,
            func.clone(),
            (),
            0.into(),
        ) {
            Ok((_, status, gas_used, execution_logs)) => {
                logs.extend(execution_logs);
                (status, None, gas_used, logs)
            }
            Err(err) => match err {
                EvmError::Execution { reason, gas_used, logs: execution_logs } => {
                    logs.extend(execution_logs);
                    // add reverted logs
                    logs.extend(self.evm.all_logs());
                    (E::revert(), Some(reason), gas_used, logs)
                }
                err => {
                    tracing::error!(?err);
                    return Err(err.into())
                }
            },
        };

        let mut traces: Option<Vec<CallTraceArena>> = None;
        let mut identified_contracts: Option<BTreeMap<Address, (String, Abi)>> = None;

        let evm_traces = self.evm.traces();
        if !evm_traces.is_empty() && self.evm.tracing_enabled() {
            let mut ident = BTreeMap::new();
            // create an iter over the traces
            let mut trace_iter = evm_traces.into_iter();
            let mut temp_traces = Vec::new();
            if setup {
                // grab the setup trace if it exists
                let setup = trace_iter.next().expect("no setup trace");
                setup.update_identified(
                    0,
                    known_contracts.expect("traces enabled but no identified_contracts"),
                    &mut ident,
                    self.evm,
                );
                temp_traces.push(setup);
            }
            // grab the test trace
            let test_trace = trace_iter.next().expect("no test trace");
            test_trace.update_identified(
                0,
                known_contracts.expect("traces enabled but no identified_contracts"),
                &mut ident,
                self.evm,
            );
            temp_traces.push(test_trace);

            // pass back the identified contracts and traces
            identified_contracts = Some(ident);
            traces = Some(temp_traces);
        }

        self.evm.reset_traces();

        let success = self.evm.check_success(self.address, &status, should_fail);
        let duration = Instant::now().duration_since(start);
        tracing::debug!(?duration, %success, %gas_used);

        Ok(TestResult {
            success,
            reason,
            gas_used,
            counterexample: None,
            logs,
            kind: TestKind::Standard(gas_used),
            traces,
            identified_contracts,
        })
    }

    #[tracing::instrument(name = "fuzz-test", skip_all, fields(name = %func.signature()))]
    pub fn run_fuzz_test(
        &mut self,
        func: &Function,
        setup: bool,
        runner: TestRunner,
    ) -> Result<TestResult> {
        // do not trace in fuzztests, as it's a big performance hit
        let prev = self.evm.set_tracing_enabled(false);
        let start = Instant::now();
        let should_fail = func.name.starts_with("testFail");
        tracing::debug!(func = ?func.signature(), should_fail, "fuzzing");

        // call the setup function in each test to reset the test's state.
        if setup {
            self.evm.setup(self.address)?;
        }

        // instantiate the fuzzed evm in line
        let evm = FuzzedExecutor::new(self.evm, runner, self.sender);
        let FuzzTestResult { cases, test_error } = evm.fuzz(func, self.address, should_fail);

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
            reason = Some(err.revert_reason);
        }

        let duration = Instant::now().duration_since(start);
        tracing::debug!(?duration, %success);

        // reset tracing to previous value in case next test *isn't* a fuzz test
        self.evm.set_tracing_enabled(prev);
        // from that call?
        Ok(TestResult {
            success,
            reason,
            gas_used: cases.median_gas(),
            counterexample,
            logs: vec![],
            kind: TestKind::Fuzz(cases),
            traces: None,
            identified_contracts: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::COMPILED;
    use ethers::solc::artifacts::CompactContractRef;
    use evm_adapters::sputnik::helpers::vm;

    mod sputnik {
        use std::str::FromStr;

        use foundry_utils::get_func;
        use proptest::test_runner::Config as FuzzConfig;

        use super::*;

        #[test]
        fn test_runner() {
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
            let evm = vm();
            super::test_runner(evm, compiled);
        }

        #[test]
        fn test_function_overriding() {
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
            let mut evm = vm();
            let (addr, _, _, _) = evm
                .deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into())
                .unwrap();

            let init_state = evm.state().clone();

            let mut runner =
                ContractRunner::new(&mut evm, compiled.abi.as_ref().unwrap(), addr, None, &[]);

            let mut cfg = FuzzConfig::default();
            cfg.failure_persistence = None;
            let mut fuzzer = TestRunner::new(cfg);
            let results = runner
                .run_tests(
                    &Regex::from_str("testGreeting").unwrap(),
                    Some(&mut fuzzer),
                    &init_state,
                    None,
                )
                .unwrap();
            assert!(results["testGreeting()"].success);
            assert!(results["testGreeting(string)"].success);
            assert!(results["testGreeting(string,string)"].success);
        }

        #[test]
        fn test_fuzzing_counterexamples() {
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
            let mut evm = vm();
            let (addr, _, _, _) = evm
                .deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into())
                .unwrap();

            let init_state = evm.state().clone();

            let mut runner =
                ContractRunner::new(&mut evm, compiled.abi.as_ref().unwrap(), addr, None, &[]);

            let mut cfg = FuzzConfig::default();
            cfg.failure_persistence = None;
            let mut fuzzer = TestRunner::new(cfg);
            let results = runner
                .run_tests(
                    &Regex::from_str("testFuzz.*").unwrap(),
                    Some(&mut fuzzer),
                    &init_state,
                    None,
                )
                .unwrap();
            for (_, res) in results {
                assert!(!res.success);
                assert!(res.counterexample.is_some());
            }
        }

        #[test]
        fn test_fuzzing_ok() {
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
            let mut evm = vm();
            let (addr, _, _, _) = evm
                .deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into())
                .unwrap();

            let mut runner =
                ContractRunner::new(&mut evm, compiled.abi.as_ref().unwrap(), addr, None, &[]);

            let mut cfg = FuzzConfig::default();
            cfg.failure_persistence = None;
            let fuzzer = TestRunner::new(cfg);
            let func = get_func("testStringFuzz(string)").unwrap();
            let res = runner.run_fuzz_test(&func, true, fuzzer).unwrap();
            assert!(res.success);
            assert!(res.counterexample.is_none());
        }

        #[test]
        fn test_fuzz_shrinking() {
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
            let mut evm = vm();
            let (addr, _, _, _) = evm
                .deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into())
                .unwrap();

            let mut runner =
                ContractRunner::new(&mut evm, compiled.abi.as_ref().unwrap(), addr, None, &[]);

            let mut cfg = FuzzConfig::default();
            cfg.failure_persistence = None;
            let fuzzer = TestRunner::new(cfg);
            let func = get_func("function testShrinking(uint256 x, uint256 y) public").unwrap();
            let res = runner.run_fuzz_test(&func, true, fuzzer).unwrap();
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
            let res = runner.run_fuzz_test(&func, true, fuzzer).unwrap();
            assert!(!res.success);

            // get the non-shrunk result
            let counterexample = res.counterexample.unwrap();
            let args =
                counterexample.args.into_iter().map(|x| x.into_uint().unwrap()).collect::<Vec<_>>();
            let product_without_shrinking = args[0].saturating_mul(args[1]);
            assert!(product_without_shrinking > product_with_shrinking.into());
        }
    }

    mod evmodin {
        use super::*;
        use ::evmodin::{tracing::NoopTracer, util::mocked_host::MockedHost, Revision};
        use evm_adapters::evmodin::EvmOdin;

        #[test]
        #[ignore]
        fn test_runner() {
            let revision = Revision::Istanbul;
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");

            let host = MockedHost::default();

            let gas_limit = 12_000_000;
            let evm = EvmOdin::new(host, gas_limit, revision, NoopTracer);
            super::test_runner(evm, compiled);
        }
    }

    pub fn test_runner<S: Clone, E: Evm<S>>(mut evm: E, compiled: CompactContractRef) {
        let (addr, _, _, _) =
            evm.deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into()).unwrap();

        let init_state = evm.state().clone();

        let mut runner =
            ContractRunner::new(&mut evm, compiled.abi.as_ref().unwrap(), addr, None, &[]);

        let res = runner.run_tests(&".*".parse().unwrap(), None, &init_state, None).unwrap();
        assert!(!res.is_empty());
        assert!(res.iter().all(|(_, result)| result.success));
    }
}
