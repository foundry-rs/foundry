use ethers::{
    abi::{Function, Token},
    prelude::Bytes,
    types::Address,
    utils::CompiledContract,
};

use evm_adapters::Evm;

use eyre::Result;
use regex::Regex;
use std::{collections::HashMap, time::Instant};

use proptest::test_runner::{TestError, TestRunner};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CounterExample {
    pub calldata: Bytes,
    // Token does not implement Serde (lol), so we just serialize the calldata
    #[serde(skip)]
    pub args: Vec<Token>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestResult {
    pub success: bool,

    pub gas_used: Option<u64>,

    /// Minimal reproduction test case for failing fuzz tests
    pub counterexample: Option<CounterExample>,
}

use std::marker::PhantomData;

pub struct ContractRunner<'a, S, E> {
    pub evm: &'a mut E,
    pub contract: &'a CompiledContract,
    pub address: Address,
    // need to constrain the trait generic
    state: PhantomData<S>,
}

impl<'a, S, E> ContractRunner<'a, S, E> {
    pub fn new(evm: &'a mut E, contract: &'a CompiledContract, address: Address) -> Self {
        Self { evm, contract, address, state: PhantomData }
    }
}

impl<'a, S, E: Evm<S>> ContractRunner<'a, S, E>
where
    E: Evm<S> + Clone,
{
    /// Runs all tests for a contract whose names match the provided regular expression
    pub fn run_tests(
        &mut self,
        regex: &Regex,
        fuzzer: Option<&mut TestRunner>,
    ) -> Result<HashMap<String, TestResult>> {
        let start = Instant::now();
        let needs_setup = self.contract.abi.functions().any(|func| func.name == "setUp");
        let test_fns = self
            .contract
            .abi
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
                let result = self.run_test(func, needs_setup)?;
                Ok((func.name.clone(), result))
            })
            .collect::<Result<HashMap<_, _>>>()?;

        let map = if let Some(mut fuzzer) = fuzzer {
            let fuzz_tests = test_fns
                .iter()
                .filter(|func| !func.inputs.is_empty())
                .map(|func| {
                    let result = self.run_fuzz_test(func, needs_setup, &mut fuzzer)?;
                    Ok((func.name.clone(), result))
                })
                .collect::<Result<HashMap<_, _>>>()?;

            let mut map = unit_tests;
            map.extend(fuzz_tests);
            map
        } else {
            unit_tests
        };

        if !map.is_empty() {
            let duration = Instant::now().duration_since(start);
            tracing::debug!("total duration: {:?}", duration);
        }
        Ok(map)
    }

    #[tracing::instrument(name = "test", skip_all, fields(name = %func.name))]
    pub fn run_test(&mut self, func: &Function, setup: bool) -> Result<TestResult> {
        let start = Instant::now();
        // the expected result depends on the function name
        // DAppTools' ds-test will not revert inside its `assertEq`-like functions
        // which allows to test multiple assertions in 1 test function while also
        // preserving logs.
        let should_fail = func.name.contains("testFail");

        // call the setup function in each test to reset the test's state.
        if setup {
            self.evm.setup(self.address)?;
        }

        let (_, reason, gas_used) =
            self.evm.call::<(), _>(Address::zero(), self.address, func, (), 0.into())?;
        let success = self.evm.check_success(self.address, &reason, should_fail);
        let duration = Instant::now().duration_since(start);
        tracing::trace!(?duration, %success, %gas_used);

        Ok(TestResult { success, gas_used: Some(gas_used), counterexample: None })
    }

    #[tracing::instrument(name = "fuzz-test", skip_all, fields(name = %func.name))]
    pub fn run_fuzz_test(
        &mut self,
        func: &Function,
        setup: bool,
        runner: &mut TestRunner,
    ) -> Result<TestResult> {
        // call the setup function in each test to reset the test's state.
        if setup {
            self.evm.setup(self.address)?;
        }

        let start = Instant::now();
        let should_fail = func.name.contains("testFail");

        // Get the calldata generation strategy for the function
        let strat = crate::fuzz::fuzz_calldata(func);

        // Run the strategy
        let result = runner.run(&strat, |calldata| {
            let mut evm = self.evm.clone();

            let (_, reason, _) = evm
                .call_raw(Address::zero(), self.address, calldata, 0.into(), false)
                .expect("could not make raw evm call");

            let success = evm.check_success(self.address, &reason, should_fail);

            // This will panic and get caught by the executor
            proptest::prop_assert!(success);

            Ok(())
        });

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
        tracing::trace!(?duration, %success);

        Ok(TestResult { success, gas_used: None, counterexample })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::COMPILED;
    use evm::Config;
    use std::marker::PhantomData;

    mod sputnik {
        use dapp_utils::get_func;
        use evm_adapters::sputnik::{
            helpers::{new_backend, new_vicinity},
            Executor,
        };
        use proptest::test_runner::Config as FuzzConfig;

        use super::*;

        #[test]
        fn test_runner() {
            let cfg = Config::istanbul();
            let compiled = COMPILED.get("GreeterTest").expect("could not find contract");
            let addr = "0x1000000000000000000000000000000000000000".parse().unwrap();
            let vicinity = new_vicinity();
            let backend = new_backend(&vicinity, Default::default());
            let evm = Executor::new(12_000_000, &cfg, &backend);
            super::test_runner(evm, addr, compiled);
        }

        #[test]
        fn test_fuzz_shrinking() {
            let cfg = Config::istanbul();
            let compiled = COMPILED.get("GreeterTest").expect("could not find contract");
            let addr = "0x1000000000000000000000000000000000000000".parse().unwrap();
            let vicinity = new_vicinity();
            let backend = new_backend(&vicinity, Default::default());

            let mut evm = Executor::new(12_000_000, &cfg, &backend);
            evm.initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

            let mut runner = ContractRunner {
                evm: &mut evm,
                contract: compiled,
                address: addr,
                state: PhantomData,
            };

            let cfg = FuzzConfig::default();
            let mut fuzzer = TestRunner::new(cfg);
            let func = get_func("function testFuzzShrinking(uint256 x, uint256 y) public").unwrap();
            let res = runner.run_fuzz_test(&func, true, &mut fuzzer).unwrap();
            assert!(!res.success);

            // get the counterexample with shrinking enabled by default
            let counterexample = res.counterexample.unwrap();
            let product_with_shrinking: u64 =
                // casting to u64 here is safe because the shrunk result is always gonna be small
                // enough to fit in a u64, whereas as seen below, that's not possible without
                // shrinking
                counterexample.args.into_iter().map(|x| x.into_uint().unwrap().as_u64()).product();

            let mut cfg = FuzzConfig::default();
            // we reduce the shrinking iters and observe a larger result
            cfg.max_shrink_iters = 5;
            let mut fuzzer = TestRunner::new(cfg);
            let res = runner.run_fuzz_test(&func, true, &mut fuzzer).unwrap();
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
            let compiled = COMPILED.get("GreeterTest").expect("could not find contract");

            let host = MockedHost::default();
            let addr: Address = "0x1000000000000000000000000000000000000000".parse().unwrap();

            let gas_limit = 12_000_000;
            let evm = EvmOdin::new(host, gas_limit, revision, NoopTracer);
            super::test_runner(evm, addr, compiled);
        }
    }

    pub fn test_runner<S, E: Clone + Evm<S>>(
        mut evm: E,
        addr: Address,
        compiled: &CompiledContract,
    ) {
        evm.initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let mut runner =
            ContractRunner { evm: &mut evm, contract: compiled, address: addr, state: PhantomData };

        let res = runner.run_tests(&".*".parse().unwrap(), None).unwrap();
        assert!(res.len() > 0);
        assert!(res.iter().all(|(_, result)| result.success == true));
    }
}
