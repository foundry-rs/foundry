use ethers::{abi::Function, types::Address, utils::CompiledContract};

use evm_adapters::Evm;

use eyre::Result;
use regex::Regex;
use std::{collections::HashMap, time::Instant};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestResult {
    pub success: bool,
    // TODO: Ensure that this is calculated properly
    pub gas_used: u64,
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
    E: Evm<S>,
{
    /// Runs all tests for a contract whose names match the provided regular expression
    pub fn run_tests(&mut self, regex: &Regex) -> Result<HashMap<String, TestResult>> {
        let start = Instant::now();
        let needs_setup = self.contract.abi.functions().any(|func| func.name == "setUp");
        let test_fns = self
            .contract
            .abi
            .functions()
            .into_iter()
            .filter(|func| func.name.starts_with("test"))
            .filter(|func| regex.is_match(&func.name));

        // run all tests
        let map = test_fns
            .map(|func| {
                // call the setup function in each test to reset the test's state.
                // if we did this outside the map, we'd not have test isolation
                if needs_setup {
                    self.evm.setup(self.address)?;
                }

                let result = self.run_test(func)?;
                Ok((func.name.clone(), result))
            })
            .collect::<Result<HashMap<_, _>>>()?;

        if !map.is_empty() {
            let duration = Instant::now().duration_since(start);
            tracing::debug!("total duration: {:?}", duration);
        }
        Ok(map)
    }

    #[tracing::instrument(name = "test", skip_all, fields(name = %func.name))]
    pub fn run_test(&mut self, func: &Function) -> Result<TestResult> {
        let start = Instant::now();

        // The test result data is not used anywhere.
        let (_, reason, gas_used) =
            self.evm.call::<(), _>(Address::zero(), self.address, func, (), 0.into())?;
        let duration = Instant::now().duration_since(start);

        // the expected result depends on the function name
        // DAppTools' ds-test will not revert inside its `assertEq`-like functions
        // which allows to test multiple assertions in 1 test function while also
        // preserving logs.
        let success = self.evm.check_success(self.address, &reason, func.name.contains("testFail"));
        tracing::trace!(?duration, %success, %gas_used);

        Ok(TestResult { success, gas_used })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::COMPILED;
    use evm::Config;
    use std::marker::PhantomData;

    mod sputnik {
        use evm_adapters::sputnik::{
            helpers::{new_backend, new_vicinity},
            Executor,
        };

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

    pub fn test_runner<S, E: Evm<S>>(mut evm: E, addr: Address, compiled: &CompiledContract) {
        evm.initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let mut runner =
            ContractRunner { evm: &mut evm, contract: compiled, address: addr, state: PhantomData };

        let res = runner.run_tests(&".*".parse().unwrap()).unwrap();
        assert!(res.len() > 0);
        assert!(res.iter().all(|(_, result)| result.success == true));
    }
}
