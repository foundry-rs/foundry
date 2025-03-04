//! Forge test runner for multiple contracts.

use crate::{
    progress::TestsProgress, result::SuiteResult, runner::LIBRARY_DEPLOYER, ContractRunner,
    TestFilter,
};
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{Address, Bytes, U256};
use eyre::Result;
use foundry_common::{get_contract_name, shell::verbosity, ContractsByArtifact, TestFunctionExt};
use foundry_compilers::{
    artifacts::{Contract, Libraries},
    compilers::Compiler,
    Artifact, ArtifactId, ProjectCompileOutput,
};
use foundry_config::{Config, InlineConfig};
use foundry_evm::{
    backend::Backend,
    decode::RevertDecoder,
    executors::{Executor, ExecutorBuilder},
    fork::CreateFork,
    inspectors::CheatsConfig,
    opts::EvmOpts,
    revm,
    traces::{InternalTraceMode, TraceMode},
};
use foundry_linking::{LinkOutput, Linker};
use rayon::prelude::*;
use revm::primitives::SpecId;
use std::{
    borrow::Borrow,
    collections::BTreeMap,
    fmt::Debug,
    path::Path,
    sync::{mpsc, Arc},
    time::Instant,
};

#[derive(Debug, Clone)]
pub struct TestContract {
    pub abi: JsonAbi,
    pub bytecode: Bytes,
}

pub type DeployableContracts = BTreeMap<ArtifactId, TestContract>;

/// A multi contract runner receives a set of contracts deployed in an EVM instance and proceeds
/// to run all test functions in these contracts.
pub struct MultiContractRunner {
    /// Mapping of contract name to JsonAbi, creation bytecode and library bytecode which
    /// needs to be deployed & linked against
    pub contracts: DeployableContracts,
    /// Known contracts linked with computed library addresses.
    pub known_contracts: ContractsByArtifact,
    /// Revert decoder. Contains all known errors and their selectors.
    pub revert_decoder: RevertDecoder,
    /// Libraries to deploy.
    pub libs_to_deploy: Vec<Bytes>,
    /// Library addresses used to link contracts.
    pub libraries: Libraries,

    /// The fork to use at launch
    pub fork: Option<CreateFork>,

    /// The base configuration for the test runner.
    pub tcfg: TestRunnerConfig,
}

impl std::ops::Deref for MultiContractRunner {
    type Target = TestRunnerConfig;

    fn deref(&self) -> &Self::Target {
        &self.tcfg
    }
}

impl std::ops::DerefMut for MultiContractRunner {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.tcfg
    }
}

impl MultiContractRunner {
    /// Returns an iterator over all contracts that match the filter.
    pub fn matching_contracts<'a: 'b, 'b>(
        &'a self,
        filter: &'b dyn TestFilter,
    ) -> impl Iterator<Item = (&'a ArtifactId, &'a TestContract)> + 'b {
        self.contracts.iter().filter(|&(id, c)| matches_contract(id, &c.abi, filter))
    }

    /// Returns an iterator over all test functions that match the filter.
    pub fn matching_test_functions<'a: 'b, 'b>(
        &'a self,
        filter: &'b dyn TestFilter,
    ) -> impl Iterator<Item = &'a Function> + 'b {
        self.matching_contracts(filter)
            .flat_map(|(_, c)| c.abi.functions())
            .filter(|func| is_matching_test(func, filter))
    }

    /// Returns an iterator over all test functions in contracts that match the filter.
    pub fn all_test_functions<'a: 'b, 'b>(
        &'a self,
        filter: &'b dyn TestFilter,
    ) -> impl Iterator<Item = &'a Function> + 'b {
        self.contracts
            .iter()
            .filter(|(id, _)| filter.matches_path(&id.source) && filter.matches_contract(&id.name))
            .flat_map(|(_, c)| c.abi.functions())
            .filter(|func| func.is_any_test())
    }

    /// Returns all matching tests grouped by contract grouped by file (file -> (contract -> tests))
    pub fn list(&self, filter: &dyn TestFilter) -> BTreeMap<String, BTreeMap<String, Vec<String>>> {
        self.matching_contracts(filter)
            .map(|(id, c)| {
                let source = id.source.as_path().display().to_string();
                let name = id.name.clone();
                let tests = c
                    .abi
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
        self.test(filter, tx, false);
        rx.into_iter()
    }

    /// Executes _all_ tests that match the given `filter`.
    ///
    /// This will create the runtime based on the configured `evm` ops and create the `Backend`
    /// before executing all contracts and their tests in _parallel_.
    ///
    /// Each Executor gets its own instance of the `Backend`.
    pub fn test(
        &mut self,
        filter: &dyn TestFilter,
        tx: mpsc::Sender<(String, SuiteResult)>,
        show_progress: bool,
    ) {
        let tokio_handle = tokio::runtime::Handle::current();
        trace!("running all tests");

        // The DB backend that serves all the data.
        let db = Backend::spawn(self.fork.take());

        let find_timer = Instant::now();
        let contracts = self.matching_contracts(filter).collect::<Vec<_>>();
        let find_time = find_timer.elapsed();
        debug!(
            "Found {} test contracts out of {} in {:?}",
            contracts.len(),
            self.contracts.len(),
            find_time,
        );

        if show_progress {
            let tests_progress = TestsProgress::new(contracts.len(), rayon::current_num_threads());
            // Collect test suite results to stream at the end of test run.
            let results: Vec<(String, SuiteResult)> = contracts
                .par_iter()
                .map(|&(id, contract)| {
                    let _guard = tokio_handle.enter();
                    tests_progress.inner.lock().start_suite_progress(&id.identifier());

                    let result = self.run_test_suite(
                        id,
                        contract,
                        &db,
                        filter,
                        &tokio_handle,
                        Some(&tests_progress),
                    );

                    tests_progress
                        .inner
                        .lock()
                        .end_suite_progress(&id.identifier(), result.summary());

                    (id.identifier(), result)
                })
                .collect();

            tests_progress.inner.lock().clear();

            results.iter().for_each(|result| {
                let _ = tx.send(result.to_owned());
            });
        } else {
            contracts.par_iter().for_each(|&(id, contract)| {
                let _guard = tokio_handle.enter();
                let result = self.run_test_suite(id, contract, &db, filter, &tokio_handle, None);
                let _ = tx.send((id.identifier(), result));
            })
        }
    }

    fn run_test_suite(
        &self,
        artifact_id: &ArtifactId,
        contract: &TestContract,
        db: &Backend,
        filter: &dyn TestFilter,
        tokio_handle: &tokio::runtime::Handle,
        progress: Option<&TestsProgress>,
    ) -> SuiteResult {
        let identifier = artifact_id.identifier();
        let mut span_name = identifier.as_str();

        if !enabled!(tracing::Level::TRACE) {
            span_name = get_contract_name(&identifier);
        }
        let span = debug_span!("suite", name = %span_name);
        let span_local = span.clone();
        let _guard = span_local.enter();

        debug!("start executing all tests in contract");

        let runner = ContractRunner::new(
            &identifier,
            contract,
            self.tcfg.executor(self.known_contracts.clone(), artifact_id, db.clone()),
            progress,
            tokio_handle,
            span,
            self,
        );
        let r = runner.run_tests(filter);

        debug!(duration=?r.duration, "executed all tests in contract");

        r
    }
}

/// Configuration for the test runner.
///
/// This is modified after instantiation through inline config.
#[derive(Clone)]
pub struct TestRunnerConfig {
    /// Project config.
    pub config: Arc<Config>,
    /// Inline configuration.
    pub inline_config: Arc<InlineConfig>,

    /// EVM configuration.
    pub evm_opts: EvmOpts,
    /// EVM environment.
    pub env: revm::primitives::Env,
    /// EVM version.
    pub spec_id: SpecId,
    /// The address which will be used to deploy the initial contracts and send all transactions.
    pub sender: Address,

    /// Whether to collect coverage info
    pub coverage: bool,
    /// Whether to collect debug info
    pub debug: bool,
    /// Whether to enable steps tracking in the tracer.
    pub decode_internal: InternalTraceMode,
    /// Whether to enable call isolation.
    pub isolation: bool,
    /// Whether to enable Odyssey features.
    pub odyssey: bool,
}

impl TestRunnerConfig {
    /// Reconfigures all fields using the given `config`.
    /// This is for example used to override the configuration with inline config.
    pub fn reconfigure_with(&mut self, config: Arc<Config>) {
        debug_assert!(!Arc::ptr_eq(&self.config, &config));

        self.spec_id = config.evm_spec_id();
        self.sender = config.sender;
        self.odyssey = config.odyssey;
        self.isolation = config.isolate;

        // Specific to Forge, not present in config.
        // TODO: self.evm_opts
        // TODO: self.env
        // self.coverage = N/A;
        // self.debug = N/A;
        // self.decode_internal = N/A;

        self.config = config;
    }

    /// Configures the given executor with this configuration.
    pub fn configure_executor(&self, executor: &mut Executor) {
        // TODO: See above

        let inspector = executor.inspector_mut();
        // inspector.set_env(&self.env);
        if let Some(cheatcodes) = inspector.cheatcodes.as_mut() {
            cheatcodes.config =
                Arc::new(cheatcodes.config.clone_with(&self.config, self.evm_opts.clone()));
        }
        inspector.tracing(self.trace_mode());
        inspector.collect_coverage(self.coverage);
        inspector.enable_isolation(self.isolation);
        inspector.odyssey(self.odyssey);
        // inspector.set_create2_deployer(self.evm_opts.create2_deployer);

        // executor.env_mut().clone_from(&self.env);
        executor.set_spec_id(self.spec_id);
        // executor.set_gas_limit(self.evm_opts.gas_limit());
        executor.set_legacy_assertions(self.config.legacy_assertions);
    }

    /// Creates a new executor with this configuration.
    pub fn executor(
        &self,
        known_contracts: ContractsByArtifact,
        artifact_id: &ArtifactId,
        db: Backend,
    ) -> Executor {
        let cheats_config = Arc::new(CheatsConfig::new(
            &self.config,
            self.evm_opts.clone(),
            Some(known_contracts),
            Some(artifact_id.clone()),
        ));
        ExecutorBuilder::new()
            .inspectors(|stack| {
                stack
                    .cheatcodes(cheats_config)
                    .trace_mode(self.trace_mode())
                    .coverage(self.coverage)
                    .enable_isolation(self.isolation)
                    .odyssey(self.odyssey)
                    .create2_deployer(self.evm_opts.create2_deployer)
            })
            .spec_id(self.spec_id)
            .gas_limit(self.evm_opts.gas_limit())
            .legacy_assertions(self.config.legacy_assertions)
            .build(self.env.clone(), db)
    }

    fn trace_mode(&self) -> TraceMode {
        TraceMode::default()
            .with_debug(self.debug)
            .with_decode_internal(self.decode_internal)
            .with_verbosity(self.evm_opts.verbosity)
            .with_state_changes(verbosity() > 4)
    }
}

/// Builder used for instantiating the multi-contract runner
#[derive(Clone, Debug)]
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
    /// Project config.
    pub config: Arc<Config>,
    /// Whether or not to collect coverage info
    pub coverage: bool,
    /// Whether or not to collect debug info
    pub debug: bool,
    /// Whether to enable steps tracking in the tracer.
    pub decode_internal: InternalTraceMode,
    /// Whether to enable call isolation
    pub isolation: bool,
    /// Whether to enable Odyssey features.
    pub odyssey: bool,
}

impl MultiContractRunnerBuilder {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            sender: Default::default(),
            initial_balance: Default::default(),
            evm_spec: Default::default(),
            fork: Default::default(),
            coverage: Default::default(),
            debug: Default::default(),
            isolation: Default::default(),
            decode_internal: Default::default(),
            odyssey: Default::default(),
        }
    }

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

    pub fn set_coverage(mut self, enable: bool) -> Self {
        self.coverage = enable;
        self
    }

    pub fn set_debug(mut self, enable: bool) -> Self {
        self.debug = enable;
        self
    }

    pub fn set_decode_internal(mut self, mode: InternalTraceMode) -> Self {
        self.decode_internal = mode;
        self
    }

    pub fn enable_isolation(mut self, enable: bool) -> Self {
        self.isolation = enable;
        self
    }

    pub fn odyssey(mut self, enable: bool) -> Self {
        self.odyssey = enable;
        self
    }

    /// Given an EVM, proceeds to return a runner which is able to execute all tests
    /// against that evm
    pub fn build<C: Compiler<CompilerContract = Contract>>(
        self,
        root: &Path,
        output: &ProjectCompileOutput,
        env: revm::primitives::Env,
        evm_opts: EvmOpts,
    ) -> Result<MultiContractRunner> {
        let contracts = output
            .artifact_ids()
            .map(|(id, v)| (id.with_stripped_file_prefixes(root), v))
            .collect();
        let linker = Linker::new(root, contracts);

        // Build revert decoder from ABIs of all artifacts.
        let abis = linker
            .contracts
            .iter()
            .filter_map(|(_, contract)| contract.abi.as_ref().map(|abi| abi.borrow()));
        let revert_decoder = RevertDecoder::new().with_abis(abis);

        let LinkOutput { libraries, libs_to_deploy } = linker.link_with_nonce_or_address(
            Default::default(),
            LIBRARY_DEPLOYER,
            0,
            linker.contracts.keys(),
        )?;

        let linked_contracts = linker.get_linked_artifacts(&libraries)?;

        // Create a mapping of name => (abi, deployment code, Vec<library deployment code>)
        let mut deployable_contracts = DeployableContracts::default();

        for (id, contract) in linked_contracts.iter() {
            let Some(abi) = &contract.abi else { continue };

            // if it's a test, link it and add to deployable contracts
            if abi.constructor.as_ref().map(|c| c.inputs.is_empty()).unwrap_or(true) &&
                abi.functions().any(|func| func.name.is_any_test())
            {
                let Some(bytecode) =
                    contract.get_bytecode_bytes().map(|b| b.into_owned()).filter(|b| !b.is_empty())
                else {
                    continue;
                };

                deployable_contracts
                    .insert(id.clone(), TestContract { abi: abi.clone(), bytecode });
            }
        }

        let known_contracts = ContractsByArtifact::new(linked_contracts);

        Ok(MultiContractRunner {
            contracts: deployable_contracts,
            revert_decoder,
            known_contracts,
            libs_to_deploy,
            libraries,

            fork: self.fork,

            tcfg: TestRunnerConfig {
                evm_opts,
                env,
                spec_id: self.evm_spec.unwrap_or_else(|| self.config.evm_spec_id()),
                sender: self.sender.unwrap_or(self.config.sender),

                coverage: self.coverage,
                debug: self.debug,
                decode_internal: self.decode_internal,
                inline_config: Arc::new(InlineConfig::new_parsed(output, &self.config)?),
                isolation: self.isolation,
                odyssey: self.odyssey,

                config: self.config,
            },
        })
    }
}

pub fn matches_contract(id: &ArtifactId, abi: &JsonAbi, filter: &dyn TestFilter) -> bool {
    (filter.matches_path(&id.source) && filter.matches_contract(&id.name)) &&
        abi.functions().any(|func| is_matching_test(func, filter))
}

/// Returns `true` if the function is a test function that matches the given filter.
pub(crate) fn is_matching_test(func: &Function, filter: &dyn TestFilter) -> bool {
    func.is_any_test() && filter.matches_test(&func.signature())
}
