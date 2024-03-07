//! Forge test runner for multiple contracts.

use crate::{result::SuiteResult, ContractRunner, TestFilter, TestOptions};
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{Address, Bytes, U256};
use eyre::Result;
use foundry_common::{get_contract_name, ContractsByArtifact, TestFunctionExt};
use foundry_compilers::{contracts::ArtifactContracts, Artifact, ArtifactId, ProjectCompileOutput};
use foundry_evm::{
    backend::Backend,
    decode::RevertDecoder,
    executors::{Executor, ExecutorBuilder},
    fork::CreateFork,
    inspectors::CheatsConfig,
    opts::EvmOpts,
    revm,
};
use foundry_linking::{LinkOutput, Linker};
use rayon::prelude::*;
use revm::primitives::SpecId;
use std::{
    collections::BTreeMap,
    fmt::Debug,
    path::Path,
    sync::{mpsc, Arc},
    time::Instant,
};

pub type DeployableContracts = BTreeMap<ArtifactId, (JsonAbi, Bytes, Vec<Bytes>)>;

/// A multi contract runner receives a set of contracts deployed in an EVM instance and proceeds
/// to run all test functions in these contracts.
pub struct MultiContractRunner {
    /// Mapping of contract name to JsonAbi, creation bytecode and library bytecode which
    /// needs to be deployed & linked against
    pub contracts: DeployableContracts,
    /// Compiled contracts by name that have an JsonAbi and runtime bytecode
    pub known_contracts: ContractsByArtifact,
    /// The EVM instance used in the test runner
    pub evm_opts: EvmOpts,
    /// The configured evm
    pub env: revm::primitives::Env,
    /// The EVM spec
    pub evm_spec: SpecId,
    /// Revert decoder. Contains all known errors and their selectors.
    pub revert_decoder: RevertDecoder,
    /// The address which will be used as the `from` field in all EVM calls
    pub sender: Option<Address>,
    /// A map of contract names to absolute source file paths
    pub source_paths: BTreeMap<String, String>,
    /// The fork to use at launch
    pub fork: Option<CreateFork>,
    /// Additional cheatcode inspector related settings derived from the `Config`
    pub cheats_config: Arc<CheatsConfig>,
    /// Whether to collect coverage info
    pub coverage: bool,
    /// Whether to collect debug info
    pub debug: bool,
    /// Settings related to fuzz and/or invariant tests
    pub test_options: TestOptions,
    /// Whether to enable call isolation
    pub isolation: bool,
}

impl MultiContractRunner {
    /// Returns an iterator over all contracts that match the filter.
    pub fn matching_contracts<'a>(
        &'a self,
        filter: &'a dyn TestFilter,
    ) -> impl Iterator<Item = (&ArtifactId, &(JsonAbi, Bytes, Vec<Bytes>))> {
        self.contracts.iter().filter(|&(id, (abi, _, _))| matches_contract(id, abi, filter))
    }

    /// Returns an iterator over all test functions that match the filter.
    pub fn matching_test_functions<'a>(
        &'a self,
        filter: &'a dyn TestFilter,
    ) -> impl Iterator<Item = &Function> {
        self.matching_contracts(filter)
            .flat_map(|(_, (abi, _, _))| abi.functions())
            .filter(|func| is_matching_test(func, filter))
    }

    /// Returns an iterator over all test functions in contracts that match the filter.
    pub fn all_test_functions<'a>(
        &'a self,
        filter: &'a dyn TestFilter,
    ) -> impl Iterator<Item = &Function> {
        self.contracts
            .iter()
            .filter(|(id, _)| filter.matches_path(&id.source) && filter.matches_contract(&id.name))
            .flat_map(|(_, (abi, _, _))| abi.functions())
            .filter(|func| func.is_test() || func.is_invariant_test())
    }

    /// Returns all matching tests grouped by contract grouped by file (file -> (contract -> tests))
    pub fn list(&self, filter: &dyn TestFilter) -> BTreeMap<String, BTreeMap<String, Vec<String>>> {
        self.matching_contracts(filter)
            .map(|(id, (abi, _, _))| {
                let source = id.source.as_path().display().to_string();
                let name = id.name.clone();
                let tests = abi
                    .functions()
                    .filter(|func| is_matching_test(func, filter))
                    .map(|func| func.name.clone())
                    .collect::<Vec<_>>();
                (source, name, tests)
            })
            .fold(BTreeMap::new(), |mut acc, (source, name, tests)| {
                acc.entry(source).or_default().insert(name, tests);
                acc
            })
    }

    /// Executes _all_ tests that match the given `filter`.
    ///
    /// The same as [`test`](Self::test), but returns the results instead of streaming them.
    ///
    /// Note that this method returns only when all tests have been executed.
    pub fn test_collect(&mut self, filter: &dyn TestFilter) -> BTreeMap<String, SuiteResult> {
        self.test_iter(filter).collect()
    }

    /// Executes _all_ tests that match the given `filter`.
    ///
    /// The same as [`test`](Self::test), but returns the results instead of streaming them.
    ///
    /// Note that this method returns only when all tests have been executed.
    pub fn test_iter(
        &mut self,
        filter: &dyn TestFilter,
    ) -> impl Iterator<Item = (String, SuiteResult)> {
        let (tx, rx) = mpsc::channel();
        self.test(filter, tx);
        rx.into_iter()
    }

    /// Executes _all_ tests that match the given `filter`.
    ///
    /// This will create the runtime based on the configured `evm` ops and create the `Backend`
    /// before executing all contracts and their tests in _parallel_.
    ///
    /// Each Executor gets its own instance of the `Backend`.
    pub fn test(&mut self, filter: &dyn TestFilter, tx: mpsc::Sender<(String, SuiteResult)>) {
        trace!("running all tests");

        // The DB backend that serves all the data.
        let db = Backend::spawn(self.fork.take());
        let executor = ExecutorBuilder::new()
            .inspectors(|stack| {
                stack
                    .cheatcodes(self.cheats_config.clone())
                    .trace(self.evm_opts.verbosity >= 3 || self.debug)
                    .debug(self.debug)
                    .coverage(self.coverage)
                    .enable_isolation(self.isolation)
            })
            .spec(self.evm_spec)
            .gas_limit(self.evm_opts.gas_limit())
            .build(self.env.clone(), db);

        let find_timer = Instant::now();
        let contracts = self.matching_contracts(filter).collect::<Vec<_>>();
        let find_time = find_timer.elapsed();
        debug!(
            "Found {} test contracts out of {} in {:?}",
            contracts.len(),
            self.contracts.len(),
            find_time,
        );

        contracts.par_iter().for_each_with(tx, |tx, &(id, (abi, deploy_code, libs))| {
            let identifier = id.identifier();
            let executor = executor.clone();
            let result = self.run_tests(&identifier, abi, executor, deploy_code, libs, filter);
            let _ = tx.send((identifier, result));
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn run_tests(
        &self,
        name: &str,
        contract: &JsonAbi,
        executor: Executor,
        deploy_code: &Bytes,
        libs: &[Bytes],
        filter: &dyn TestFilter,
    ) -> SuiteResult {
        let mut span_name = name;
        if !enabled!(tracing::Level::TRACE) {
            span_name = get_contract_name(span_name);
        }
        let _guard = info_span!("run_tests", name = span_name).entered();

        debug!("start executing all tests in contract");

        let runner = ContractRunner::new(
            name,
            executor,
            contract,
            deploy_code,
            self.evm_opts.initial_balance,
            self.sender,
            &self.revert_decoder,
            libs,
            self.debug,
        );
        let r = runner.run_tests(filter, &self.test_options, Some(&self.known_contracts));

        debug!(duration=?r.duration, "executed all tests in contract");

        r
    }
}

/// Builder used for instantiating the multi-contract runner
#[derive(Clone, Debug, Default)]
#[must_use = "builders do nothing unless you call `build` on them"]
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
    /// Whether or not to collect debug info
    pub debug: bool,
    /// Whether to enable call isolation
    pub isolation: bool,
    /// Settings related to fuzz and/or invariant tests
    pub test_options: Option<TestOptions>,
}

impl MultiContractRunnerBuilder {
    pub fn sender(mut self, sender: Address) -> Self {
        self.sender = Some(sender);
        self
    }

    pub fn initial_balance(mut self, initial_balance: U256) -> Self {
        self.initial_balance = initial_balance;
        self
    }

    pub fn evm_spec(mut self, spec: SpecId) -> Self {
        self.evm_spec = Some(spec);
        self
    }

    pub fn with_fork(mut self, fork: Option<CreateFork>) -> Self {
        self.fork = fork;
        self
    }

    pub fn with_cheats_config(mut self, cheats_config: CheatsConfig) -> Self {
        self.cheats_config = Some(cheats_config);
        self
    }

    pub fn with_test_options(mut self, test_options: TestOptions) -> Self {
        self.test_options = Some(test_options);
        self
    }

    pub fn set_coverage(mut self, enable: bool) -> Self {
        self.coverage = enable;
        self
    }

    pub fn set_debug(mut self, enable: bool) -> Self {
        self.debug = enable;
        self
    }

    pub fn enable_isolation(mut self, enable: bool) -> Self {
        self.isolation = enable;
        self
    }

    /// Given an EVM, proceeds to return a runner which is able to execute all tests
    /// against that evm
    pub fn build(
        self,
        root: &Path,
        output: ProjectCompileOutput,
        env: revm::primitives::Env,
        evm_opts: EvmOpts,
    ) -> Result<MultiContractRunner> {
        // This is just the contracts compiled, but we need to merge this with the read cached
        // artifacts.
        let contracts = output
            .with_stripped_file_prefixes(root)
            .into_artifacts()
            .map(|(i, c)| (i, c.into_contract_bytecode()))
            .collect::<ArtifactContracts>();

        let source_paths = contracts
            .iter()
            .map(|(i, _)| (i.identifier(), root.join(&i.source).to_string_lossy().into()))
            .collect::<BTreeMap<String, String>>();

        let linker = Linker::new(root, contracts);

        // Create a mapping of name => (abi, deployment code, Vec<library deployment code>)
        let mut deployable_contracts = DeployableContracts::default();

        let mut known_contracts = ContractsByArtifact::default();

        for (id, contract) in &linker.contracts.0 {
            let Some(abi) = contract.abi.as_ref() else {
                continue;
            };

            let LinkOutput { libs_to_deploy, libraries } =
                linker.link_with_nonce_or_address(Default::default(), evm_opts.sender, 1, id)?;

            let linked_contract = linker.link(id, &libraries)?;

            // get bytes if deployable, else add to known contracts and continue.
            // interfaces and abstract contracts should be known to enable fuzzing of their ABI
            // but they should not be deployable and their source code should be skipped by the
            // debugger and linker.
            let Some(bytecode) = linked_contract
                .get_bytecode_bytes()
                .map(|b| b.into_owned())
                .filter(|b| !b.is_empty())
            else {
                known_contracts.insert(id.clone(), (abi.clone(), vec![]));
                continue;
            };

            // if it's a test, add it to deployable contracts
            if abi.constructor.as_ref().map(|c| c.inputs.is_empty()).unwrap_or(true) &&
                abi.functions().any(|func| func.name.is_test() || func.name.is_invariant_test())
            {
                deployable_contracts.insert(id.clone(), (abi.clone(), bytecode, libs_to_deploy));
            }

            if let Some(bytes) = linked_contract.get_deployed_bytecode_bytes() {
                known_contracts.insert(id.clone(), (abi.clone(), bytes.into_owned().into()));
            }
        }

        let revert_decoder =
            RevertDecoder::new().with_abis(known_contracts.values().map(|(abi, _)| abi));
        Ok(MultiContractRunner {
            contracts: deployable_contracts,
            known_contracts,
            evm_opts,
            env,
            evm_spec: self.evm_spec.unwrap_or(SpecId::MERGE),
            sender: self.sender,
            revert_decoder,
            source_paths,
            fork: self.fork,
            cheats_config: self.cheats_config.unwrap_or_default().into(),
            coverage: self.coverage,
            debug: self.debug,
            test_options: self.test_options.unwrap_or_default(),
            isolation: self.isolation,
        })
    }
}

pub fn matches_contract(id: &ArtifactId, abi: &JsonAbi, filter: &dyn TestFilter) -> bool {
    (filter.matches_path(&id.source) && filter.matches_contract(&id.name)) &&
        abi.functions().any(|func| is_matching_test(func, filter))
}

/// Returns `true` if the function is a test function that matches the given filter.
pub(crate) fn is_matching_test(func: &Function, filter: &dyn TestFilter) -> bool {
    (func.is_test() || func.is_invariant_test()) && filter.matches_test(&func.signature())
}
