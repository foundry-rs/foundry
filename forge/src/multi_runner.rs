use crate::{runner::TestResult, ContractRunner, TestFilter};

use ethers::solc::Artifact;

use evm_adapters::Evm;

use ethers::{
    abi::Abi,
    prelude::ArtifactOutput,
    solc::Project,
    types::{Address, U256},
};

use proptest::test_runner::TestRunner;

use eyre::Result;
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
        println!("compiling...");
        let output = project.compile()?;
        if output.has_compiler_errors() {
            // return the diagnostics error back to the user.
            eyre::bail!(output.to_string())
        } else if output.is_unchanged() {
            println!("no files changed, compilation skipped.");
        } else {
            println!("success.");
        }

        let sender = self.sender.unwrap_or_default();
        let initial_balance = self.initial_balance;

        // This is just the contracts compiled, but we need to merge this with the read cached
        // artifacts
        let contracts = output.into_artifacts();
        let mut known_contracts: BTreeMap<String, (Abi, Vec<u8>)> = Default::default();
        let mut deployed_contracts: BTreeMap<String, (Abi, Address, Vec<String>)> =
            Default::default();

        for (fname, contract) in contracts {
            let (maybe_abi, maybe_deploy_bytes, maybe_runtime_bytes) = contract.into_parts();
            if let (Some(abi), Some(bytecode)) = (maybe_abi, maybe_deploy_bytes) {
                // skip deployment of abstract contracts
                if bytecode.as_ref().is_empty() {
                    continue
                }

                if abi.constructor.as_ref().map(|c| c.inputs.is_empty()).unwrap_or(true) &&
                    abi.functions().any(|func| func.name.starts_with("test"))
                {
                    let span = tracing::trace_span!("deploying", ?fname);
                    let _enter = span.enter();
                    let (addr, _, _, logs) = evm.deploy(sender, bytecode.clone(), 0u32.into())?;
                    evm.set_balance(addr, initial_balance);
                    deployed_contracts.insert(fname.clone(), (abi.clone(), addr, logs));
                }

                let split = fname.split(':').collect::<Vec<&str>>();
                let contract_name = if split.len() > 1 { split[1] } else { split[0] };
                if let Some(runtime_code) = maybe_runtime_bytes {
                    known_contracts.insert(contract_name.to_string(), (abi, runtime_code.to_vec()));
                }
            }
        }

        Ok(MultiContractRunner {
            contracts: deployed_contracts,
            known_contracts,
            identified_contracts: Default::default(),
            evm,
            state: PhantomData,
            sender: self.sender,
            fuzzer: self.fuzzer,
        })
    }

    #[must_use]
    pub fn sender(mut self, sender: Address) -> Self {
        self.sender = Some(sender);
        self
    }

    #[must_use]
    pub fn initial_balance(mut self, initial_balance: U256) -> Self {
        self.initial_balance = initial_balance;
        self
    }

    #[must_use]
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
    pub contracts: BTreeMap<String, (Abi, Address, Vec<String>)>,
    /// Compiled contracts by name that have an Abi and runtime bytecode
    pub known_contracts: BTreeMap<String, (Abi, Vec<u8>)>,
    /// Identified contracts by test
    pub identified_contracts: BTreeMap<String, BTreeMap<Address, (String, Abi)>>,
    /// The EVM instance used in the test runner
    pub evm: E,
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
        filter: &impl TestFilter,
    ) -> Result<BTreeMap<String, BTreeMap<String, TestResult>>> {
        // TODO: Convert to iterator, ideally parallel one?
        let contracts = std::mem::take(&mut self.contracts);

        let init_state: S = self.evm.state().clone();
        let results = contracts
            .iter()
            .filter(|(name, _)| filter.matches_contract(name))
            .map(|(name, (abi, address, logs))| {
                let result = self.run_tests(name, abi, *address, logs, filter, &init_state)?;
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
        filter: &impl TestFilter,
        init_state: &S,
    ) -> Result<BTreeMap<String, TestResult>> {
        let mut runner =
            ContractRunner::new(&mut self.evm, contract, address, self.sender, init_logs);
        runner.run_tests(filter, self.fuzzer.as_mut(), init_state, Some(&self.known_contracts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::Filter;
    use ethers::solc::ProjectPathsConfig;
    use std::path::PathBuf;

    fn project() -> Project {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata");

        let paths = ProjectPathsConfig::builder().root(&root).sources(&root).build().unwrap();

        Project::builder()
            // need to add the ilb path here. would it be better placed in the ProjectPathsConfig
            // instead? what is the `libs` modifier useful for then? linked libraries?
            .allowed_path(root.join("../../evm-adapters/testdata"))
            .paths(paths)
            .ephemeral()
            .no_artifacts()
            .build()
            .unwrap()
    }

    fn runner<S: Clone, E: Evm<S>>(evm: E) -> MultiContractRunner<E, S> {
        MultiContractRunnerBuilder::default().build(project(), evm).unwrap()
    }

    fn test_multi_runner<S: Clone, E: Evm<S>>(evm: E) {
        let mut runner = runner(evm);
        let results = runner.test(&Filter::new(".*", ".*")).unwrap();

        // 6 contracts being built
        assert_eq!(results.keys().len(), 7);
        for (_, contract_tests) in results {
            assert_ne!(contract_tests.keys().len(), 0);
            assert!(contract_tests.iter().all(|(_, result)| result.success));
        }

        // can also filter
        let only_gm = runner.test(&Filter::new("testGm.*", ".*")).unwrap();
        assert_eq!(only_gm.len(), 1);

        assert_eq!(only_gm["GmTest.json:GmTest"].len(), 1);
        assert!(only_gm["GmTest.json:GmTest"]["testGm()"].success);
    }

    fn test_abstract_contract<S: Clone, E: Evm<S>>(evm: E) {
        let mut runner = runner(evm);
        let results = runner.test(&Filter::new(".*", ".*")).unwrap();
        assert!(results.get("Tests.json:Tests").is_none());
        assert!(results.get("ATests.json:ATests").is_some());
        assert!(results.get("BTests.json:BTests").is_some());
    }

    mod sputnik {
        use super::*;
        use evm_adapters::sputnik::helpers::vm;
        use std::collections::HashMap;

        #[test]
        fn test_sputnik_debug_logs() {
            let evm = vm();

            let mut runner = runner(evm);
            let results = runner.test(&Filter::new(".*", ".*")).unwrap();

            let reasons = results["DebugLogsTest.json:DebugLogsTest"]
                .iter()
                .map(|(name, res)| (name, res.logs.clone()))
                .collect::<HashMap<_, _>>();
            assert_eq!(
                reasons[&"test1()".to_owned()],
                vec!["constructor".to_owned(), "setUp".to_owned(), "one".to_owned()]
            );
            assert_eq!(
                reasons[&"test2()".to_owned()],
                vec!["constructor".to_owned(), "setUp".to_owned(), "two".to_owned()]
            );
            assert_eq!(
                reasons[&"testFailWithRevert()".to_owned()],
                vec![
                    "constructor".to_owned(),
                    "setUp".to_owned(),
                    "three".to_owned(),
                    "failure".to_owned()
                ]
            );
            assert_eq!(
                reasons[&"testFailWithRequire()".to_owned()],
                vec!["constructor".to_owned(), "setUp".to_owned(), "four".to_owned()]
            );
        }

        #[test]
        fn test_sputnik_multi_runner() {
            test_multi_runner(vm());
        }

        #[test]
        fn test_sputnik_abstract_contract() {
            test_abstract_contract(vm());
        }
    }

    // TODO: Add EvmOdin tests once we get the Mocked Host working
}
