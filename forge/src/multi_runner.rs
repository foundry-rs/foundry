use crate::{runner::TestResult, ContractRunner};
use evm_adapters::Evm;

use ethers::{
    abi::Abi,
    prelude::ArtifactOutput,
    solc::{Artifact, Project},
    types::{Address, U256},
};

use proptest::test_runner::TestRunner;
use regex::Regex;

use eyre::{Context, Result};
use std::{collections::BTreeMap, marker::PhantomData};

/// Builder used for instantiating the multi-contract runner
#[derive(Debug, Default)]
pub struct MultiContractRunnerBuilder {
    /// The fuzzer to be used for running fuzz tests
    pub fuzzer: Option<TestRunner>,
    /// The address which will be used to deploy the initial contracts and send all
    /// transactions
    pub sender: Option<Address>,
    /// The initial balance for each one of the deployed smart contracts
    pub initial_balance: U256,
}

impl MultiContractRunnerBuilder {
    /// Given an EVM, proceeds to return a runner which is able to execute all tests
    /// against that evm
    pub fn build<A, E, S>(
        self,
        project: Project<A>,
        mut evm: E,
    ) -> Result<MultiContractRunner<E, S>>
    where
        // TODO: Can we remove the static? It's due to the `into_artifacts()` call below
        A: ArtifactOutput + 'static,
        E: Evm<S>,
    {
        let output = project.compile()?;
        if output.has_compiler_errors() {
            // return the diagnostics error back to the user.
            eyre::bail!(output.to_string())
        } else if output.is_unchanged() {
            println!("no files changed, compilation skippped.");
        } else {
            println!("success.");
        }

        let sender = self.sender.unwrap_or_default();
        let initial_balance = self.initial_balance;

        // This is just the contracts compiled, but we need to merge this with the read cached
        // artifacts
        let contracts = output.into_artifacts();
        let contracts: BTreeMap<String, (Abi, Address, Vec<String>)> = contracts
            // only take contracts with valid abi and bytecode
            .filter_map(|(fname, contract)| {
                let (abi, bytecode) = contract.into_inner();
                abi.and_then(|abi| bytecode.map(|bytecode| (fname, abi, bytecode)))
            })
            // Only take contracts with empty constructors.
            .filter(|(_, abi, _)| {
                abi.constructor.as_ref().map(|c| c.inputs.is_empty()).unwrap_or(true)
            })
            // Only take contracts which contain a `test` function
            .filter(|(_, abi, _)| abi.functions().any(|func| func.name.starts_with("test")))
            // deploy the contracts
            .map(|(name, abi, bytecode)| {
                let span = tracing::trace_span!("deploying", ?name);
                let _enter = span.enter();

                let (addr, _, _, logs) = evm
                    .deploy(sender, bytecode, 0.into())
                    .wrap_err(format!("could not deploy {}", name))?;

                evm.set_balance(addr, initial_balance);
                Ok((name, (abi, addr, logs)))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;

        Ok(MultiContractRunner {
            contracts,
            evm,
            state: PhantomData,
            sender: self.sender,
            fuzzer: self.fuzzer,
        })
    }

    pub fn sender(mut self, sender: Address) -> Self {
        self.sender = Some(sender);
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
}

/// A multi contract runner receives a set of contracts deployed in an EVM instance and proceeds
/// to run all test functions in these contracts.
pub struct MultiContractRunner<E, S> {
    /// Mapping of contract name to compiled bytecode, deployed address and logs emitted during
    /// deployment
    contracts: BTreeMap<String, (Abi, Address, Vec<String>)>,
    /// The EVM instance used in the test runner
    evm: E,
    /// The fuzzer which will be used to run parametric tests (w/ non-0 solidity args)
    fuzzer: Option<TestRunner>,
    /// The address which will be used as the `from` field in all EVM calls
    sender: Option<Address>,
    /// Market type for the EVM state being used
    state: PhantomData<S>,
}

impl<E, S> MultiContractRunner<E, S>
where
    E: Evm<S>,
    S: Clone,
{
    pub fn test(
        &mut self,
        pattern: Regex,
    ) -> Result<BTreeMap<String, BTreeMap<String, TestResult>>> {
        // TODO: Convert to iterator, ideally parallel one?
        let contracts = std::mem::take(&mut self.contracts);
        let results = contracts
            .iter()
            .map(|(name, (abi, address, logs))| {
                let result = self.run_tests(name, abi, *address, logs, &pattern)?;
                Ok((name.clone(), result))
            })
            .filter_map(|x: Result<_>| x.ok())
            .filter_map(|(name, res)| if res.is_empty() { None } else { Some((name, res)) })
            .collect::<BTreeMap<_, _>>();

        self.contracts = contracts;

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
        contract: &Abi,
        address: Address,
        init_logs: &[String],
        pattern: &Regex,
    ) -> Result<BTreeMap<String, TestResult>> {
        let mut runner =
            ContractRunner::new(&mut self.evm, contract, address, self.sender, init_logs);
        runner.run_tests(pattern, self.fuzzer.as_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::solc::ProjectPathsConfig;
    use std::path::PathBuf;

    fn project() -> Project {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata");

        let paths = ProjectPathsConfig::builder().root(&root).sources(&root).build().unwrap();

        let project = Project::builder()
            // need to add the ilb path here. would it be better placed in the ProjectPathsConfig
            // instead? what is the `libs` modifier useful for then? linked libraries?
            .allowed_path(root.join("../../evm-adapters/testdata"))
            .paths(paths)
            .ephemeral()
            .no_artifacts()
            .build()
            .unwrap();

        project
    }

    fn runner<S: Clone, E: Evm<S>>(evm: E) -> MultiContractRunner<E, S> {
        MultiContractRunnerBuilder::default().build(project(), evm).unwrap()
    }

    fn test_multi_runner<S: Clone, E: Evm<S>>(evm: E) {
        let mut runner = runner(evm);
        let results = runner.test(Regex::new(".*").unwrap()).unwrap();

        // 6 contracts being built
        assert_eq!(results.keys().len(), 5);
        for (_, contract_tests) in results {
            assert_ne!(contract_tests.keys().len(), 0);
            assert!(contract_tests.iter().all(|(_, result)| result.success));
        }

        // can also filter
        let only_gm = runner.test(Regex::new("testGm.*").unwrap()).unwrap();
        assert_eq!(only_gm.len(), 1);

        assert_eq!(only_gm["GmTest"].len(), 1);
        assert!(only_gm["GmTest"]["testGm()"].success);
    }

    mod sputnik {
        use super::*;
        use evm::Config;
        use evm_adapters::sputnik::{
            helpers::{new_backend, new_vicinity},
            Executor, PRECOMPILES_MAP,
        };
        use std::collections::HashMap;

        #[test]
        fn test_sputnik_debug_logs() {
            let config = Config::istanbul();
            let gas_limit = 12_500_000;
            let env = new_vicinity();
            let backend = new_backend(&env, Default::default());
            // important to instantiate the VM with cheatcodes
            let precompiles = PRECOMPILES_MAP.clone();
            let evm =
                Executor::new_with_cheatcodes(backend, gas_limit, &config, &precompiles, false);

            let mut runner = runner(evm);
            let results = runner.test(Regex::new(".*").unwrap()).unwrap();

            let reasons = results["DebugLogsTest"]
                .iter()
                .map(|(name, res)| (name, res.logs.clone()))
                .collect::<HashMap<_, _>>();
            dbg!(&reasons);
            assert_eq!(
                reasons[&"test1()".to_owned()],
                vec!["constructor".to_owned(), "setUp".to_owned(), "one".to_owned()]
            );
            assert_eq!(
                reasons[&"test2()".to_owned()],
                vec!["constructor".to_owned(), "setUp".to_owned(), "two".to_owned()]
            );
        }

        #[test]
        fn test_sputnik_multi_runner() {
            let config = Config::istanbul();
            let gas_limit = 12_500_000;
            let env = new_vicinity();
            let backend = new_backend(&env, Default::default());
            let precompiles = PRECOMPILES_MAP.clone();
            let evm = Executor::new(gas_limit, &config, &backend, &precompiles);
            test_multi_runner(evm);
        }
    }

    // TODO: Add EvmOdin tests once we get the Mocked Host working
}
