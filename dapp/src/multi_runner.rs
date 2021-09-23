use crate::{
    artifacts::DapptoolsArtifact, executor, runner::TestResult, ContractRunner, Executor,
    MemoryState, SolcBuilder,
};
use regex::Regex;

use ethers::{
    types::Address,
    utils::{keccak256, CompiledContract},
};

use evm::{
    backend::{MemoryBackend, MemoryVicinity},
    Config,
};

use eyre::Result;
use std::{collections::HashMap, path::PathBuf};

pub struct MultiContractRunner<'a> {
    pub contracts: HashMap<String, CompiledContract>,
    pub addresses: HashMap<String, Address>,
    pub config: &'a Config,
    /// The blockchain environment (chain_id, gas_price, block gas limit etc.)
    // TODO: The DAPP_XXX env vars should allow instantiating this via the cli
    pub env: MemoryVicinity,
    /// The initial blockchain state. All test contracts get inserted here at
    /// initialization.
    pub init_state: MemoryState,
    pub state: MemoryState,
    pub gas_limit: u64,
}

impl<'a> MultiContractRunner<'a> {
    pub fn build(
        contracts: &str,
        remappings: Vec<String>,
        lib_paths: Vec<String>,
        out_path: PathBuf,
        no_compile: bool,
    ) -> Result<HashMap<String, CompiledContract>> {
        // TODO:
        // 1. incremental compilation
        // 2. parallel compilation
        // 3. Hardhat / Truffle-style artifacts
        Ok(if no_compile {
            let out_file = std::fs::read_to_string(out_path)?;
            serde_json::from_str::<DapptoolsArtifact>(&out_file)?.contracts()?
        } else {
            SolcBuilder::new(contracts, &remappings, &lib_paths)?.build_all()?
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        contracts: &str,
        remappings: Vec<String>,
        lib_paths: Vec<String>,
        out_path: PathBuf,
        config: &'a Config,
        gas_limit: u64,
        env: MemoryVicinity,
        no_compile: bool,
    ) -> Result<Self> {
        // 1. compile the contracts
        let contracts = Self::build(contracts, remappings, lib_paths, out_path, no_compile)?;

        // 2. create the initial state
        // TODO: Allow further overriding perhaps?
        let mut addresses = HashMap::new();
        let init_state = contracts
            .iter()
            .map(|(name, compiled)| {
                // make a fake address for the contract, maybe anti-pattern
                let addr = Address::from_slice(&keccak256(&compiled.runtime_bytecode)[..20]);
                addresses.insert(name.clone(), addr);
                (addr, compiled.runtime_bytecode.clone())
            })
            .collect::<Vec<_>>();
        let state = executor::initialize_contracts(init_state);

        Ok(Self { contracts, addresses, config, env, init_state: state.clone(), state, gas_limit })
    }

    /// instantiate an executor with the init state
    // TODO: Is this right? How would we cache results between calls when in
    // forking mode?
    fn backend(&self) -> MemoryBackend<'_> {
        MemoryBackend::new(&self.env, self.init_state.clone())
    }

    pub fn test(&self, pattern: Regex) -> Result<HashMap<String, HashMap<String, TestResult>>> {
        // NB: We also have access to the contract's abi. When running the test.
        // Can this be useful for decorating the stacktrace during a revert?
        // TODO: Check if the function starts with `prove` or `invariant`
        // Filter out for contracts that have at least 1 test function
        let tests = self
            .contracts
            .iter()
            .filter(|(_, contract)| contract.abi.functions().any(|x| x.name.starts_with("test")));

        let results = tests
            .into_iter()
            .map(|(name, contract)| {
                let address = *self
                    .addresses
                    .get(name)
                    .ok_or_else(|| eyre::eyre!("could not find contract address"))?;

                // TODO: Can we re-use the backend in a nice way, instead of re-instantiating
                // it each time?
                let backend = self.backend();
                let result = self.run_tests(name, contract, address, &backend, &pattern)?;
                Ok((name.clone(), result))
            })
            .filter_map(|x: Result<_>| x.ok())
            .filter_map(|(name, res)| if res.is_empty() { None } else { Some((name, res)) })
            .collect::<HashMap<_, _>>();

        Ok(results)
    }

    #[tracing::instrument(
        name = "contract",
        skip_all,
        fields(name = %_name)
    )]
    fn run_tests(
        &self,
        _name: &str,
        contract: &CompiledContract,
        address: Address,
        backend: &MemoryBackend<'_>,
        pattern: &Regex,
    ) -> Result<HashMap<String, TestResult>> {
        let mut dapp = Executor::new(self.gas_limit, self.config, backend);
        let mut runner = ContractRunner { executor: &mut dapp, contract, address };

        runner.run_tests(pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::new_vicinity;

    #[test]
    fn test_multi_runner() {
        let contracts = "./GreetTest.sol";
        let cfg = Config::istanbul();
        let gas_limit = 12_500_000;
        let env = new_vicinity();

        let runner = MultiContractRunner::new(
            contracts,
            vec![],
            vec![],
            PathBuf::new(),
            &cfg,
            gas_limit,
            env,
            false,
        )
        .unwrap();
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

    #[test]
    fn test_ds_test_fail() {
        let contracts = "./../FooTest.sol";
        let cfg = Config::istanbul();
        let gas_limit = 12_500_000;
        let env = new_vicinity();

        let runner = MultiContractRunner::new(
            contracts,
            vec![],
            vec![],
            PathBuf::new(),
            &cfg,
            gas_limit,
            env,
            false,
        )
        .unwrap();
        let results = runner.test(Regex::new("testFail").unwrap()).unwrap();
        let test = results.get("FooTest").unwrap().get("testFailX").unwrap();
        assert!(test.success);
    }
}
