//! Forge test runner for multiple contracts.

use crate::{
    ContractRunner, TestFilter,
    progress::TestsProgress,
    result::{SuiteResult, SymbolicCounterexampleArtifact, SymbolicCounterexampleArtifactKind},
    runner::{
        ContractRunnerContext, InvariantCampaignScope, LIBRARY_DEPLOYER,
        count_runnable_invariant_campaign_anchors,
    },
};
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{Address, Bytes, U256};
use eyre::Result;
use foundry_cli::opts::configure_pcx_from_compile_output;
use foundry_common::{
    ContractsByArtifact, ContractsByArtifactBuilder, EmptyTestFilter, TestFunctionKind,
    get_contract_name,
};
use foundry_compilers::{
    Artifact, ArtifactId, Compiler, ProjectCompileOutput,
    artifacts::{Contract, Libraries},
};
use foundry_config::{Config, InlineConfig};
use foundry_evm::{
    backend::Backend,
    core::evm::{EvmEnvFor, FoundryEvmNetwork, SpecFor, TxEnvFor},
    decode::RevertDecoder,
    executors::{EarlyExit, Executor, ExecutorBuilder, ReplayObservation, ShowmapDomain},
    fork::CreateFork,
    fuzz::{
        BasicTxDetails,
        strategies::{EnumBounds, LiteralsDictionary},
    },
    inspectors::{CheatsConfig, EdgeIndexMap},
    opts::EvmOpts,
    traces::{InternalTraceMode, TraceRequirements},
};
use foundry_evm_networks::NetworkVariant;

use foundry_linking::{LinkOutput, Linker};
use rayon::prelude::*;
use std::{
    borrow::Borrow,
    collections::BTreeMap,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
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
#[derive(Clone, Debug)]
pub struct MultiContractRunner<FEN: FoundryEvmNetwork> {
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
    /// Solar compiler instance, to grant syntactic and semantic analysis capabilities
    pub analysis: Arc<solar::sema::Compiler>,
    /// Literals dictionary for fuzzing.
    pub fuzz_literals: LiteralsDictionary,
    /// Literals dictionary for invariant fuzzing.
    pub invariant_literals: LiteralsDictionary,
    /// Variant counts for project enums, used to constrain fuzzed enum inputs.
    pub enum_bounds: EnumBounds,

    /// The fork to use at launch
    pub fork: Option<CreateFork>,

    /// The base configuration for the test runner.
    pub tcfg: TestRunnerConfig<FEN>,
}

impl<FEN: FoundryEvmNetwork> Deref for MultiContractRunner<FEN> {
    type Target = TestRunnerConfig<FEN>;

    fn deref(&self) -> &Self::Target {
        &self.tcfg
    }
}

impl<FEN: FoundryEvmNetwork> DerefMut for MultiContractRunner<FEN> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.tcfg
    }
}

impl<FEN: FoundryEvmNetwork> MultiContractRunner<FEN> {
    fn test_function_matcher(&self) -> TestFunctionMatcher<'_> {
        TestFunctionMatcher::new(
            &self.config,
            &self.inline_config,
            self.tcfg.symbolic_artifact_replay.as_ref(),
        )
    }

    /// Returns an iterator over all contracts that match the filter.
    pub fn matching_contracts<'a: 'b, 'b>(
        &'a self,
        filter: &'b dyn TestFilter,
    ) -> impl Iterator<Item = (&'a ArtifactId, &'a TestContract)> + 'b {
        let matcher = self.test_function_matcher();
        self.contracts.iter().filter(move |&(id, c)| matcher.matches_contract(filter, id, &c.abi))
    }

    /// Returns an iterator over all test functions that match the filter.
    pub fn matching_test_functions<'a: 'b, 'b>(
        &'a self,
        filter: &'b dyn TestFilter,
    ) -> impl Iterator<Item = &'a Function> + 'b {
        let matcher = self.test_function_matcher();
        self.matching_contracts(filter).flat_map(move |(id, c)| {
            let identifier = id.identifier();
            c.abi
                .functions()
                .filter(move |func| matcher.matches_test_function(filter, &identifier, func))
        })
    }

    /// Returns an iterator over all test functions in contracts that match the filter.
    pub fn all_test_functions<'a: 'b, 'b>(
        &'a self,
        filter: &'b dyn TestFilter,
    ) -> impl Iterator<Item = &'a Function> + 'b {
        let matcher = self.test_function_matcher();
        self.contracts
            .iter()
            .filter(|(id, _)| filter.matches_path(&id.source) && filter.matches_contract(&id.name))
            .flat_map(move |(id, c)| {
                let identifier = id.identifier();
                c.abi
                    .functions()
                    .filter(move |func| matcher.test_function_kind(&identifier, func).is_any_test())
            })
    }

    /// Returns all matching tests grouped by contract grouped by file (file -> (contract -> tests))
    pub fn list(&self, filter: &dyn TestFilter) -> BTreeMap<String, BTreeMap<String, Vec<String>>> {
        let matcher = self.test_function_matcher();
        self.matching_contracts(filter)
            .map(move |(id, c)| {
                let source = id.source.as_path().display().to_string();
                let name = id.name.clone();
                let identifier = id.identifier();
                let tests = c
                    .abi
                    .functions()
                    // TODO(@mablr): in fuzz-only mode, make `--list` mirror execution
                    // by hiding unit/table/symbolic tests that `forge fuzz run/replay` skips.
                    .filter(|func| matcher.matches_test_function(filter, &identifier, func))
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
    pub fn test_collect(
        &mut self,
        filter: &dyn TestFilter,
    ) -> Result<BTreeMap<String, SuiteResult>> {
        Ok(self.test_iter(filter)?.collect())
    }

    /// Executes _all_ tests that match the given `filter`.
    ///
    /// The same as [`test`](Self::test), but returns the results instead of streaming them.
    ///
    /// Note that this method returns only when all tests have been executed.
    pub fn test_iter(
        &mut self,
        filter: &dyn TestFilter,
    ) -> Result<impl Iterator<Item = (String, SuiteResult)>> {
        let (tx, rx) = mpsc::channel();
        self.test(filter, tx, false)?;
        Ok(rx.into_iter())
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
    ) -> Result<()> {
        let tokio_handle = tokio::runtime::Handle::current();
        trace!("running all tests");

        // The DB backend that serves all the data.
        let db = Backend::spawn(self.fork.take())?;

        let find_timer = Instant::now();
        let contracts = self.matching_contracts(filter).collect::<Vec<_>>();
        let find_time = find_timer.elapsed();
        debug!(
            "Found {} test contracts out of {} in {:?}",
            contracts.len(),
            self.contracts.len(),
            find_time,
        );
        let num_invariant_campaign_anchors = contracts
            .iter()
            .map(|(id, contract)| {
                count_runnable_invariant_campaign_anchors(
                    &contract.abi,
                    filter,
                    InvariantCampaignScope {
                        config: &self.tcfg.config,
                        inline_config: &self.tcfg.inline_config,
                        contract_name: &id.identifier(),
                        all_override_networks: &self.tcfg.multi_network.all_override_networks,
                        pass_network: self.tcfg.multi_network.pass_network.as_ref(),
                    },
                )
            })
            .sum();

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
                        ContractRunnerContext {
                            progress: Some(&tests_progress),
                            tokio_handle: tokio_handle.clone(),
                            num_invariant_campaign_anchors,
                        },
                    );

                    tests_progress
                        .inner
                        .lock()
                        .end_suite_progress(&id.identifier(), result.summary());

                    (id.identifier(), result)
                })
                .collect();

            tests_progress.inner.lock().clear();

            for result in &results {
                let _ = tx.send(result.to_owned());
            }
        } else {
            contracts.par_iter().for_each(|&(id, contract)| {
                let _guard = tokio_handle.enter();
                let result = self.run_test_suite(
                    id,
                    contract,
                    &db,
                    filter,
                    ContractRunnerContext {
                        progress: None,
                        tokio_handle: tokio_handle.clone(),
                        num_invariant_campaign_anchors,
                    },
                );
                let _ = tx.send((id.identifier(), result));
            })
        }

        Ok(())
    }

    fn run_test_suite(
        &self,
        artifact_id: &ArtifactId,
        contract: &TestContract,
        db: &Backend<FEN>,
        filter: &dyn TestFilter,
        context: ContractRunnerContext<'_>,
    ) -> SuiteResult {
        let identifier = artifact_id.identifier();
        let span_name = if enabled!(tracing::Level::TRACE) {
            identifier.as_str()
        } else {
            get_contract_name(&identifier)
        };
        let span = debug_span!("suite", name = %span_name);
        let span_local = span.clone();
        let _guard = span_local.enter();

        debug!("start executing all tests in contract");

        let executor = self.tcfg.executor(
            self.known_contracts.clone(),
            self.analysis.clone(),
            artifact_id,
            db.clone(),
        );
        let runner = ContractRunner::new(&identifier, contract, executor, span, self, context);
        let r = runner.run_tests(filter);

        debug!(duration=?r.duration, "executed all tests in contract");

        r
    }
}

/// Tracks network assignment across a multi-network test run.
///
/// When inline config specifies different networks for different tests, the runner performs one
/// pass per distinct network. This struct encodes which pass we're in so each `ContractRunner`
/// can skip tests that belong to a different pass.
///
/// Default (empty `all_override_networks`, `None` pass) = single-pass mode, every test runs.
#[derive(Clone, Debug, Default)]
pub struct MultiNetworkConfig {
    /// All networks explicitly referenced in inline config annotations across the whole suite.
    /// Empty means single-pass mode (no per-test network overrides present).
    pub all_override_networks: Vec<NetworkVariant>,
    /// The network this pass is responsible for.
    /// `None` = default pass: runs tests *without* an explicit network annotation (or annotated
    /// with a network not in `all_override_networks`).
    /// `Some(v)` = override pass: runs only tests annotated with exactly `v`.
    pub pass_network: Option<NetworkVariant>,
}

/// CLI-only options that switch fuzz/invariant tests into corpus replay
/// mode that emits AFL-`afl-showmap`-style coverage files.
#[derive(Clone, Debug)]
pub struct ShowmapConfig {
    /// Output root directory for showmap files.
    pub out_dir: PathBuf,
    /// Approach name; used as a subdirectory under `out_dir`.
    pub approach: String,
    /// Trial identifier embedded in each emitted filename to keep reruns separate.
    pub trial: String,
    /// One file per corpus entry instead of one aggregated file per test.
    pub per_input: bool,
    /// Which bitmap(s) to dump.
    pub domain: ShowmapDomain,
    /// Optional override for the corpus directory to replay from.
    /// When unset, the per-test corpus dir derived from config is used.
    pub corpus_dir: Option<PathBuf>,
    /// Whether replay should emit showmap files.
    pub emit_files: bool,
}

pub type FuzzMinimizeEdgeIndices = Arc<Mutex<BTreeMap<String, Arc<Mutex<EdgeIndexMap>>>>>;

/// CLI-only options that switch fuzz/invariant tests into single-entry replay
/// mode for corpus minimization.
#[derive(Clone, Debug)]
pub struct FuzzMinimizeConfig {
    /// Entry to replay.
    pub input: Arc<[BasicTxDetails]>,
    /// Shared edge-index assignments for all candidate replays in this minimization invocation,
    /// namespaced by matched target.
    pub evm_edge_indices: FuzzMinimizeEdgeIndices,
    /// Shared replay observations collected from matched fuzz/invariant tests.
    pub observations: Arc<Mutex<Vec<FuzzMinimizeObservation>>>,
}

/// Replay observation for one matched minimization target.
#[derive(Clone, Debug)]
pub struct FuzzMinimizeObservation {
    /// Stable target identity for this minimization run.
    pub target: String,
    /// Replay result for this target.
    pub observation: ReplayObservation,
}

#[derive(Clone, Debug)]
pub struct SymbolicArtifactReplayConfig {
    /// Artifact payload to replay.
    pub artifact: SymbolicCounterexampleArtifact,
    /// Path the artifact was loaded from, used in diagnostics.
    pub path: PathBuf,
}

/// Configuration for the test runner.
///
/// This is modified after instantiation through inline config.
#[derive(Clone, Debug)]
pub struct TestRunnerConfig<FEN: FoundryEvmNetwork> {
    /// Project config.
    pub config: Arc<Config>,
    /// Inline configuration.
    pub inline_config: Arc<InlineConfig>,

    /// EVM configuration.
    pub evm_opts: EvmOpts,
    /// EVM environment.
    pub evm_env: EvmEnvFor<FEN>,
    /// Transaction environment.
    pub tx_env: TxEnvFor<FEN>,
    /// EVM version.
    pub spec_id: SpecFor<FEN>,
    /// The address which will be used to deploy the initial contracts and send all transactions.
    pub sender: Address,

    /// Whether to collect line coverage info
    pub line_coverage: bool,
    /// Whether to collect debug info
    pub debug: bool,
    /// Whether to enable steps tracking in the tracer.
    pub decode_internal: InternalTraceMode,
    /// Whether to record every opcode step without debugger snapshots.
    pub record_all_steps: bool,
    /// Whether to enable call isolation.
    pub isolation: bool,
    /// Whether to exit early on test failure or if test run interrupted.
    pub early_exit: EarlyExit,

    /// Multi-network pass configuration. Default = single-pass mode.
    pub multi_network: MultiNetworkConfig,

    /// When set, fuzz/invariant tests run in corpus replay mode and emit
    /// AFL-`afl-showmap`-style files instead of running a campaign.
    pub showmap: Option<ShowmapConfig>,
    /// When set, fuzz/invariant tests replay one candidate input and record minimization facts.
    pub fuzz_minimize: Option<FuzzMinimizeConfig>,
    /// Run only fuzz and invariant tests.
    pub fuzz_only: bool,
    /// Replay persisted fuzz failures without running a new fuzz campaign.
    pub fuzz_failure_replay: bool,

    /// When set, run only the matching test and replay this artifact's concrete payload.
    pub symbolic_artifact_replay: Option<SymbolicArtifactReplayConfig>,
}

impl<FEN: FoundryEvmNetwork> TestRunnerConfig<FEN> {
    /// Reconfigures all fields using the given `config`.
    /// This is for example used to override the configuration with inline config.
    pub fn reconfigure_with(&mut self, config: Arc<Config>) {
        debug_assert!(!Arc::ptr_eq(&self.config, &config));

        self.spec_id = config.evm_spec_id();
        self.sender = config.sender;
        self.evm_opts.networks = config.networks;
        self.isolation = config.isolate;

        // Specific to Forge, not present in config.
        // self.line_coverage = N/A;
        // self.debug = N/A;
        // self.decode_internal = N/A;
        // self.record_all_steps = N/A;

        // TODO: self.evm_opts
        self.evm_opts.always_use_create_2_factory = config.always_use_create_2_factory;

        // TODO: self.env

        self.config = config;
    }

    /// Configures the given executor with this configuration.
    pub fn configure_executor(&self, executor: &mut Executor<FEN>) {
        // TODO: See above

        let inspector = executor.inspector_mut();
        // inspector.set_env(&self.env);
        if let Some(cheatcodes) = inspector.cheatcodes.as_mut() {
            let mut config = cheatcodes.config.clone_with(&self.config, self.evm_opts.clone());
            config.isolate = self.isolation;
            cheatcodes.config = Arc::new(config);
        }
        inspector.tracing_requirements(self.trace_requirements());
        inspector.collect_line_coverage(self.line_coverage);
        inspector.enable_isolation(self.isolation);
        inspector.networks(self.evm_opts.networks);
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
        analysis: Arc<solar::sema::Compiler>,
        artifact_id: &ArtifactId,
        db: Backend<FEN>,
    ) -> Executor<FEN> {
        let mut cheats_config = CheatsConfig::new(
            &self.config,
            self.evm_opts.clone(),
            Some(known_contracts),
            Some(artifact_id.clone()),
            None,
            false,
        );
        cheats_config.isolate = self.isolation;
        let cheats_config = Arc::new(cheats_config);
        ExecutorBuilder::default()
            .inspectors(|stack| {
                stack
                    .logs(self.config.live_logs)
                    .cheatcodes(cheats_config)
                    .trace_requirements(self.trace_requirements())
                    .line_coverage(self.line_coverage)
                    .enable_isolation(self.isolation)
                    .networks(self.evm_opts.networks)
                    .create2_deployer(self.evm_opts.create2_deployer)
                    .set_analysis(analysis)
            })
            .spec_id(self.spec_id)
            .gas_limit(self.evm_opts.gas_limit())
            .legacy_assertions(self.config.legacy_assertions)
            .build(self.evm_env.clone(), self.tx_env.clone(), db)
    }

    const fn trace_requirements(&self) -> TraceRequirements {
        TraceRequirements::none()
            .with_debug(self.debug)
            .with_decode_internal(self.decode_internal)
            .with_all_steps(self.record_all_steps)
            .with_verbosity(self.evm_opts.verbosity)
    }
}

/// Builder used for instantiating the multi-contract runner
#[derive(Clone)]
#[must_use = "builders do nothing unless you call `build` on them"]
pub struct MultiContractRunnerBuilder {
    /// The address which will be used to deploy the initial contracts and send all
    /// transactions
    pub sender: Option<Address>,
    /// The initial balance for each one of the deployed smart contracts
    pub initial_balance: U256,
    /// The fork to use at launch
    pub fork: Option<CreateFork>,
    /// Project config.
    pub config: Arc<Config>,
    /// Parsed inline configuration.
    pub inline_config: Arc<InlineConfig>,
    /// Whether or not to collect line coverage info
    pub line_coverage: bool,
    /// Whether or not to collect debug info
    pub debug: bool,
    /// Whether to enable steps tracking in the tracer.
    pub decode_internal: InternalTraceMode,
    /// Whether to record every opcode step without debugger snapshots.
    pub record_all_steps: bool,
    /// Whether to enable call isolation
    pub isolation: bool,
    /// Whether to exit early on test failure.
    pub fail_fast: bool,
    /// Multi-network pass configuration.
    pub multi_network: MultiNetworkConfig,
    /// Showmap replay mode (CLI-only, off by default).
    pub showmap: Option<ShowmapConfig>,
    /// Fuzz minimization replay mode (CLI-only, off by default).
    pub fuzz_minimize: Option<FuzzMinimizeConfig>,
    /// Run only fuzz and invariant tests.
    pub fuzz_only: bool,
    /// Replay persisted fuzz failures without running a new fuzz campaign.
    pub fuzz_failure_replay: bool,
    /// Symbolic artifact replay mode (CLI-only, off by default).
    pub symbolic_artifact_replay: Option<SymbolicArtifactReplayConfig>,
}

impl MultiContractRunnerBuilder {
    pub fn new(config: Arc<Config>, inline_config: Arc<InlineConfig>) -> Self {
        Self {
            config,
            inline_config,
            sender: Default::default(),
            initial_balance: Default::default(),
            fork: Default::default(),
            line_coverage: Default::default(),
            debug: Default::default(),
            isolation: Default::default(),
            decode_internal: Default::default(),
            record_all_steps: Default::default(),
            fail_fast: false,
            multi_network: Default::default(),
            showmap: None,
            fuzz_minimize: None,
            fuzz_only: false,
            fuzz_failure_replay: false,
            symbolic_artifact_replay: None,
        }
    }

    pub fn with_showmap(mut self, showmap: Option<ShowmapConfig>) -> Self {
        self.showmap = showmap;
        self
    }

    pub fn with_fuzz_minimize(mut self, fuzz_minimize: Option<FuzzMinimizeConfig>) -> Self {
        self.fuzz_minimize = fuzz_minimize;
        self
    }

    pub const fn with_fuzz_only(mut self, fuzz_only: bool) -> Self {
        self.fuzz_only = fuzz_only;
        self
    }

    pub const fn with_fuzz_failure_replay(mut self, fuzz_failure_replay: bool) -> Self {
        self.fuzz_failure_replay = fuzz_failure_replay;
        self
    }

    pub fn with_symbolic_artifact_replay(
        mut self,
        replay: Option<SymbolicArtifactReplayConfig>,
    ) -> Self {
        self.symbolic_artifact_replay = replay;
        self
    }

    pub const fn sender(mut self, sender: Address) -> Self {
        self.sender = Some(sender);
        self
    }

    pub const fn initial_balance(mut self, initial_balance: U256) -> Self {
        self.initial_balance = initial_balance;
        self
    }

    pub fn with_fork(mut self, fork: Option<CreateFork>) -> Self {
        self.fork = fork;
        self
    }

    pub const fn set_coverage(mut self, enable: bool) -> Self {
        self.line_coverage = enable;
        self
    }

    pub const fn set_debug(mut self, enable: bool) -> Self {
        self.debug = enable;
        self
    }

    pub const fn set_decode_internal(mut self, mode: InternalTraceMode) -> Self {
        self.decode_internal = mode;
        self
    }

    pub const fn set_record_all_steps(mut self, enable: bool) -> Self {
        self.record_all_steps = enable;
        self
    }

    pub fn with_multi_network(mut self, multi_network: MultiNetworkConfig) -> Self {
        self.multi_network = multi_network;
        self
    }

    pub const fn fail_fast(mut self, fail_fast: bool) -> Self {
        self.fail_fast = fail_fast;
        self
    }

    pub const fn enable_isolation(mut self, enable: bool) -> Self {
        self.isolation = enable;
        self
    }

    /// Given an EVM, proceeds to return a runner which is able to execute all tests
    /// against that evm
    pub fn build<FEN: FoundryEvmNetwork, C: Compiler<CompilerContract = Contract>>(
        self,
        output: &ProjectCompileOutput,
        evm_env: EvmEnvFor<FEN>,
        tx_env: TxEnvFor<FEN>,
        evm_opts: EvmOpts,
    ) -> Result<MultiContractRunner<FEN>> {
        let root = &self.config.root;
        let contracts = output
            .artifact_ids()
            .map(|(id, v)| (id.with_stripped_file_prefixes(root), v))
            .collect();
        let linker = Linker::new(root, contracts);

        // Build revert decoder from ABIs of all artifacts.
        let abis = linker
            .contracts
            .values()
            .filter_map(|contract| contract.abi.as_ref().map(|abi| abi.borrow()));
        let revert_decoder = RevertDecoder::new().with_abis(abis);

        let LinkOutput { libraries, libs_to_deploy } = linker.link_with_nonce_or_address(
            Default::default(),
            LIBRARY_DEPLOYER,
            0,
            linker.contracts.keys(),
        )?;

        let linked_contracts = linker.get_linked_artifacts_cow(&libraries)?;
        let inline_config = self.inline_config;

        // Create a mapping of name => (abi, deployment code, Vec<library deployment code>)
        let mut deployable_contracts = DeployableContracts::default();
        let test_matcher = TestFunctionMatcher::new(
            &self.config,
            &inline_config,
            self.symbolic_artifact_replay.as_ref(),
        );
        let empty_filter = EmptyTestFilter::default();

        for (id, contract) in linked_contracts.iter() {
            let Some(abi) = contract.abi.as_ref() else { continue };

            // if it's a test, link it and add to deployable contracts
            if abi.constructor.as_ref().map(|c| c.inputs.is_empty()).unwrap_or(true)
                && test_matcher.matches_contract(&empty_filter, id, abi.borrow())
            {
                linker.ensure_linked(contract, id)?;

                let Some(bytecode) =
                    contract.get_bytecode_bytes().map(|b| b.into_owned()).filter(|b| !b.is_empty())
                else {
                    continue;
                };

                deployable_contracts
                    .insert(id.clone(), TestContract { abi: abi.clone().into_owned(), bytecode });
            }
        }

        // Create known contracts from linked contracts and storage layout information (if any).
        let known_contracts =
            ContractsByArtifactBuilder::new(linked_contracts).with_output(output, root).build();

        // Initialize and configure the solar compiler.
        let mut analysis = solar::sema::Compiler::new(
            solar::interface::Session::builder().with_stderr_emitter().build(),
        );
        let dcx = analysis.dcx_mut();
        dcx.set_emitter(Box::new(
            solar::interface::diagnostics::HumanEmitter::stderr(Default::default())
                .source_map(Some(dcx.source_map().unwrap())),
        ));
        dcx.set_flags_mut(|f| f.track_diagnostics = false);

        // Populate solar's global context by parsing and lowering the sources.
        let files: Vec<_> = output.output().sources.as_ref().keys().cloned().collect();

        analysis.enter_mut(|compiler| -> Result<()> {
            let mut pcx = compiler.parse();
            configure_pcx_from_compile_output(
                &mut pcx,
                &self.config,
                output,
                if files.is_empty() { None } else { Some(&files) },
            )?;
            pcx.parse();
            let _ = compiler.lower_asts();
            Ok(())
        })?;

        let analysis = Arc::new(analysis);
        // Enum variant counts used to constrain fuzzed enum inputs to valid values.
        let enum_bounds = EnumBounds::collect(&analysis);
        let fuzz_max_literals = self.config.fuzz.dictionary.max_fuzz_dictionary_literals;
        let invariant_max_literals = self.config.invariant.dictionary.max_fuzz_dictionary_literals;
        let fuzz_literals = LiteralsDictionary::new(
            Some(analysis.clone()),
            Some(self.config.project_paths()),
            fuzz_max_literals,
        );
        let invariant_literals = if invariant_max_literals == fuzz_max_literals {
            fuzz_literals.clone()
        } else {
            LiteralsDictionary::new(
                Some(analysis.clone()),
                Some(self.config.project_paths()),
                invariant_max_literals,
            )
        };

        Ok(MultiContractRunner {
            contracts: deployable_contracts,
            revert_decoder,
            known_contracts,
            libs_to_deploy,
            libraries,
            analysis,
            fuzz_literals,
            invariant_literals,
            enum_bounds,

            tcfg: TestRunnerConfig {
                evm_opts,
                evm_env,
                tx_env,
                spec_id: self.config.evm_spec_id(),
                sender: self.sender.unwrap_or(self.config.sender),
                line_coverage: self.line_coverage,
                debug: self.debug,
                decode_internal: self.decode_internal,
                record_all_steps: self.record_all_steps,
                inline_config,
                isolation: self.isolation,
                early_exit: EarlyExit::new(self.fail_fast),
                multi_network: self.multi_network,
                showmap: self.showmap,
                fuzz_minimize: self.fuzz_minimize,
                fuzz_only: self.fuzz_only,
                fuzz_failure_replay: self.fuzz_failure_replay,
                symbolic_artifact_replay: self.symbolic_artifact_replay,
                config: self.config,
            },

            fork: self.fork,
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct TestFunctionMatcher<'a> {
    config: &'a Config,
    inline_config: &'a InlineConfig,
    symbolic_artifact_replay: Option<&'a SymbolicArtifactReplayConfig>,
}

impl<'a> TestFunctionMatcher<'a> {
    pub(crate) const fn new(
        config: &'a Config,
        inline_config: &'a InlineConfig,
        symbolic_artifact_replay: Option<&'a SymbolicArtifactReplayConfig>,
    ) -> Self {
        Self { config, inline_config, symbolic_artifact_replay }
    }

    fn symbolic_tests_enabled(&self, contract_id: &str) -> bool {
        self.symbolic_artifact_replay.is_some_and(|artifact| {
            artifact.artifact.kind == SymbolicCounterexampleArtifactKind::SingleCall
        }) || self.inline_config.contract_symbolic_enabled(
            &self.config.profile,
            contract_id,
            self.config.symbolic.enabled,
        )
    }

    pub(crate) fn test_function_kind(
        &self,
        contract_id: &str,
        func: &Function,
    ) -> TestFunctionKind {
        TestFunctionKind::classify(
            func.name.as_str(),
            !func.inputs.is_empty(),
            self.symbolic_tests_enabled(contract_id),
        )
    }

    pub(crate) fn matches_test_function(
        &self,
        filter: &dyn TestFilter,
        contract_id: &str,
        func: &Function,
    ) -> bool {
        filter.matches_test_function_kind_in_contract(
            contract_id,
            func,
            self.test_function_kind(contract_id, func),
        )
    }

    pub(crate) fn matches_contract(
        &self,
        filter: &dyn TestFilter,
        id: &ArtifactId,
        abi: &JsonAbi,
    ) -> bool {
        let identifier = id.identifier();
        matches_contract(filter, &id.source, &id.name, &identifier, abi.functions(), |func| {
            self.test_function_kind(&identifier, func)
        })
    }
}

pub(crate) fn matches_contract(
    filter: &dyn TestFilter,
    path: &Path,
    contract_name: &str,
    contract_id: &str,
    functions: impl IntoIterator<Item = impl std::borrow::Borrow<Function>>,
    test_function_kind: impl Fn(&Function) -> TestFunctionKind,
) -> bool {
    (filter.matches_path(path) && filter.matches_contract(contract_name))
        && functions.into_iter().any(|func| {
            let func = func.borrow();
            filter.matches_test_function_kind_in_contract(
                contract_id,
                func,
                test_function_kind(func),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_common::TestFunctionExt;

    #[test]
    fn matches_contract_uses_provided_function_kind() {
        let filter = EmptyTestFilter::default();
        let path = Path::new("test/Symbolic.t.sol");
        let func = Function::parse("checkFilteredCompile(uint256)").unwrap();

        assert!(matches_contract(&filter, path, "Symbolic", "Symbolic", [func.clone()], |_| {
            TestFunctionKind::SymbolicTest
        },));
        assert!(!matches_contract(&filter, path, "Symbolic", "Symbolic", [func], |func| {
            func.test_function_kind()
        }));
    }
}
