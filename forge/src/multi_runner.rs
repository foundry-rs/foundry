use crate::{result::SuiteResult, ContractRunner, TestFilter, TestOptions};
use ethers::{
    abi::Abi,
    prelude::{artifacts::CompactContractBytecode, ArtifactId, ArtifactOutput},
    solc::{contracts::ArtifactContracts, Artifact, ProjectCompileOutput},
    types::{Address, Bytes, U256},
};
use eyre::Result;
use foundry_common::{ContractsByArtifact, TestFunctionExt};
use foundry_evm::{
    executor::{
        backend::Backend, fork::CreateFork, inspector::CheatsConfig, opts::EvmOpts, Executor,
        ExecutorBuilder,
    },
    revm,
};
use foundry_utils::PostLinkInput;
use rayon::prelude::*;
use revm::primitives::SpecId;
use std::{collections::BTreeMap, path::Path, sync::mpsc::Sender};

pub type DeployableContracts = BTreeMap<ArtifactId, (Abi, Bytes, Vec<Bytes>)>;

/// A multi contract runner receives a set of contracts deployed in an EVM instance and proceeds
/// to run all test functions in these contracts.
pub struct MultiContractRunner {
    /// Mapping of contract name to Abi, creation bytecode and library bytecode which
    /// needs to be deployed & linked against
    pub contracts: DeployableContracts,
    /// Compiled contracts by name that have an Abi and runtime bytecode
    pub known_contracts: ContractsByArtifact,
    /// The EVM instance used in the test runner
    pub evm_opts: EvmOpts,
    /// The configured evm
    pub env: revm::primitives::Env,
    /// The EVM spec
    pub evm_spec: SpecId,
    /// All known errors, used for decoding reverts
    pub errors: Option<Abi>,
    /// The address which will be used as the `from` field in all EVM calls
    pub sender: Option<Address>,
    /// A map of contract names to absolute source file paths
    pub source_paths: BTreeMap<String, String>,
    /// The fork to use at launch
    pub fork: Option<CreateFork>,
    /// Additional cheatcode inspector related settings derived from the `Config`
    pub cheats_config: CheatsConfig,
    /// Whether to collect coverage info
    pub coverage: bool,
    /// Settings related to fuzz and/or invariant tests
    pub test_options: TestOptions,
}

impl MultiContractRunner {
    /// Returns the number of matching tests
    pub fn count_filtered_tests(&self, filter: &impl TestFilter) -> usize {
        self.contracts
            .iter()
            .filter(|(id, _)| {
                filter.matches_path(id.source.to_string_lossy()) &&
                    filter.matches_contract(&id.name)
            })
            .flat_map(|(_, (abi, _, _))| {
                abi.functions().filter(|func| filter.matches_test(func.signature()))
            })
            .count()
    }

    // Get all tests of matching path and contract
    pub fn get_tests(&self, filter: &impl TestFilter) -> Vec<String> {
        self.contracts
            .iter()
            .filter(|(id, _)| {
                filter.matches_path(id.source.to_string_lossy()) &&
                    filter.matches_contract(&id.name)
            })
            .flat_map(|(_, (abi, _, _))| abi.functions().map(|func| func.name.clone()))
            .filter(|sig| sig.is_test())
            .collect()
    }

    /// Returns all matching tests grouped by contract grouped by file (file -> (contract -> tests))
    pub fn list(
        &self,
        filter: &impl TestFilter,
    ) -> BTreeMap<String, BTreeMap<String, Vec<String>>> {
        self.contracts
            .iter()
            .filter(|(id, _)| {
                filter.matches_path(id.source.to_string_lossy()) &&
                    filter.matches_contract(&id.name)
            })
            .filter(|(_, (abi, _, _))| abi.functions().any(|func| filter.matches_test(&func.name)))
            .map(|(id, (abi, _, _))| {
                let source = id.source.as_path().display().to_string();
                let name = id.name.clone();
                let tests = abi
                    .functions()
                    .filter(|func| func.name.is_test())
                    .filter(|func| filter.matches_test(func.signature()))
                    .map(|func| func.name.clone())
                    .collect::<Vec<_>>();

                (source, name, tests)
            })
            .fold(BTreeMap::new(), |mut acc, (source, name, tests)| {
                acc.entry(source).or_default().insert(name, tests);
                acc
            })
    }

    /// Executes _all_ tests that match the given `filter`
    ///
    /// This will create the runtime based on the configured `evm` ops and create the `Backend`
    /// before executing all contracts and their tests in _parallel_.
    ///
    /// Each Executor gets its own instance of the `Backend`.
    pub fn test(
        &mut self,
        filter: &impl TestFilter,
        stream_result: Option<Sender<(String, SuiteResult)>>,
        test_options: TestOptions,
    ) -> Result<BTreeMap<String, SuiteResult>> {
        tracing::trace!("start all tests");

        // the db backend that serves all the data, each contract gets its own instance
        let db = Backend::spawn(self.fork.take());

        let results = self
            .contracts
            .par_iter()
            .filter(|(id, _)| {
                filter.matches_path(id.source.to_string_lossy()) &&
                    filter.matches_contract(&id.name)
            })
            .filter(|(_, (abi, _, _))| abi.functions().any(|func| filter.matches_test(&func.name)))
            .map(|(id, (abi, deploy_code, libs))| {
                let executor = ExecutorBuilder::default()
                    .with_cheatcodes(self.cheats_config.clone())
                    .with_config(self.env.clone())
                    .with_spec(self.evm_spec)
                    .with_gas_limit(self.evm_opts.gas_limit())
                    .set_tracing(self.evm_opts.verbosity >= 3)
                    .set_coverage(self.coverage)
                    .build(db.clone());
                let identifier = id.identifier();
                tracing::trace!(contract= ?identifier, "start executing all tests in contract");

                let result = self.run_tests(
                    &identifier,
                    abi,
                    executor,
                    deploy_code.clone(),
                    libs,
                    (filter, test_options),
                )?;

                tracing::trace!(contract= ?identifier, "executed all tests in contract");
                Ok((identifier, result))
            })
            .filter_map(Result::<_>::ok)
            .filter(|(_, results)| !results.is_empty())
            .map_with(stream_result, |stream_result, (name, result)| {
                if let Some(stream_result) = stream_result.as_ref() {
                    let _ = stream_result.send((name.clone(), result.clone()));
                }
                (name, result)
            })
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
    fn run_tests(
        &self,
        _name: &str,
        contract: &Abi,
        executor: Executor,
        deploy_code: Bytes,
        libs: &[Bytes],
        (filter, test_options): (&impl TestFilter, TestOptions),
    ) -> Result<SuiteResult> {
        let runner = ContractRunner::new(
            executor,
            contract,
            deploy_code,
            self.evm_opts.initial_balance,
            self.sender,
            self.errors.as_ref(),
            libs,
        );
        runner.run_tests(filter, test_options, Some(&self.known_contracts))
    }
}

/// Builder used for instantiating the multi-contract runner
#[derive(Debug, Default)]
pub struct MultiContractRunnerBuilder {
    /// The address which will be used to deploy the initial contracts and send all
    /// transactions
    pub sender: Option<Address>,
    /// The initial balance for each one of the deployed smart contracts
    pub initial_balance: U256,
    /// The EVM spec to use
    pub evm_spec: Option<SpecId>,
    /// The fork to use at launch
    pub fork: Option<CreateFork>,
    /// Additional cheatcode inspector related settings derived from the `Config`
    pub cheats_config: Option<CheatsConfig>,
    /// Whether or not to collect coverage info
    pub coverage: bool,
    /// Settings related to fuzz and/or invariant tests
    pub test_options: Option<TestOptions>,
}

impl MultiContractRunnerBuilder {
    /// Given an EVM, proceeds to return a runner which is able to execute all tests
    /// against that evm
    pub fn build<A>(
        self,
        root: impl AsRef<Path>,
        output: ProjectCompileOutput<A>,
        env: revm::primitives::Env,
        evm_opts: EvmOpts,
    ) -> Result<MultiContractRunner>
    where
        A: ArtifactOutput,
    {
        // This is just the contracts compiled, but we need to merge this with the read cached
        // artifacts
        let contracts = output
            .with_stripped_file_prefixes(&root)
            .into_artifacts()
            .map(|(i, c)| (i, c.into_contract_bytecode()))
            .collect::<Vec<(ArtifactId, CompactContractBytecode)>>();

        let mut known_contracts = ContractsByArtifact::default();
        let source_paths = contracts
            .iter()
            .map(|(i, _)| (i.identifier(), root.as_ref().join(&i.source).to_string_lossy().into()))
            .collect::<BTreeMap<String, String>>();

        // create a mapping of name => (abi, deployment code, Vec<library deployment code>)
        let mut deployable_contracts = DeployableContracts::default();

        foundry_utils::link_with_nonce_or_address(
            ArtifactContracts::from_iter(contracts),
            &mut known_contracts,
            Default::default(),
            evm_opts.sender,
            U256::one(),
            &mut deployable_contracts,
            |file, key| (format!("{key}.json:{key}"), file, key),
            |post_link_input| {
                let PostLinkInput {
                    contract,
                    known_contracts,
                    id,
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
                // if it's a test, add it to deployable contracts
                if abi.constructor.as_ref().map(|c| c.inputs.is_empty()).unwrap_or(true) &&
                    abi.functions()
                        .any(|func| func.name.is_test() || func.name.is_invariant_test())
                {
                    deployable_contracts.insert(
                        id.clone(),
                        (
                            abi.clone(),
                            bytecode,
                            dependencies
                                .into_iter()
                                .map(|(_, bytecode)| bytecode)
                                .collect::<Vec<_>>(),
                        ),
                    );
                }

                contract
                    .deployed_bytecode
                    .and_then(|d_bcode| d_bcode.bytecode)
                    .and_then(|bcode| bcode.object.into_bytes())
                    .and_then(|bytes| known_contracts.insert(id.clone(), (abi, bytes.to_vec())));
                Ok(())
            },
        )?;

        let execution_info = known_contracts.flatten();
        Ok(MultiContractRunner {
            contracts: deployable_contracts,
            known_contracts,
            evm_opts,
            env,
            evm_spec: self.evm_spec.unwrap_or(SpecId::MERGE),
            sender: self.sender,
            errors: Some(execution_info.2),
            source_paths,
            fork: self.fork,
            cheats_config: self.cheats_config.unwrap_or_default(),
            coverage: self.coverage,
            test_options: self.test_options.unwrap_or_default(),
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
    pub fn evm_spec(mut self, spec: SpecId) -> Self {
        self.evm_spec = Some(spec);
        self
    }

    #[must_use]
    pub fn with_fork(mut self, fork: Option<CreateFork>) -> Self {
        self.fork = fork;
        self
    }

    #[must_use]
    pub fn with_cheats_config(mut self, cheats_config: CheatsConfig) -> Self {
        self.cheats_config = Some(cheats_config);
        self
    }

    #[must_use]
    pub fn with_test_options(mut self, test_options: TestOptions) -> Self {
        self.test_options = Some(test_options);
        self
    }

    #[must_use]
    pub fn set_coverage(mut self, enable: bool) -> Self {
        self.coverage = enable;
        self
    }
}
