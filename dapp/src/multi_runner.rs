use crate::{artifacts::DapptoolsArtifact, runner::TestResult, ContractRunner};
use dapp_solc::SolcBuilder;
use evm_adapters::Evm;

use ethers::{
    types::{Address, U256},
    utils::CompiledContract,
};

use proptest::test_runner::TestRunner;
use regex::Regex;

use eyre::{Context, Result};
use std::{collections::HashMap, marker::PhantomData, path::PathBuf};

/// Builder used for instantiating the multi-contract runner
#[derive(Clone, Debug, Default)]
pub struct MultiContractRunnerBuilder<'a> {
    /// Glob to the contracts we want compiled
    pub contracts: &'a str,
    /// Solc remappings
    pub remappings: &'a [String],
    /// Solc lib import paths
    pub libraries: &'a [String],
    /// The path for the output file
    pub out_path: PathBuf,
    pub no_compile: bool,
    /// The fuzzer to be used for running fuzz tests
    pub fuzzer: Option<TestRunner>,
    /// The address which will be used to deploy the initial contracts
    pub deployer: Address,
    /// The initial balance for each one of the deployed smart contracts
    pub initial_balance: U256,
}

impl<'a> MultiContractRunnerBuilder<'a> {
    /// Given an EVM, proceeds to return a runner which is able to execute all tests
    /// against that evm
    pub fn build<E, S>(self, mut evm: E) -> Result<MultiContractRunner<E, S>>
    where
        E: Evm<S>,
    {
        // 1. incremental compilation
        // 2. parallel compilation
        // 3. Hardhat / Truffle-style artifacts
        let contracts = if self.no_compile {
            DapptoolsArtifact::read(self.out_path)?.into_contracts()?
        } else {
            SolcBuilder::new(self.contracts, self.remappings, self.libraries)?.build_all()?
        };

        let deployer = self.deployer;
        let initial_balance = self.initial_balance;
        let addresses = contracts
            .iter()
            .filter(|(_, compiled)| {
                compiled.abi.constructor.as_ref().map(|c| c.inputs.is_empty()).unwrap_or(true)
            })
            .filter(|(_, compiled)| {
                compiled.abi.functions().any(|func| func.name.starts_with("test"))
            })
            .map(|(name, compiled)| {
                let span = tracing::trace_span!("deploying", ?name);
                let _enter = span.enter();

                let (addr, _, _, logs) = evm
                    .deploy(deployer, compiled.bytecode.clone(), 0.into())
                    .wrap_err(format!("could not deploy {}", name))?;

                evm.set_balance(addr, initial_balance);
                Ok((name.clone(), (addr, logs)))
            })
            .collect::<Result<HashMap<_, _>>>()?;

        Ok(MultiContractRunner {
            contracts,
            addresses,
            evm,
            state: PhantomData,
            fuzzer: self.fuzzer,
        })
    }

    pub fn contracts(mut self, contracts: &'a str) -> Self {
        self.contracts = contracts;
        self
    }

    pub fn deployer(mut self, deployer: Address) -> Self {
        self.deployer = deployer;
        self
    }

    pub fn initial_balance(mut self, initial_balance: U256) -> Self {
        self.initial_balance = initial_balance;
        self
    }

    pub fn fuzzer(mut self, fuzzer: TestRunner) -> Self {
        self.fuzzer = Some(fuzzer);
        self
    }

    pub fn remappings(mut self, remappings: &'a [String]) -> Self {
        self.remappings = remappings;
        self
    }

    pub fn libraries(mut self, libraries: &'a [String]) -> Self {
        self.libraries = libraries;
        self
    }

    pub fn out_path(mut self, out_path: PathBuf) -> Self {
        self.out_path = out_path;
        self
    }

    pub fn skip_compilation(mut self, flag: bool) -> Self {
        self.no_compile = flag;
        self
    }
}

pub struct MultiContractRunner<E, S> {
    /// Mapping of contract name to compiled bytecode
    contracts: HashMap<String, CompiledContract>,
    /// Mapping of contract name to the address it's been injected in the EVM state
    addresses: HashMap<String, (Address, Vec<String>)>,
    /// The EVM instance used in the test runner
    evm: E,
    fuzzer: Option<TestRunner>,
    state: PhantomData<S>,
}

impl<E, S> MultiContractRunner<E, S>
where
    E: Evm<S>,
    S: Clone,
{
    pub fn test(&mut self, pattern: Regex) -> Result<HashMap<String, HashMap<String, TestResult>>> {
        // NB: We also have access to the contract's abi. When running the test.
        // Can this be useful for decorating the stacktrace during a revert?
        // TODO: Check if the function starts with `prove` or `invariant`
        // Filter out for contracts that have at least 1 test function
        let contracts = std::mem::take(&mut self.contracts);
        let tests = contracts
            .iter()
            .filter(|(_, contract)| contract.abi.functions().any(|x| x.name.starts_with("test")));

        // TODO: Is this pattern OK? We use the memory and then write it back to avoid any
        // borrow checker issues. Otherwise, we'd need to clone large vectors.
        let addresses = std::mem::take(&mut self.addresses);
        let results = tests
            .into_iter()
            .map(|(name, contract)| {
                let (address, init_logs) = addresses
                    .get(name)
                    .ok_or_else(|| eyre::eyre!("could not find contract address"))?;

                let result = self.run_tests(name, contract, *address, init_logs, &pattern)?;
                Ok((name.clone(), result))
            })
            .filter_map(|x: Result<_>| x.ok())
            .filter_map(|(name, res)| if res.is_empty() { None } else { Some((name, res)) })
            .collect::<HashMap<_, _>>();

        self.contracts = contracts;
        self.addresses = addresses;

        Ok(results)
    }

    // The _name field is unused because we only want it for tracing
    #[tracing::instrument(
        name = "contract",
        skip_all,
        err,
        fields(name = %_name)
    )]
    fn run_tests(
        &mut self,
        _name: &str,
        contract: &CompiledContract,
        address: Address,
        init_logs: &[String],
        pattern: &Regex,
    ) -> Result<HashMap<String, TestResult>> {
        let mut runner = ContractRunner::new(&mut self.evm, contract, address, init_logs);
        runner.run_tests(pattern, self.fuzzer.as_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_multi_runner<S: Clone, E: Evm<S>>(evm: E) {
        let mut runner =
            MultiContractRunnerBuilder::default().contracts("./GreetTest.sol").build(evm).unwrap();

        let results = runner.test(Regex::new(".*").unwrap()).unwrap();

        // 2 contracts
        assert_eq!(results.len(), 2);

        // 3 tests on greeter 1 on gm
        assert_eq!(results["GreeterTest"].len(), 3);
        assert_eq!(results["GmTest"].len(), 1);
        for (_, res) in results {
            assert!(res.iter().all(|(_, result)| result.success));
        }

        let only_gm = runner.test(Regex::new("testGm.*").unwrap()).unwrap();
        assert_eq!(only_gm.len(), 1);
        assert_eq!(only_gm["GmTest"].len(), 1);
    }

    fn test_ds_test_fail<S: Clone, E: Evm<S>>(evm: E) {
        let mut runner =
            MultiContractRunnerBuilder::default().contracts("./../FooTest.sol").build(evm).unwrap();
        let results = runner.test(Regex::new(".*").unwrap()).unwrap();
        let test = results.get("FooTest").unwrap().get("testFailX").unwrap();
        assert!(test.success);
    }

    mod sputnik {
        use super::*;
        use evm::Config;
        use evm_adapters::sputnik::{
            helpers::{new_backend, new_vicinity},
            Executor,
        };

        #[test]
        fn test_sputnik_multi_runner() {
            let config = Config::istanbul();
            let gas_limit = 12_500_000;
            let env = new_vicinity();
            let backend = new_backend(&env, Default::default());
            let evm = Executor::new(gas_limit, &config, &backend);
            test_multi_runner(evm);
        }

        #[test]
        fn test_sputnik_ds_test_fail() {
            let config = Config::istanbul();
            let gas_limit = 12_500_000;
            let env = new_vicinity();
            let backend = new_backend(&env, Default::default());
            let evm = Executor::new(gas_limit, &config, &backend);
            test_ds_test_fail(evm);
        }
    }

    // TODO: Add EvmOdin tests once we get the Mocked Host working
}
