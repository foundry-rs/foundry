use ethers::{
    abi::{Abi, Function, Token},
    types::{Address, Bytes},
};

use evm_adapters::{fuzz::FuzzedExecutor, Evm, EvmError};

use eyre::{Context, Result};
use regex::Regex;
use std::{collections::HashMap, fmt, time::Instant};

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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestResult {
    /// Whether the test case was successful. This means that the transaction executed
    /// properly, or that there was a revert and that the test was expected to fail
    /// (prefixed with `testFail`)
    pub success: bool,

    /// If there was a revert, this field will be populated. Note that the test can
    /// still be successful (i.e self.success == true) when it's expected to fail.
    pub reason: Option<String>,

    /// The gas used during execution
    pub gas_used: Option<u64>,

    /// Minimal reproduction test case for failing fuzz tests
    pub counterexample: Option<CounterExample>,

    /// Any captured & parsed as strings logs along the test's execution which should
    /// be printed to the user.
    pub logs: Vec<String>,
}

use std::marker::PhantomData;

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
    ) -> Result<HashMap<String, TestResult>> {
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

        let init_state = self.evm.state().clone();

        // run all unit tests
        let unit_tests = test_fns
            .iter()
            .filter(|func| func.inputs.is_empty())
            .map(|func| {
                // Before each test run executes, ensure we're at our initial state.
                self.evm.reset(init_state.clone());
                let result = self.run_test(func, needs_setup)?;
                Ok((func.signature(), result))
            })
            .collect::<Result<HashMap<_, _>>>()?;

        let map = if let Some(fuzzer) = fuzzer {
            let fuzz_tests = test_fns
                .iter()
                .filter(|func| !func.inputs.is_empty())
                .map(|func| {
                    let result = self.run_fuzz_test(func, needs_setup, fuzzer.clone())?;
                    Ok((func.signature(), result))
                })
                .collect::<Result<HashMap<_, _>>>()?;

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
    pub fn run_test(&mut self, func: &Function, setup: bool) -> Result<TestResult> {
        let start = Instant::now();
        // the expected result depends on the function name
        // DAppTools' ds-test will not revert inside its `assertEq`-like functions
        // which allows to test multiple assertions in 1 test function while also
        // preserving logs.
        let should_fail = func.name.starts_with("testFail");
        tracing::debug!(func = ?func.signature(), should_fail, "unit-testing");

        let mut logs = self.init_logs.to_vec();

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
                    (E::revert(), Some(reason), gas_used, logs)
                }
                err => {
                    tracing::error!(?err);
                    return Err(err.into())
                }
            },
        };
        let success = self.evm.check_success(self.address, &status, should_fail);
        let duration = Instant::now().duration_since(start);
        tracing::debug!(?duration, %success, %gas_used);

        Ok(TestResult { success, reason, gas_used: Some(gas_used), counterexample: None, logs })
    }

    #[tracing::instrument(name = "fuzz-test", skip_all, fields(name = %func.signature()))]
    pub fn run_fuzz_test(
        &mut self,
        func: &Function,
        setup: bool,
        runner: TestRunner,
    ) -> Result<TestResult> {
        let start = Instant::now();
        let should_fail = func.name.starts_with("testFail");
        tracing::debug!(func = ?func.signature(), should_fail, "fuzzing");

        // call the setup function in each test to reset the test's state.
        if setup {
            self.evm.setup(self.address)?;
        }

        // instantiate the fuzzed evm in line
        let evm = FuzzedExecutor::new(self.evm, runner, self.sender);
        let result = evm.fuzz(func, self.address, should_fail);

        let (success, counterexample) = match result {
            Ok(_) => (true, None),
            Err(TestError::Fail(_, value)) => {
                // skip the function selector when decoding
                let args = func.decode_input(&value.as_ref()[4..])?;
                let counterexample = CounterExample { calldata: value.clone(), args };
                tracing::info!("Found minimal failing case: {}", hex::encode(&value));
                (false, Some(counterexample))
            }
            result => panic!("Unexpected result: {:?}", result),
        };

        let duration = Instant::now().duration_since(start);
        tracing::debug!(?duration, %success);

        // TODO: How can we have proptest also return us the gas_used and the revert reason
        // from that call?
        Ok(TestResult { success, reason: None, gas_used: None, counterexample, logs: vec![] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::COMPILED;
    use ethers::solc::artifacts::CompactContractRef;
    use evm::Config;
    use evm_adapters::sputnik::PRECOMPILES_MAP;

    mod sputnik {
        use std::str::FromStr;

        use evm_adapters::sputnik::{
            helpers::{new_backend, new_vicinity},
            Executor,
        };
        use foundry_utils::get_func;
        use proptest::test_runner::Config as FuzzConfig;

        use super::*;

        #[test]
        fn test_runner() {
            let cfg = Config::istanbul();
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
            let vicinity = new_vicinity();
            let backend = new_backend(&vicinity, Default::default());
            let precompiles = PRECOMPILES_MAP.clone();
            let evm = Executor::new(12_000_000, &cfg, &backend, &precompiles);
            super::test_runner(evm, compiled);
        }

        #[test]
        fn test_function_overriding() {
            let cfg = Config::istanbul();
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
            let vicinity = new_vicinity();
            let backend = new_backend(&vicinity, Default::default());

            let precompiles = PRECOMPILES_MAP.clone();
            let mut evm = Executor::new(12_000_000, &cfg, &backend, &precompiles);
            let (addr, _, _, _) = evm
                .deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into())
                .unwrap();

            let mut runner =
                ContractRunner::new(&mut evm, compiled.abi.as_ref().unwrap(), addr, None, &[]);

            let mut cfg = FuzzConfig::default();
            cfg.failure_persistence = None;
            let mut fuzzer = TestRunner::new(cfg);
            let results = runner
                .run_tests(&Regex::from_str("testGreeting").unwrap(), Some(&mut fuzzer))
                .unwrap();
            assert!(results["testGreeting()"].success);
            assert!(results["testGreeting(string)"].success);
            assert!(results["testGreeting(string,string)"].success);
        }

        #[test]
        fn test_fuzzing_counterexamples() {
            let cfg = Config::istanbul();
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
            let vicinity = new_vicinity();
            let backend = new_backend(&vicinity, Default::default());

            let precompiles = PRECOMPILES_MAP.clone();
            let mut evm = Executor::new(12_000_000, &cfg, &backend, &precompiles);
            let (addr, _, _, _) = evm
                .deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into())
                .unwrap();

            let mut runner =
                ContractRunner::new(&mut evm, compiled.abi.as_ref().unwrap(), addr, None, &[]);

            let mut cfg = FuzzConfig::default();
            cfg.failure_persistence = None;
            let mut fuzzer = TestRunner::new(cfg);
            let results = runner
                .run_tests(&Regex::from_str("testFuzz.*").unwrap(), Some(&mut fuzzer))
                .unwrap();
            for (_, res) in results {
                assert!(!res.success);
                assert!(res.counterexample.is_some());
            }
        }

        #[test]
        fn test_fuzzing_ok() {
            let cfg = Config::istanbul();
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
            let vicinity = new_vicinity();
            let backend = new_backend(&vicinity, Default::default());

            let precompiles = PRECOMPILES_MAP.clone();
            let mut evm = Executor::new(u64::MAX, &cfg, &backend, &precompiles);
            let (addr, _, _, _) = evm
                .deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into())
                .unwrap();

            let mut runner =
                ContractRunner::new(&mut evm, compiled.abi.as_ref().unwrap(), addr, None, &[]);

            let mut cfg = FuzzConfig::default();
            cfg.failure_persistence = None;
            let fuzzer = TestRunner::new(cfg);
            let func = get_func("testStringFuzz(string)").unwrap();
            let res = runner.run_fuzz_test(&func, true, fuzzer.clone()).unwrap();
            assert!(res.success);
            assert!(res.counterexample.is_none());
        }

        #[test]
        fn test_fuzz_shrinking() {
            let cfg = Config::istanbul();
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
            let vicinity = new_vicinity();
            let backend = new_backend(&vicinity, Default::default());

            let precompiles = PRECOMPILES_MAP.clone();
            let mut evm = Executor::new(12_000_000, &cfg, &backend, &precompiles);
            let (addr, _, _, _) = evm
                .deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into())
                .unwrap();

            let mut runner =
                ContractRunner::new(&mut evm, compiled.abi.as_ref().unwrap(), addr, None, &[]);

            let mut cfg = FuzzConfig::default();
            cfg.failure_persistence = None;
            let fuzzer = TestRunner::new(cfg);
            let func = get_func("function testShrinking(uint256 x, uint256 y) public").unwrap();
            let res = runner.run_fuzz_test(&func, true, fuzzer.clone()).unwrap();
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

        let mut runner =
            ContractRunner::new(&mut evm, compiled.abi.as_ref().unwrap(), addr, None, &[]);

        let res = runner.run_tests(&".*".parse().unwrap(), None).unwrap();
        assert!(!res.is_empty());
        assert!(res.iter().all(|(_, result)| result.success));
    }
}
