use crate::{
    executor::{opts::EvmOpts, Executor, ExecutorBuilder, SpecId},
    runner::TestResult,
    ContractRunner, TestFilter,
};
use foundry_utils::PostLinkInput;
use revm::db::DatabaseRef;

use ethers::{
    abi::{Abi, Event, Function},
    prelude::{artifacts::CompactContractBytecode, ArtifactOutput},
    solc::{Artifact, Project},
    types::{Address, Bytes, H256, U256},
};

use proptest::test_runner::TestRunner;

use eyre::Result;
use rayon::prelude::*;
use std::collections::BTreeMap;

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
    /// The EVM spec to use
    pub evm_spec: Option<SpecId>,
}

pub type DeployableContracts = BTreeMap<String, (Abi, Bytes, Vec<Bytes>)>;

impl MultiContractRunnerBuilder {
    /// Given an EVM, proceeds to return a runner which is able to execute all tests
    /// against that evm
    pub fn build<A>(self, project: Project<A>, evm_opts: EvmOpts) -> Result<MultiContractRunner>
    where
        A: ArtifactOutput,
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

        // This is just the contracts compiled, but we need to merge this with the read cached
        // artifacts
        let contracts = output
            .into_artifacts()
            .map(|(n, c)| (n, c.into_contract_bytecode()))
            .collect::<BTreeMap<String, CompactContractBytecode>>();
        let mut known_contracts: BTreeMap<String, (Abi, Vec<u8>)> = Default::default();

        // create a mapping of name => (abi, deployment code, Vec<library deployment code>)
        let mut deployable_contracts = DeployableContracts::default();

        foundry_utils::link(
            &contracts,
            &mut known_contracts,
            evm_opts.sender,
            &mut deployable_contracts,
            |file, key| (format!("{}.json:{}", key, key), file, key),
            |post_link_input| {
                let PostLinkInput {
                    contract,
                    known_contracts,
                    fname,
                    extra: deployable_contracts,
                    dependencies,
                } = post_link_input;

                // get bytes
                let bytecode =
                    if let Some(b) = contract.bytecode.expect("No bytecode").object.into_bytes() {
                        b
                    } else {
                        return Ok(())
                    };

                let abi = contract.abi.expect("We should have an abi by now");
                // if its a test, add it to deployable contracts
                if abi.constructor.as_ref().map(|c| c.inputs.is_empty()).unwrap_or(true) &&
                    abi.functions().any(|func| func.name.starts_with("test"))
                {
                    deployable_contracts
                        .insert(fname.clone(), (abi.clone(), bytecode, dependencies.to_vec()));
                }

                let split = fname.split(':').collect::<Vec<&str>>();
                let contract_name = if split.len() > 1 { split[1] } else { split[0] };
                contract
                    .deployed_bytecode
                    .and_then(|d_bcode| d_bcode.bytecode)
                    .and_then(|bcode| bcode.object.into_bytes())
                    .and_then(|bytes| {
                        known_contracts.insert(contract_name.to_string(), (abi, bytes.to_vec()))
                    });
                Ok(())
            },
        )?;

        // TODO Add forge specific contracts
        //known_contracts.insert("VM".to_string(), (HEVM_ABI.clone(), Vec::new()));
        //known_contracts.insert("VM_CONSOLE".to_string(), (CONSOLE_ABI.clone(), Vec::new()));

        let execution_info = foundry_utils::flatten_known_contracts(&known_contracts);
        Ok(MultiContractRunner {
            contracts: deployable_contracts,
            known_contracts,
            identified_contracts: Default::default(),
            evm_opts,
            evm_spec: self.evm_spec.unwrap_or(SpecId::LONDON),
            sender: self.sender,
            fuzzer: self.fuzzer,
            execution_info,
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

    #[must_use]
    pub fn evm_spec(mut self, spec: SpecId) -> Self {
        self.evm_spec = Some(spec);
        self
    }
}

/// A multi contract runner receives a set of contracts deployed in an EVM instance and proceeds
/// to run all test functions in these contracts.
pub struct MultiContractRunner {
    /// Mapping of contract name to Abi, creation bytecode and library bytecode which
    /// needs to be deployed & linked against
    pub contracts: BTreeMap<String, (Abi, ethers::prelude::Bytes, Vec<ethers::prelude::Bytes>)>,
    /// Compiled contracts by name that have an Abi and runtime bytecode
    pub known_contracts: BTreeMap<String, (Abi, Vec<u8>)>,
    /// Identified contracts by test
    pub identified_contracts: BTreeMap<String, BTreeMap<Address, (String, Abi)>>,
    /// The EVM instance used in the test runner
    pub evm_opts: EvmOpts,
    /// The EVM spec
    pub evm_spec: SpecId,
    /// All contract execution info, (functions, events, errors)
    pub execution_info: (BTreeMap<[u8; 4], Function>, BTreeMap<H256, Event>, Abi),
    /// The fuzzer which will be used to run parametric tests (w/ non-0 solidity args)
    fuzzer: Option<TestRunner>,
    /// The address which will be used as the `from` field in all EVM calls
    sender: Option<Address>,
}

impl MultiContractRunner {
    pub fn test(
        &mut self,
        filter: &(impl TestFilter + Send + Sync),
    ) -> Result<BTreeMap<String, BTreeMap<String, TestResult>>> {
        let results = self
            .contracts
            .par_iter()
            .filter(|(name, _)| filter.matches_contract(name))
            .filter(|(_, (abi, _, _))| abi.functions().any(|func| filter.matches_test(&func.name)))
            .map(|(name, (abi, deploy_code, libs))| {
                // TODO: Fork mode and "vicinity"
                let executor = ExecutorBuilder::new()
                    .with_cheatcodes(self.evm_opts.ffi)
                    .with_config(self.evm_opts.env.evm_env())
                    .with_spec(self.evm_spec)
                    .build();
                let result =
                    self.run_tests(name, abi, executor, deploy_code.clone(), libs, filter)?;
                Ok((name.clone(), result))
            })
            .filter_map(|x: Result<_>| x.ok())
            .filter_map(|(name, res)| if res.is_empty() { None } else { Some((name, res)) })
            .collect::<BTreeMap<_, _>>();

        Ok(results)
    }

    // The _name field is unused because we only want it for tracing
    #[tracing::instrument(
        name = "contract",
        skip_all,
        err,
        fields(name = %_name)
    )]
    fn run_tests<DB: DatabaseRef + Clone + Send + Sync>(
        &self,
        _name: &str,
        contract: &Abi,
        executor: Executor<DB>,
        deploy_code: Bytes,
        libs: &[Bytes],
        filter: &impl TestFilter,
    ) -> Result<BTreeMap<String, TestResult>> {
        let mut runner = ContractRunner::new(
            executor,
            contract,
            deploy_code,
            self.evm_opts.initial_balance,
            self.sender,
            Some((&self.execution_info.0, &self.execution_info.1, &self.execution_info.2)),
            libs,
        );
        runner.run_tests(filter, self.fuzzer.clone(), Some(&self.known_contracts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{Filter, EVM_OPTS};
    use ethers::solc::ProjectPathsConfig;
    use std::collections::HashMap;

    fn project() -> Project {
        let paths =
            ProjectPathsConfig::builder().root("testdata").sources("testdata").build().unwrap();

        Project::builder().paths(paths).ephemeral().no_artifacts().build().unwrap()
    }

    fn runner() -> MultiContractRunner {
        MultiContractRunnerBuilder::default().build(project(), EVM_OPTS.clone()).unwrap()
    }

    #[test]
    fn test_multi_runner() {
        let mut runner = runner();
        let results = runner.test(&Filter::new(".*", ".*")).unwrap();

        // 9 contracts being built
        assert_eq!(results.keys().len(), 11);
        for (key, contract_tests) in results {
            match key.as_str() {
                // Tests that should revert
                "SetupTest.json:SetupTest" | "FuzzTests.json:FuzzTests" => {
                    assert!(contract_tests.iter().all(|(_, result)| !result.success))
                }
                // The rest should pass
                _ => {
                    assert_ne!(contract_tests.keys().len(), 0);
                    assert!(contract_tests.iter().all(|(_, result)| { result.success }))
                }
            }
        }

        // can also filter
        let only_gm = runner.test(&Filter::new("testGm.*", ".*")).unwrap();
        assert_eq!(only_gm.len(), 1);

        assert_eq!(only_gm["GmTest.json:GmTest"].len(), 1);
        assert!(only_gm["GmTest.json:GmTest"]["testGm()"].success);
    }

    #[test]
    fn test_abstract_contract() {
        let mut runner = runner();
        let results = runner.test(&Filter::new(".*", ".*")).unwrap();
        assert!(results.get("Tests.json:Tests").is_none());
        assert!(results.get("ATests.json:ATests").is_some());
        assert!(results.get("BTests.json:BTests").is_some());
    }

    // TODO: This test is currently ignored since we have no means of getting logs from reverted
    // tests (case 3 and 4). We should re-enable when we have that capability.
    #[test]
    #[ignore]
    fn test_debug_logs() {
        let mut runner = runner();
        let results = runner.test(&Filter::new(".*", ".*")).unwrap();

        let reasons = results["DebugLogsTest.json:DebugLogsTest"]
            .iter()
            .map(|(name, res)| (name, res.logs.clone()))
            .collect::<HashMap<_, _>>();
        assert_eq!(
            reasons[&"test1()".to_owned()],
            //vec!["constructor".to_owned(), "setUp".to_owned(), "one".to_owned()]
            vec![]
        );
        assert_eq!(
            reasons[&"test2()".to_owned()],
            //vec!["constructor".to_owned(), "setUp".to_owned(), "two".to_owned()]
            vec![]
        );
        assert_eq!(
            reasons[&"testFailWithRevert()".to_owned()],
            //vec![
            //    "constructor".to_owned(),
            //    "setUp".to_owned(),
            //    "three".to_owned(),
            //    "failure".to_owned()
            //]
            vec![]
        );
        assert_eq!(
            reasons[&"testFailWithRequire()".to_owned()],
            //vec!["constructor".to_owned(), "setUp".to_owned(), "four".to_owned()]
            vec![]
        );
    }
}
