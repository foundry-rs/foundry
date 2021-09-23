use ethers::{
    abi::{Function, FunctionExt},
    types::Address,
    utils::CompiledContract,
};

use evm::{
    backend::MemoryBackend, executor::MemoryStackState, ExitReason, ExitRevert, ExitSucceed,
    Handler,
};

use dapp_utils::get_func;

use crate::executor::Executor;

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

pub struct ContractRunner<'a, S> {
    pub executor: &'a mut Executor<'a, S>,
    pub contract: &'a CompiledContract,
    pub address: Address,
}

impl<'a> ContractRunner<'a, MemoryStackState<'a, 'a, MemoryBackend<'a>>> {
    /// Runs the `setUp()` function call to initiate the contract's state
    fn setup(&mut self) -> Result<()> {
        let (_, status, _) = self.executor.call::<(), _>(
            Address::zero(),
            self.address,
            &get_func("function setUp() external").unwrap(),
            (),
            0.into(),
        )?;
        debug_assert_eq!(status, ExitReason::Succeed(ExitSucceed::Stopped));
        Ok(())
    }

    /// Runs the `failed()` function call to inspect the test contract's state
    fn failed(&mut self) -> Result<bool> {
        let (failed, _, _) = self.executor.call::<bool, _>(
            Address::zero(),
            self.address,
            &get_func("function failed() returns (bool)").unwrap(),
            (),
            0.into(),
        )?;
        Ok(failed)
    }

    /// runs all tests under a contract
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
                    self.setup()?;
                }

                let result = self.run_test(func);
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
    pub fn run_test(&mut self, func: &Function) -> TestResult {
        let start = Instant::now();

        // set the selector & execute the call
        let calldata = func.selector();
        let gas_before = self.executor.executor.gas_left();
        let (result, _) = self.executor.executor.transact_call(
            Address::zero(),
            self.address,
            0.into(),
            calldata.to_vec(),
            self.executor.gas_limit,
            vec![],
        );
        let gas_after = self.executor.executor.gas_left();
        // We subtract the calldata & base gas cost from our test's
        // gas consumption
        let gas_used = crate::remove_extra_costs(gas_before - gas_after, &calldata).as_u64();

        let duration = Instant::now().duration_since(start);

        // the expected result depends on the function name
        // DAppTools' ds-test will not revert inside its `assertEq`-like functions
        // which allows to test multiple assertions in 1 test function while also
        // preserving logs.
        let success = if func.name.contains("testFail") {
            match result {
                // If the function call failed, we're good.
                ExitReason::Revert(inner) => inner == ExitRevert::Reverted,
                // If the function call was successful in an expected fail case,
                // we make a call to the `failed()` function inherited from DS-Test
                ExitReason::Succeed(ExitSucceed::Stopped) => self.failed().unwrap_or(false),
                err => {
                    tracing::error!(?err);
                    false
                }
            }
        } else {
            result == ExitReason::Succeed(ExitSucceed::Stopped)
        };
        tracing::trace!(?duration, %success, %gas_used);

        TestResult { success, gas_used }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        executor::initialize_contracts,
        test_helpers::{new_backend, new_vicinity, COMPILED},
    };
    use evm::Config;

    #[test]
    fn test_runner() {
        let cfg = Config::istanbul();

        let compiled = COMPILED.get("GreeterTest").expect("could not find contract");

        let addr = "0x1000000000000000000000000000000000000000".parse().unwrap();
        let state = initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let vicinity = new_vicinity();
        let backend = new_backend(&vicinity, state);
        let mut dapp = Executor::new(12_000_000, &cfg, &backend);

        let mut runner = ContractRunner { executor: &mut dapp, contract: compiled, address: addr };

        let res = runner.run_tests(&".*".parse().unwrap()).unwrap();
        assert!(res.iter().all(|(_, result)| result.success == true));
    }
}
