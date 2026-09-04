//! Forge test runner for multiple contracts.

use crate::{
    ContractRunner, TestFilter,
    progress::TestsProgress,
    result::{SuiteResult, SymbolicCounterexampleArtifact, SymbolicCounterexampleArtifactKind},
    runner::{
        ContractRunnerContext, InvariantCampaignScope, count_runnable_invariant_campaign_anchors,
        function_matches_network_pass,
    },
    symbolic_regression::SYMBOLIC_REGRESSION_MARKER,
};
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{Address, Bytes, ChainId, U256};
use eyre::Result;
use foundry_cli::opts::configure_pcx_from_compile_output;
use foundry_common::{
    ContractsByArtifact, ContractsByArtifactBuilder, EmptyTestFilter, LIBRARY_DEPLOYER,
    TestFunctionKind, get_contract_name,
};
use foundry_compilers::{
    Artifact, ArtifactId, Compiler, ProjectCompileOutput,
    artifacts::{Contract, Libraries},
};
use foundry_config::{Config, FoundryHardfork, InlineConfig};
use foundry_evm::{
    backend::Backend,
    core::evm::{EvmEnvFor, FoundryEvmNetwork, SpecFor, TxEnvFor},
    decode::RevertDecoder,
    executors::{EarlyExit, Executor, ExecutorBuilder, ReplayObservation, ShowmapDomain},
    fork::CreateFork,
    fuzz::{
        BaseCounterExample, BasicTxDetails,
        strategies::{EnumBounds, LiteralsDictionary},
    },
    inspectors::{CheatsConfig, EdgeIndexMap},
    opts::{EvmOpts, ExecutionSpecContext, resolve_execution_spec},
    traces::{InternalTraceMode, TraceRequirements},
};
use foundry_evm_networks::NetworkVariant;

use foundry_linking::{DetailedLinkOutput, LinkOutput, Linker, LinkerError, Resolver};
use rayon::prelude::*;
use std::{
    borrow::Borrow,
    collections::{BTreeMap, BTreeSet},
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::Instant,
};

#[derive(Debug, Clone)]
pub struct TestContract {
    pub abi: JsonAbi,
    pub bytecode: Bytes,
    pub library_addresses: BTreeSet<Address>,
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
    /// Addresses of libraries required by linked test artifacts.
    pub library_addresses: Vec<Address>,
    /// How libraries should be deployed.
    pub library_deployment: LibraryDeployment,
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

/// Forge-local library deployment strategy.
#[derive(Clone, Copy, Debug)]
pub enum LibraryDeployment {
    Nonce,
    Create2 { deployer: Address, salt: alloy_primitives::B256 },
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
    pub(crate) fn test_function_matcher(&self) -> TestFunctionMatcher<'_> {
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
        self.matching_contracts(filter)
            .flat_map(move |(id, c)| matcher.matching_test_functions(filter, id, &c.abi))
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
                matcher.test_functions(id.identifier(), &c.abi, |_, _, kind| kind.is_any_test())
            })
    }

    /// Returns all matching tests grouped by contract grouped by file (file -> (contract -> tests))
    pub fn list(&self, filter: &dyn TestFilter) -> BTreeMap<String, BTreeMap<String, Vec<String>>> {
        self.list_with(filter, |func| func.name.clone())
    }

    pub(crate) fn list_signatures(
        &self,
        filter: &dyn TestFilter,
    ) -> BTreeMap<String, BTreeMap<String, Vec<String>>> {
        self.list_with(filter, |func| func.signature())
    }

    fn list_with(
        &self,
        filter: &dyn TestFilter,
        format_test: impl Fn(&Function) -> String,
    ) -> BTreeMap<String, BTreeMap<String, Vec<String>>> {
        let matcher = self.test_function_matcher();
        let fuzz_only = self.tcfg.fuzz_only;
        let mut out = BTreeMap::<_, BTreeMap<_, _>>::new();
        for (id, c) in self.matching_contracts(filter) {
            let tests = matcher
                .test_functions(id.identifier(), &c.abi, |contract_id, func, kind| {
                    (!fuzz_only
                        || matches!(
                            kind,
                            TestFunctionKind::FuzzTest { .. } | TestFunctionKind::InvariantTest
                        ))
                        && filter.matches_test_function_kind_in_contract(contract_id, func, kind)
                })
                .map(&format_test)
                .collect::<Vec<_>>();
            if !tests.is_empty() {
                out.entry(id.source.display().to_string())
                    .or_default()
                    .insert(id.name.clone(), tests);
            }
        }
        out
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
        let (tx, rx) = mpsc::channel();
        self.test(filter, tx, false)?;
        Ok(rx.into_iter().collect())
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
        debug!(
            "Found {} test contracts out of {} in {:?}",
            contracts.len(),
            self.contracts.len(),
            find_timer.elapsed(),
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

        let progress = show_progress
            .then(|| TestsProgress::new(contracts.len(), rayon::current_num_threads()));
        let run_suite = |&(id, contract): &(&ArtifactId, &TestContract)| {
            let _guard = tokio_handle.enter();
            let identifier = id.identifier();
            if let Some(progress) = &progress {
                progress.inner.lock().start_suite_progress(&identifier);
            }
            let result = self.run_test_suite(
                id,
                contract,
                &db,
                filter,
                ContractRunnerContext {
                    progress: progress.as_ref(),
                    tokio_handle: tokio_handle.clone(),
                    num_invariant_campaign_anchors,
                },
            );
            if let Some(progress) = &progress {
                progress.inner.lock().end_suite_progress(&identifier, result.summary());
            }
            (identifier, result)
        };

        if let Some(progress) = &progress {
            // Collect test suite results to stream at the end of test run, once the progress bars
            // have been cleared.
            let results = contracts.par_iter().map(run_suite).collect::<Vec<_>>();
            progress.inner.lock().clear();
            for result in results {
                let _ = tx.send(result);
            }
        } else {
            contracts.par_iter().for_each(|contract| {
                let _ = tx.send(run_suite(contract));
            });
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
        let _guard = span.clone().entered();

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

/// Replay behavior required by a fuzz minimization command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FuzzMinimizeMode {
    /// Replay complete entries so corpus minimization observes all coverage and failures.
    Cmin,
    /// Stop at the campaign boundary so transaction minimization ignores unreachable suffixes.
    Tmin,
}

/// CLI-only options that switch fuzz/invariant tests into single-entry replay
/// mode for corpus minimization.
#[derive(Clone, Debug)]
pub struct FuzzMinimizeConfig {
    /// Entry to replay.
    pub input: Arc<[BasicTxDetails]>,
    /// Whether replay serves corpus or transaction minimization.
    pub mode: FuzzMinimizeMode,
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

/// A validated stateless fuzz failure and its unique replay target.
#[derive(Clone, Debug)]
pub struct FuzzFailureReplayConfig {
    /// Artifact payload to replay.
    pub failure: Arc<BaseCounterExample>,
    /// Fully qualified contract identifier selected for replay.
    pub contract: String,
    /// Function signature selected for replay.
    pub test: String,
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
    /// Executor construction selected by concrete network dispatch.
    pub executor_builder: ExecutorBuilder<FEN>,
    /// EVM environment.
    pub evm_env: EvmEnvFor<FEN>,
    /// Transaction environment.
    pub tx_env: TxEnvFor<FEN>,
    /// EVM version.
    pub spec_id: SpecFor<FEN>,
    /// Exact network hardfork selected for the execution environment.
    pub hardfork: Option<FoundryHardfork>,
    /// Source chain ID used to resolve fork hardfork schedules.
    pub fork_chain_id: Option<ChainId>,
    /// Exact hardfork reported by the fork endpoint.
    pub fork_hardfork: Option<FoundryHardfork>,
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
    /// Validated explicit stateless fuzz failure to replay.
    pub fuzz_input: Option<FuzzFailureReplayConfig>,

    /// When set, run only the matching test and replay this artifact's concrete payload.
    pub symbolic_artifact_replay: Option<SymbolicArtifactReplayConfig>,
}

impl<FEN: FoundryEvmNetwork> TestRunnerConfig<FEN> {
    /// Reconfigures all fields using the given `config`.
    /// This is for example used to override the configuration with inline config.
    pub fn reconfigure_with(&mut self, config: Arc<Config>) {
        debug_assert!(!Arc::ptr_eq(&self.config, &config));

        self.sender = config.sender;
        self.evm_opts.networks = config.networks;
        self.hardfork = resolve_execution_spec(
            &config,
            self.evm_opts.networks,
            &mut self.evm_env,
            ExecutionSpecContext::local_or_fork(self.fork_chain_id, self.fork_hardfork),
            None,
            None,
        );
        self.spec_id = self.evm_env.cfg_env.spec;
        self.isolation = config.isolate;
        // `line_coverage`, `debug`, `decode_internal` and `record_all_steps` are Forge-specific
        // and not present in the config.
        // TODO: `self.evm_opts` and `self.evm_env` are only partially reconfigured.
        self.evm_opts.always_use_create_2_factory = config.always_use_create_2_factory;
        self.config = config;
    }

    /// Configures the given executor with this configuration.
    pub fn configure_executor(&self, executor: &mut Executor<FEN>) {
        debug_assert!(
            executor.backend().networks().has_same_execution_profile(&self.evm_opts.networks)
        );
        debug_assert!(
            executor.inspector().networks.has_same_execution_profile(&self.evm_opts.networks)
        );
        let inspector = executor.inspector_mut();
        if let Some(cheatcodes) = inspector.cheatcodes.as_mut() {
            let mut config = cheatcodes.config.clone_with(&self.config, self.evm_opts.clone());
            config.isolate = self.isolation;
            cheatcodes.config = Arc::new(config);
        }
        inspector.tracing_requirements(self.trace_requirements());
        inspector.collect_line_coverage(self.line_coverage);
        inspector.enable_isolation(self.isolation);
        executor.set_spec_id(self.spec_id);
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
            false,
        );
        cheats_config.isolate = self.isolation;
        let cheats_config = Arc::new(cheats_config);
        self.executor_builder
            .clone()
            .inspectors(|stack| {
                stack
                    .logs(self.config.live_logs)
                    .cheatcodes(cheats_config)
                    .trace_requirements(self.trace_requirements())
                    .line_coverage(self.line_coverage)
                    .enable_isolation(self.isolation)
                    .create2_deployer(self.evm_opts.create2_deployer)
                    .set_analysis(analysis)
            })
            .spec_id(self.spec_id)
            .gas_limit(self.evm_opts.gas_limit())
            .legacy_assertions(self.config.legacy_assertions)
            .build(self.evm_env.clone(), self.tx_env.clone(), db, self.evm_opts.networks)
    }

    fn trace_requirements(&self) -> TraceRequirements {
        TraceRequirements::none()
            .with_debug(self.debug)
            .with_decode_internal(self.decode_internal)
            .with_all_steps(self.record_all_steps)
            .with_verbosity(self.config.tracing.verbosity.max(self.evm_opts.verbosity))
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
    /// Source chain ID used to resolve the fork's hardfork schedule.
    pub fork_chain_id: Option<ChainId>,
    /// Exact hardfork reported by the fork endpoint.
    pub fork_hardfork: Option<FoundryHardfork>,
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
    /// Validated explicit stateless fuzz failure to replay.
    pub fuzz_input: Option<FuzzFailureReplayConfig>,
    /// Symbolic artifact replay mode (CLI-only, off by default).
    pub symbolic_artifact_replay: Option<SymbolicArtifactReplayConfig>,
    /// Whether the configured CREATE2 deployer is available in the execution environment.
    pub create2_deployer_available: Option<bool>,
}

impl MultiContractRunnerBuilder {
    fn create2_deployer_available(&self, evm_opts: &EvmOpts) -> bool {
        self.create2_deployer_available.unwrap_or_else(|| {
            self.fork.is_none()
                && evm_opts.fork_url.is_none()
                && evm_opts.create2_deployer == foundry_evm::constants::DEFAULT_CREATE2_DEPLOYER
        })
    }

    pub fn new(config: Arc<Config>, inline_config: Arc<InlineConfig>) -> Self {
        Self {
            config,
            inline_config,
            sender: None,
            initial_balance: U256::ZERO,
            fork: None,
            fork_chain_id: None,
            fork_hardfork: None,
            line_coverage: false,
            debug: false,
            isolation: false,
            decode_internal: Default::default(),
            record_all_steps: false,
            fail_fast: false,
            multi_network: Default::default(),
            showmap: None,
            fuzz_minimize: None,
            fuzz_only: false,
            fuzz_failure_replay: false,
            fuzz_input: None,
            symbolic_artifact_replay: None,
            create2_deployer_available: None,
        }
    }

    pub const fn with_create2_deployer_available(mut self, available: bool) -> Self {
        self.create2_deployer_available = Some(available);
        self
    }

    pub fn with_showmap(mut self, showmap: Option<ShowmapConfig>) -> Self {
        self.showmap = showmap;
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

    pub fn with_fuzz_input(mut self, fuzz_input: Option<FuzzFailureReplayConfig>) -> Self {
        self.fuzz_input = fuzz_input;
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

    pub const fn with_fork_chain_id(mut self, chain_id: Option<ChainId>) -> Self {
        self.fork_chain_id = chain_id;
        self
    }

    pub const fn with_fork_hardfork(mut self, hardfork: Option<FoundryHardfork>) -> Self {
        self.fork_hardfork = hardfork;
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
        mut evm_env: EvmEnvFor<FEN>,
        tx_env: TxEnvFor<FEN>,
        evm_opts: EvmOpts,
        executor_builder: ExecutorBuilder<FEN>,
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

        let configured_libraries = self.config.libraries_with_remappings()?;
        let create2 = if self.create2_deployer_available(&evm_opts) {
            match linker.link_with_create2_detailed(
                configured_libraries.clone(),
                evm_opts.create2_deployer,
                self.config.create2_library_salt,
                linker.contracts.keys(),
            ) {
                Ok(output) => Some(output),
                Err(LinkerError::CyclicDependency) => None,
                Err(err) => return Err(err.into()),
            }
        } else {
            None
        };
        let (
            DetailedLinkOutput {
                output: LinkOutput { libraries, library_addresses, libs_to_deploy },
                artifact_libraries,
                ..
            },
            library_deployment,
        ) = match create2 {
            Some(output) => {
                let deployment = if output.output.libs_to_deploy.is_empty() {
                    LibraryDeployment::Nonce
                } else {
                    LibraryDeployment::Create2 {
                        deployer: evm_opts.create2_deployer,
                        salt: self.config.create2_library_salt,
                    }
                };
                (output, deployment)
            }
            None => (
                linker.link_with_nonce_or_address_detailed(
                    configured_libraries,
                    LIBRARY_DEPLOYER,
                    0,
                    linker.contracts.keys(),
                )?,
                LibraryDeployment::Nonce,
            ),
        };

        let linked_contracts = linker
            .get_linked_artifacts_cow_with_artifact_libraries(&libraries, &artifact_libraries)?;
        let inline_config = self.inline_config;

        // Collect every deployable test contract: a test contract with a default constructor.
        let mut deployable_contracts = DeployableContracts::default();
        let test_matcher = TestFunctionMatcher::new(
            &self.config,
            &inline_config,
            self.symbolic_artifact_replay.as_ref(),
        );
        let empty_filter = EmptyTestFilter::default();
        let resolver = Resolver::new(&linker);
        for (id, contract) in linked_contracts.iter() {
            let Some(abi) = contract.abi.as_ref() else { continue };
            if abi.constructor.as_ref().is_some_and(|c| !c.inputs.is_empty())
                || !test_matcher.matches_contract(&empty_filter, id, abi)
            {
                continue;
            }
            linker.ensure_linked(contract, id)?;
            let Some(bytecode) =
                contract.get_bytecode_bytes().map(|b| b.into_owned()).filter(|b| !b.is_empty())
            else {
                continue;
            };
            let artifact_libraries = artifact_libraries.get(id).unwrap_or(&libraries);
            let library_addresses = resolver.linked_library_addresses(id, artifact_libraries)?;
            deployable_contracts.insert(
                id.clone(),
                TestContract { abi: abi.clone().into_owned(), bytecode, library_addresses },
            );
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
                (!files.is_empty()).then_some(&files),
            )?;
            pcx.parse();
            let _ = compiler.lower_asts();
            Ok(())
        })?;
        let analysis = Arc::new(analysis);

        // Enum variant counts used to constrain fuzzed enum inputs to valid values.
        let enum_bounds = EnumBounds::collect(&analysis);
        let literals = |max_literals| {
            LiteralsDictionary::new(
                Some(analysis.clone()),
                Some(self.config.project_paths()),
                max_literals,
            )
        };
        let fuzz_max_literals = self.config.fuzz.dictionary.max_fuzz_dictionary_literals;
        let invariant_max_literals = self.config.invariant.dictionary.max_fuzz_dictionary_literals;
        let fuzz_literals = literals(fuzz_max_literals);
        let invariant_literals = if invariant_max_literals == fuzz_max_literals {
            fuzz_literals.clone()
        } else {
            literals(invariant_max_literals)
        };

        let fork_chain_id = self.fork_chain_id.or_else(|| {
            (self.fork.is_some() || evm_opts.fork_url.is_some()).then_some(evm_env.cfg_env.chain_id)
        });
        let hardfork = resolve_execution_spec(
            &self.config,
            evm_opts.networks,
            &mut evm_env,
            ExecutionSpecContext::local_or_fork(fork_chain_id, self.fork_hardfork),
            None,
            None,
        );
        let spec_id = evm_env.cfg_env.spec;

        Ok(MultiContractRunner {
            contracts: deployable_contracts,
            revert_decoder,
            known_contracts,
            libs_to_deploy,
            library_addresses,
            library_deployment,
            libraries,
            analysis,
            fuzz_literals,
            invariant_literals,
            enum_bounds,

            tcfg: TestRunnerConfig {
                evm_opts,
                executor_builder,
                evm_env,
                tx_env,
                spec_id,
                hardfork,
                fork_chain_id,
                fork_hardfork: self.fork_hardfork,
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
                fuzz_input: self.fuzz_input,
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
        generated_symbolic_regression: bool,
    ) -> TestFunctionKind {
        if generated_symbolic_regression && !func.name.starts_with("test_regression_") {
            return TestFunctionKind::Unknown;
        }

        TestFunctionKind::classify(
            func.name.as_str(),
            !func.inputs.is_empty(),
            self.symbolic_tests_enabled(contract_id),
        )
    }

    /// Returns the functions of `abi` accepted by `keep`, which is given the contract identifier,
    /// the function and its classification.
    pub(crate) fn test_functions(
        self,
        contract_id: String,
        abi: &JsonAbi,
        mut keep: impl FnMut(&str, &Function, TestFunctionKind) -> bool,
    ) -> impl Iterator<Item = &Function> {
        let generated_symbolic_regression = is_generated_symbolic_regression_contract(abi);
        abi.functions().filter(move |func| {
            let kind = self.test_function_kind(&contract_id, func, generated_symbolic_regression);
            keep(&contract_id, func, kind)
        })
    }

    /// Returns the test functions of `abi` that match `filter`.
    fn matching_test_functions<'b>(
        self,
        filter: &dyn TestFilter,
        id: &ArtifactId,
        abi: &'b JsonAbi,
    ) -> impl Iterator<Item = &'b Function> {
        self.test_functions(id.identifier(), abi, move |contract_id, func, kind| {
            filter.matches_test_function_kind_in_contract(contract_id, func, kind)
        })
    }

    /// Counts the fuzz test functions and runnable invariant campaign anchors of `abi` that
    /// match `filter` in the current network pass.
    pub(crate) fn count_fuzz_engine_targets(
        &self,
        filter: &dyn TestFilter,
        id: &ArtifactId,
        abi: &JsonAbi,
        multi_network: &MultiNetworkConfig,
    ) -> (usize, usize) {
        let contract_name = id.identifier();
        let matches_network_pass = |func: &Function| {
            function_matches_network_pass(
                &multi_network.all_override_networks,
                multi_network.pass_network.as_ref(),
                self.inline_config.network_for(&self.config.profile, &contract_name, &func.name),
            )
        };
        let fuzz = self
            .test_functions(contract_name.clone(), abi, |contract_id, func, kind| {
                matches!(kind, TestFunctionKind::FuzzTest { .. })
                    && filter.matches_test_function_kind_in_contract(contract_id, func, kind)
                    && matches_network_pass(func)
            })
            .count();
        let invariant = count_runnable_invariant_campaign_anchors(
            abi,
            filter,
            InvariantCampaignScope {
                config: self.config,
                inline_config: self.inline_config,
                contract_name: &contract_name,
                all_override_networks: &multi_network.all_override_networks,
                pass_network: multi_network.pass_network.as_ref(),
            },
        );
        (fuzz, invariant)
    }

    pub(crate) fn matches_contract(
        &self,
        filter: &dyn TestFilter,
        id: &ArtifactId,
        abi: &JsonAbi,
    ) -> bool {
        filter.matches_path(&id.source)
            && filter.matches_contract(&id.name)
            && self.matching_test_functions(filter, id, abi).next().is_some()
    }
}

pub(crate) fn is_generated_symbolic_regression_contract(abi: &JsonAbi) -> bool {
    abi.functions().any(|func| func.name == SYMBOLIC_REGRESSION_MARKER && func.inputs.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abi_with_functions(functions: &[&str]) -> JsonAbi {
        let mut abi = JsonAbi::new();
        for function in functions {
            let function = Function::parse(function).unwrap();
            abi.functions.entry(function.name.clone()).or_default().push(function);
        }
        abi
    }

    #[test]
    fn generated_symbolic_regression_detection_uses_marker() {
        let user_suffix_abi = abi_with_functions(&["test_fails()"]);
        assert!(!is_generated_symbolic_regression_contract(&user_suffix_abi));

        let generated_abi =
            abi_with_functions(&[&format!("{SYMBOLIC_REGRESSION_MARKER}()"), "test_fails()"]);
        assert!(is_generated_symbolic_regression_contract(&generated_abi));
    }

    #[test]
    fn create2_deployer_availability_default_is_conservative() {
        let config = Arc::new(Config::default());
        let mut builder = MultiContractRunnerBuilder::new(config, Arc::new(InlineConfig::new()));
        let mut evm_opts = EvmOpts::default();
        assert!(builder.create2_deployer_available(&evm_opts));

        builder.fork = Some(CreateFork {
            enable_caching: false,
            url: "http://localhost:8545".into(),
            evm_opts: evm_opts.clone(),
            resolved: None,
        });
        assert!(!builder.create2_deployer_available(&evm_opts));
        builder.fork = None;

        evm_opts.fork_url = Some("http://localhost:8545".into());
        assert!(!builder.create2_deployer_available(&evm_opts));
        evm_opts.fork_url = None;
        evm_opts.create2_deployer = Address::ZERO;
        assert!(!builder.create2_deployer_available(&evm_opts));
        assert!(
            builder.with_create2_deployer_available(true).create2_deployer_available(&evm_opts)
        );
    }
}
