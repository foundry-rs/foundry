use super::{fuzz::FuzzRunArgs, install, watch::WatchArgs};
use crate::{
    MultiContractRunner, MultiContractRunnerBuilder, brutalizer,
    decode::decode_console_logs,
    gas_report::GasReport,
    multi_runner::{
        FuzzFailureReplayConfig, FuzzMinimizeConfig, FuzzMinimizeEdgeIndices, FuzzMinimizeMode,
        FuzzMinimizeObservation, MultiNetworkConfig, ShowmapConfig, SymbolicArtifactReplayConfig,
        TestFunctionMatcher, is_generated_symbolic_regression_contract,
    },
    mutation::{MutationRunConfig, run_mutation_testing},
    result::{
        SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA, SuiteResult, SymbolicCounterexampleArtifact,
        SymbolicReplayStatus, TestKind, TestKindReport, TestOutcome, TestResult, TestStatus,
    },
    runner::{effective_test_function_kind, inline_config_for},
    symbolic_regression::{
        SymbolicRegression, SymbolicRegressionConfig, attach_symbolic_regressions_to_suites,
        collect_symbolic_artifacts_from_suites, emit_symbolic_regressions,
    },
    traces::{
        CallTraceDecoderBuilder, InternalTraceMode, TraceKind,
        debug::{ContractSources, DebugTraceIdentifier},
        decode_trace_arena, folded_stack_trace,
        identifier::SignaturesIdentifier,
        render_trace_arena_inner, speedscope,
    },
    workspace,
};
use alloy_json_abi::JsonAbi;
use alloy_primitives::U256;
use chrono::Utc;
use clap::{Parser, ValueEnum, ValueHint};
use dialoguer::{Select, console::Term};
use eyre::{Context, OptionExt, Result, bail};
use foundry_cli::{
    opts::{BuildOpts, EvmArgs, GlobalArgs, TracingArgs},
    utils::{self, FoundryPathExt, LoadConfig},
};
use foundry_common::{
    ContractsByArtifact, EmptyTestFilter, TestFilter, TestFunctionExt, TestFunctionKind,
    compile::{ProjectCompiler, compile_abi_project},
    fs, sh_status, sh_warn, shell,
};
use foundry_compilers::{
    Artifact, ArtifactId, ProjectCompileOutput,
    artifacts::{
        BytecodeObject, ConfigurableContractArtifact, Libraries,
        output_selection::ContractOutputSelection,
    },
    compilers::{
        Language,
        multi::{MultiCompiler, MultiCompilerLanguage},
    },
    utils::source_files_iter,
};
use foundry_config::{
    Config, InlineConfig, InvariantDepthMode, InvariantWorkers, figment,
    figment::{
        Metadata, Profile, Provider,
        value::{Dict, Map, Value},
    },
    filter::GlobMatcher,
    fs_permissions::FsAccessPermission,
};
use foundry_debugger::{Debugger, DebuggerLayout};
#[cfg(feature = "monad")]
use foundry_evm::core::evm::MonadEvmNetwork;
#[cfg(feature = "optimism")]
use foundry_evm::core::evm::OpEvmNetwork;
use foundry_evm::{
    core::evm::{
        BlockEnvFor, EthEvmNetwork, FoundryEvmNetwork, SpecFor, TempoEvmNetwork, TxEnvFor,
    },
    executors::{ExecutorBuilder, ShowmapDomain},
    fork::ResolvedFork,
    fuzz::{BaseCounterExample, BasicTxDetails, CounterExample},
    opts::EvmOpts,
    traces::{
        backtrace::BacktraceBuilder, identifier::TraceIdentifiers, prune_trace_depth,
        trace_arena_at_depth,
    },
};
use foundry_evm_networks::NetworkVariant;
use foundry_tui::tui_mode;
use quick_junit::{NonSuccessKind, Report, TestCase, TestCaseStatus, TestSuite};
use rand::Rng;
use regex::Regex;
use revm::{bytecode::opcode::OpCode, context::Transaction};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc::channel},
    time::{Duration, Instant},
};
use tempfile::TempDir;
use yansi::Paint;

mod evm_profile_server;
mod filter;
mod summary;
use filter::RerunFailures;
pub use filter::{FilterArgs, ProjectPathsAwareFilter, RerunFailure};
use summary::{TestSummaryReport, format_invariant_metrics_table};

const DEBUGGER_MATCHING_TESTS_DISPLAY_LIMIT: usize = 12;
const AUTO_FUZZ_FAILURE_DIR: &str = "fuzz";
const AUTO_CORPUS_DIR: &str = "corpus";

// Loads project's figment and merges the build cli arguments into it
foundry_config::merge_impl_figment_convert!(TestArgs, build, evm);

fn validate_showmap_config(showmap: &ShowmapConfig) -> Result<()> {
    for (kind, name) in [("approach", &showmap.approach), ("trial", &showmap.trial)] {
        let path = Path::new(name);
        if name.is_empty()
            || path.is_absolute()
            || path.components().count() != 1
            || name.contains(['/', '\\'])
            || matches!(name.as_str(), "." | "..")
        {
            bail!(
                "invalid showmap {kind} `{name}`: expected a single file-name component without path separators"
            );
        }
    }
    Ok(())
}

/// Compiled runners for every network pass of a `forge fuzz` minimization command.
pub(crate) struct FuzzMinimizeReplaySession {
    filter: ProjectPathsAwareFilter,
    passes: Vec<FuzzMinimizeReplayPass>,
}

type FuzzMinimizeReplay = Box<dyn Fn(&ProjectPathsAwareFilter, FuzzMinimizeConfig) -> Result<()>>;

struct FuzzMinimizeReplayPass {
    target_count: usize,
    replay: FuzzMinimizeReplay,
}

impl FuzzMinimizeReplaySession {
    pub(crate) fn replay(
        &self,
        sequence: Vec<BasicTxDetails>,
        evm_edge_indices: FuzzMinimizeEdgeIndices,
        mode: FuzzMinimizeMode,
    ) -> Result<Vec<FuzzMinimizeObservation>> {
        let observations = Arc::new(Mutex::new(Vec::new()));
        let fuzz_minimize = FuzzMinimizeConfig {
            input: sequence.into(),
            mode,
            evm_edge_indices,
            observations: observations.clone(),
        };
        for pass in self.passes.iter().filter(|pass| pass.target_count > 0) {
            (pass.replay)(&self.filter, fuzz_minimize.clone())?;
        }
        let observations = observations
            .lock()
            .map_err(|_| eyre::eyre!("minimize observations lock poisoned"))?
            .clone();
        if observations.is_empty() {
            bail!("fuzz minimization replay produced no observation for the matched test");
        }
        Ok(observations)
    }
}

fn fuzz_minimize_pass<FEN: FoundryEvmNetwork>(
    runner: MultiContractRunner<FEN>,
    filter: &ProjectPathsAwareFilter,
) -> FuzzMinimizeReplayPass {
    let target_count = count_fuzz_minimize_targets(&runner, filter);
    let replay = move |filter: &ProjectPathsAwareFilter, fuzz_minimize| -> Result<()> {
        let mut runner = runner.clone();
        runner.tcfg.fuzz_minimize = Some(fuzz_minimize);
        for (suite, suite_result) in runner.test_collect(filter)? {
            for (test, test_result) in suite_result.test_results {
                if test_result.status == TestStatus::Failure {
                    bail!(
                        "fuzz minimization replay failed for {suite}::{test}: {}",
                        test_result.reason.as_deref().unwrap_or("unknown error")
                    );
                }
            }
        }
        Ok(())
    };
    FuzzMinimizeReplayPass { target_count, replay: Box::new(replay) }
}

fn count_fuzz_minimize_targets<FEN: FoundryEvmNetwork>(
    runner: &MultiContractRunner<FEN>,
    filter: &dyn TestFilter,
) -> usize {
    let matcher = runner.test_function_matcher();
    runner
        .matching_contracts(filter)
        .map(|(id, contract)| {
            let (fuzz, invariant) = matcher.count_fuzz_engine_targets(
                filter,
                id,
                &contract.abi,
                &runner.tcfg.multi_network,
            );
            fuzz + invariant
        })
        .sum()
}

#[derive(Clone, Copy)]
enum NetworkDispatchKind {
    Tempo,
    #[cfg(feature = "monad")]
    Monad,
    #[cfg(feature = "optimism")]
    Optimism,
    Eth,
}

const fn network_dispatch_kind(evm_opts: &EvmOpts) -> NetworkDispatchKind {
    if evm_opts.networks.is_tempo() {
        return NetworkDispatchKind::Tempo;
    }
    #[cfg(feature = "monad")]
    if evm_opts.networks.is_monad() {
        return NetworkDispatchKind::Monad;
    }
    #[cfg(feature = "optimism")]
    if evm_opts.networks.is_optimism() {
        return NetworkDispatchKind::Optimism;
    }
    NetworkDispatchKind::Eth
}

/// Evaluates `$body` with `$fen` bound to the concrete network type selected by `$evm_opts`.
macro_rules! dispatch_network {
    ($evm_opts:expr, | $fen:ident | $body:expr) => {
        match network_dispatch_kind($evm_opts) {
            NetworkDispatchKind::Tempo => {
                type $fen = TempoEvmNetwork;
                $body
            }
            #[cfg(feature = "monad")]
            NetworkDispatchKind::Monad => {
                type $fen = MonadEvmNetwork;
                $body
            }
            #[cfg(feature = "optimism")]
            NetworkDispatchKind::Optimism => {
                type $fen = OpEvmNetwork;
                $body
            }
            NetworkDispatchKind::Eth => {
                type $fen = EthEvmNetwork;
                $body
            }
        }
    };
}

/// Output format for EVM execution profiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum EvmProfileFormat {
    /// Speedscope format, opens in speedscope.app.
    #[default]
    Speedscope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TraceOutputKind {
    Flamegraph,
    Flamechart,
    EvmProfile(EvmProfileFormat),
}

impl TraceOutputKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Flamegraph => "flamegraph",
            Self::Flamechart => "flamechart",
            Self::EvmProfile(_) => "EVM profile",
        }
    }
}

/// CLI mirror of `foundry_evm::executors::ShowmapDomain`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum ShowmapDomainArg {
    #[default]
    Evm,
    Sancov,
    Both,
}

impl From<ShowmapDomainArg> for ShowmapDomain {
    fn from(d: ShowmapDomainArg) -> Self {
        match d {
            ShowmapDomainArg::Evm => Self::Evm,
            ShowmapDomainArg::Sancov => Self::Sancov,
            ShowmapDomainArg::Both => Self::Both,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TestExecutionOptions {
    pub(crate) coverage: bool,
    pub(crate) decode_internal: InternalTraceMode,
    pub(crate) multi_network: MultiNetworkConfig,
    pub(crate) fuzz_input: Option<FuzzFailureReplayConfig>,
    pub(crate) replay_symbolic_artifact: Option<SymbolicArtifactReplayConfig>,
    pub(crate) inline_config: Arc<InlineConfig>,
    pub(crate) selected_sources: BTreeSet<PathBuf>,
}

impl TestExecutionOptions {
    pub(crate) fn default_run(inline_config: Arc<InlineConfig>) -> Self {
        Self {
            coverage: false,
            decode_internal: InternalTraceMode::None,
            multi_network: MultiNetworkConfig::default(),
            fuzz_input: None,
            replay_symbolic_artifact: None,
            inline_config,
            selected_sources: BTreeSet::new(),
        }
    }

    pub(crate) fn coverage(inline_config: Arc<InlineConfig>) -> Self {
        Self { coverage: true, ..Self::default_run(inline_config) }
    }
}

/// Config and EVM options for one network pass of a multi-network run.
struct NetworkPass {
    config: Config,
    evm_opts: EvmOpts,
    multi_network: MultiNetworkConfig,
}

/// Splits a run into the default network pass and one override pass per annotated network.
fn network_passes(
    config: Config,
    evm_opts: EvmOpts,
    override_networks: &[NetworkVariant],
) -> (NetworkPass, Vec<NetworkPass>) {
    let multi_network = |pass_network| MultiNetworkConfig {
        all_override_networks: override_networks.to_vec(),
        pass_network,
    };
    let override_passes = override_networks
        .iter()
        .map(|&network| {
            let mut evm_opts = evm_opts.clone();
            evm_opts.set_explicit_network(network);
            let mut config = config.clone();
            config.networks = evm_opts.networks;
            NetworkPass { config, evm_opts, multi_network: multi_network(Some(network)) }
        })
        .collect();
    (NetworkPass { config, evm_opts, multi_network: multi_network(None) }, override_passes)
}

struct CompiledTestProject {
    project_root: PathBuf,
    config: Config,
    evm_opts: EvmOpts,
    output: ProjectCompileOutput,
    filter: ProjectPathsAwareFilter,
    inline_config: Arc<InlineConfig>,
    replay_symbolic_artifact: Option<SymbolicArtifactReplayConfig>,
    selected_sources: BTreeSet<PathBuf>,
    /// Keeps the brutalized copy of the project alive while its tests run.
    _brutalized_workspace: Option<TempDir>,
}

/// Shared campaign arguments for `forge fuzz run`.
#[derive(Clone, Debug, Parser)]
#[command(next_help_heading = "Campaign options")]
pub struct CampaignArgs {
    /// Number of runs to execute for each fuzz or invariant campaign.
    #[arg(long, value_name = "RUNS")]
    pub runs: Option<u64>,

    /// Campaign-global timeout in seconds.
    #[arg(long, value_name = "TIMEOUT")]
    pub timeout: Option<u32>,

    /// Set seed used to generate randomness during fuzz runs.
    #[arg(long)]
    pub seed: Option<U256>,

    /// Number of calls executed to try to break invariants in one run.
    #[arg(long, value_name = "DEPTH")]
    pub depth: Option<u32>,

    /// Minimum sampled invariant depth when `--depth-mode random` is active.
    #[arg(long, value_name = "DEPTH")]
    pub min_depth: Option<u32>,

    /// How invariant run depth is selected.
    #[arg(long, value_name = "fixed|random")]
    pub depth_mode: Option<InvariantDepthMode>,

    /// Number of workers to use for invariant test campaigns, or `auto` to derive from `--jobs`.
    #[arg(long, value_name = "WORKERS")]
    pub workers: Option<InvariantWorkers>,

    /// Directory for fuzz and invariant corpus persistence.
    #[arg(long, value_name = "PATH", value_hint = ValueHint::DirPath)]
    pub corpus_dir: Option<PathBuf>,

    /// Percent of calldata generated from the dictionary.
    #[arg(long, value_name = "PERCENT")]
    pub dictionary_weight: Option<u32>,

    /// Maximum dictionary addresses, or `max`.
    #[arg(long, value_name = "N|max")]
    pub dictionary_addresses: Option<String>,

    /// Maximum dictionary values, or `max`.
    #[arg(long, value_name = "N|max")]
    pub dictionary_values: Option<String>,

    /// Maximum dictionary literals, or `max`.
    #[arg(long, value_name = "N|max")]
    pub dictionary_literals: Option<String>,

    /// Percent chance that coverage-guided fuzzing generates fresh input instead of mutating
    /// corpus input.
    #[arg(long, value_name = "PERCENT")]
    pub corpus_random_sequence_weight: Option<u32>,

    /// Percent chance that fuzzed payable calls carry non-zero msg.value.
    #[arg(long, value_name = "PERCENT")]
    pub payable_value_weight: Option<u32>,

    /// Corpus mutation weight for splice.
    #[arg(long, value_name = "WEIGHT")]
    pub mutation_weight_splice: Option<u32>,

    /// Corpus mutation weight for repeat.
    #[arg(long, value_name = "WEIGHT")]
    pub mutation_weight_repeat: Option<u32>,

    /// Corpus mutation weight for interleave.
    #[arg(long, value_name = "WEIGHT")]
    pub mutation_weight_interleave: Option<u32>,

    /// Corpus mutation weight for prefix replacement.
    #[arg(long, value_name = "WEIGHT")]
    pub mutation_weight_prefix: Option<u32>,

    /// Corpus mutation weight for suffix replacement.
    #[arg(long, value_name = "WEIGHT")]
    pub mutation_weight_suffix: Option<u32>,

    /// Corpus mutation weight for ABI argument mutation.
    #[arg(long, value_name = "WEIGHT")]
    pub mutation_weight_abi: Option<u32>,

    /// Corpus mutation weight for comparison-operand mutation.
    #[arg(long, value_name = "WEIGHT")]
    pub mutation_weight_cmp: Option<u32>,

    /// Directory for fuzz branch frontier artifacts.
    #[arg(long, value_name = "PATH", value_hint = ValueHint::DirPath)]
    pub frontier_dir: Option<PathBuf>,

    /// Maximum number of fuzz branch frontier records to write per test.
    #[arg(long, value_name = "COUNT")]
    pub frontier_limit: Option<usize>,
}

/// CLI arguments for `forge test`.
#[derive(Clone, Debug, Default, Parser)]
#[command(next_help_heading = "Test options")]
pub struct TestArgs {
    /// Internal mode used by `forge fuzz` to run only fuzz and invariant tests.
    #[arg(skip)]
    fuzz_only: bool,

    /// Internal mode used by `forge fuzz run` to default the fuzz corpus dir.
    #[arg(skip)]
    auto_fuzz_corpus: bool,

    /// Internal showmap/replay override used by `forge fuzz replay`.
    #[arg(skip)]
    pub(crate) showmap_override: Option<ShowmapConfig>,

    /// Internal mode used by `forge fuzz replay` to replay persisted fuzz failures.
    #[arg(skip)]
    pub(crate) fuzz_failure_replay: bool,

    /// Internal override used by `forge fuzz run --runs` for invariant campaigns.
    #[arg(skip)]
    pub(crate) invariant_runs_override: Option<u64>,

    /// Internal override used by `forge fuzz run --timeout` for invariant campaigns.
    #[arg(skip)]
    pub(crate) invariant_timeout_override: Option<u32>,

    // Include global options for users of this struct.
    #[command(flatten)]
    pub global: GlobalArgs,

    /// The contract file you want to test, it's a shortcut for --match-path.
    #[arg(value_hint = ValueHint::FilePath)]
    pub path: Option<GlobMatcher>,

    /// Run a single test in the debugger.
    ///
    /// The matching test will be opened in the debugger regardless of the outcome of the test.
    ///
    /// If the matching test is a fuzz test, then it will open the debugger on the first failure
    /// case. If the fuzz test does not fail, it will open the debugger on the last fuzz case.
    #[arg(long, conflicts_with_all = ["flamegraph", "flamechart", "evm_profile", "decode_internal", "rerun"])]
    debug: bool,

    /// Debugger layout to use.
    #[arg(long = "debug-layout", requires = "debug", value_enum)]
    debug_layout: Option<DebuggerLayout>,

    /// Generate a flamegraph for a single test. Implies `--decode-internal`.
    ///
    /// A flame graph is used to visualize which functions or operations within the smart contract
    /// are consuming the most gas overall in a sorted manner.
    #[arg(
        long,
        group = "trace_output",
        conflicts_with_all = ["flamechart", "evm_profile", "json", "junit", "list"]
    )]
    flamegraph: bool,

    /// Generate a flamechart for a single test. Implies `--decode-internal`.
    ///
    /// A flame chart shows the gas usage over time, illustrating when each function is
    /// called (execution order) and how much gas it consumes at each point in the timeline.
    #[arg(
        long,
        group = "trace_output",
        conflicts_with_all = ["flamegraph", "evm_profile", "json", "junit", "list"]
    )]
    flamechart: bool,

    /// Generate an execution profile for a single test.
    ///
    /// Creates a profile where each EVM call is recorded with gas consumption.
    /// Opens the profile in speedscope.app unless `--no-open` is passed.
    /// Implies `--decode-internal`.
    #[arg(
        long,
        value_name = "FORMAT",
        num_args = 0..=1,
        default_missing_value = "speedscope",
        value_enum,
        group = "trace_output",
        conflicts_with_all = ["flamegraph", "flamechart", "json", "junit", "list"]
    )]
    evm_profile: Option<EvmProfileFormat>,

    /// Don't open the generated profile, flamegraph, or flamechart.
    ///
    /// The profile is saved to disk without starting the local viewer server.
    #[arg(long, requires = "trace_output")]
    no_open: bool,

    #[command(flatten)]
    tracing: TracingArgs,

    /// Dumps all debugger steps to file.
    #[arg(
        long,
        requires = "debug",
        value_hint = ValueHint::FilePath,
        value_name = "PATH"
    )]
    dump: Option<PathBuf>,

    /// Print a gas report.
    #[arg(long, env = "FORGE_GAS_REPORT")]
    gas_report: bool,

    /// Check gas snapshots against previous runs.
    #[arg(long, env = "FORGE_SNAPSHOT_CHECK")]
    gas_snapshot_check: Option<bool>,

    /// Enable/disable recording of gas snapshot results.
    #[arg(long, env = "FORGE_SNAPSHOT_EMIT")]
    gas_snapshot_emit: Option<bool>,

    /// Exit with code 0 even if a test fails.
    #[arg(long, env = "FORGE_ALLOW_FAILURE")]
    allow_failure: bool,

    /// Suppress successful test traces and show only traces for failures.
    #[arg(long, short, env = "FORGE_SUPPRESS_SUCCESSFUL_TRACES", help_heading = "Trace options")]
    suppress_successful_traces: bool,

    /// Write test results as JSON to the specified file.
    #[arg(
        long,
        value_name = "PATH",
        value_hint = ValueHint::FilePath,
        conflicts_with = "list",
        help_heading = "Display options"
    )]
    json_file: Option<PathBuf>,

    /// Output test results as JUnit XML report.
    #[arg(long, conflicts_with_all = ["quiet", "json", "gas_report", "summary", "list", "show_progress"], help_heading = "Display options")]
    pub junit: bool,

    /// Stop running tests after the first failure.
    #[arg(long)]
    pub fail_fast: bool,

    /// The Etherscan (or equivalent) API key.
    #[arg(long, env = "ETHERSCAN_API_KEY", value_name = "KEY")]
    etherscan_api_key: Option<String>,

    /// List tests instead of running them.
    #[arg(long, short, conflicts_with_all = ["show_progress", "decode_internal", "summary"], help_heading = "Display options")]
    list: bool,

    /// Set seed used to generate randomness during your fuzz runs.
    #[arg(long)]
    pub fuzz_seed: Option<U256>,

    #[arg(long, env = "FOUNDRY_FUZZ_RUNS", value_name = "RUNS")]
    pub fuzz_runs: Option<u64>,

    /// Number of workers to use for invariant test campaigns, or `auto` to derive from `--jobs`.
    #[arg(long, env = "FOUNDRY_INVARIANT_WORKERS", value_name = "WORKERS")]
    pub invariant_workers: Option<InvariantWorkers>,

    /// Run only the fuzz case at the given 1-based run index.
    #[arg(long, env = "FOUNDRY_FUZZ_RUN", value_name = "RUN")]
    pub fuzz_run: Option<u32>,

    /// Run the fuzz case from the given worker. Requires `--fuzz-run`.
    #[arg(long, env = "FOUNDRY_FUZZ_WORKER", value_name = "WORKER", requires = "fuzz_run")]
    pub fuzz_worker: Option<u32>,

    /// Timeout for each fuzz run in seconds.
    #[arg(long, env = "FOUNDRY_FUZZ_TIMEOUT", value_name = "TIMEOUT")]
    pub fuzz_timeout: Option<u64>,

    /// Percent of fuzz calldata generated from the dictionary.
    #[arg(long, env = "FOUNDRY_FUZZ_DICTIONARY_WEIGHT", value_name = "PERCENT")]
    pub fuzz_dictionary_weight: Option<u32>,

    /// Maximum fuzz dictionary addresses, or `max`.
    #[arg(long, env = "FOUNDRY_FUZZ_MAX_FUZZ_DICTIONARY_ADDRESSES", value_name = "N|max")]
    pub fuzz_dictionary_addresses: Option<String>,

    /// Maximum fuzz dictionary values, or `max`.
    #[arg(long, env = "FOUNDRY_FUZZ_MAX_FUZZ_DICTIONARY_VALUES", value_name = "N|max")]
    pub fuzz_dictionary_values: Option<String>,

    /// Maximum fuzz dictionary literals, or `max`.
    #[arg(long, env = "FOUNDRY_FUZZ_MAX_FUZZ_DICTIONARY_LITERALS", value_name = "N|max")]
    pub fuzz_dictionary_literals: Option<String>,

    /// Percent chance that coverage-guided fuzzing generates fresh input instead of mutating
    /// corpus input.
    #[arg(long, env = "FOUNDRY_FUZZ_CORPUS_RANDOM_SEQUENCE_WEIGHT", value_name = "PERCENT")]
    pub fuzz_corpus_random_sequence_weight: Option<u32>,

    /// Directory for fuzz corpus persistence.
    #[arg(long, env = "FOUNDRY_FUZZ_CORPUS_DIR", value_name = "PATH", value_hint = ValueHint::DirPath)]
    pub fuzz_corpus_dir: Option<PathBuf>,

    /// Directory for fuzz branch frontier artifacts.
    #[arg(long, env = "FOUNDRY_FUZZ_FRONTIER_DIR", value_name = "PATH", value_hint = ValueHint::DirPath)]
    pub fuzz_frontier_dir: Option<PathBuf>,

    /// Maximum number of fuzz branch frontier records to write per test.
    #[arg(long, env = "FOUNDRY_FUZZ_FRONTIER_LIMIT", value_name = "COUNT")]
    pub fuzz_frontier_limit: Option<usize>,

    /// Percent chance that fuzzed payable calls carry non-zero msg.value.
    #[arg(long, env = "FOUNDRY_FUZZ_PAYABLE_VALUE_WEIGHT", value_name = "PERCENT")]
    pub fuzz_payable_value_weight: Option<u32>,

    /// Corpus mutation weight for splice.
    #[arg(long, env = "FOUNDRY_FUZZ_MUTATION_WEIGHT_SPLICE", value_name = "WEIGHT")]
    pub fuzz_mutation_weight_splice: Option<u32>,

    /// Corpus mutation weight for repeat.
    #[arg(long, env = "FOUNDRY_FUZZ_MUTATION_WEIGHT_REPEAT", value_name = "WEIGHT")]
    pub fuzz_mutation_weight_repeat: Option<u32>,

    /// Corpus mutation weight for interleave.
    #[arg(long, env = "FOUNDRY_FUZZ_MUTATION_WEIGHT_INTERLEAVE", value_name = "WEIGHT")]
    pub fuzz_mutation_weight_interleave: Option<u32>,

    /// Corpus mutation weight for prefix replacement.
    #[arg(long, env = "FOUNDRY_FUZZ_MUTATION_WEIGHT_PREFIX", value_name = "WEIGHT")]
    pub fuzz_mutation_weight_prefix: Option<u32>,

    /// Corpus mutation weight for suffix replacement.
    #[arg(long, env = "FOUNDRY_FUZZ_MUTATION_WEIGHT_SUFFIX", value_name = "WEIGHT")]
    pub fuzz_mutation_weight_suffix: Option<u32>,

    /// Corpus mutation weight for ABI argument mutation.
    #[arg(long, env = "FOUNDRY_FUZZ_MUTATION_WEIGHT_ABI", value_name = "WEIGHT")]
    pub fuzz_mutation_weight_abi: Option<u32>,

    /// Corpus mutation weight for comparison-operand mutation.
    #[arg(long, env = "FOUNDRY_FUZZ_MUTATION_WEIGHT_CMP", value_name = "WEIGHT")]
    pub fuzz_mutation_weight_cmp: Option<u32>,

    /// File to rerun fuzz failures from.
    #[arg(
        long,
        value_name = "PATH",
        value_hint = ValueHint::FilePath,
        conflicts_with_all = ["fuzz_run", "list"]
    )]
    pub fuzz_input_file: Option<PathBuf>,

    /// Number of calls executed to try to break invariants in one run.
    #[arg(long, env = "FOUNDRY_INVARIANT_DEPTH", value_name = "DEPTH")]
    pub invariant_depth: Option<u32>,

    /// Minimum sampled invariant depth when `--invariant-depth-mode random` is active.
    #[arg(long, env = "FOUNDRY_INVARIANT_MIN_DEPTH", value_name = "DEPTH")]
    pub invariant_min_depth: Option<u32>,

    /// How invariant run depth is selected.
    #[arg(long, env = "FOUNDRY_INVARIANT_DEPTH_MODE", value_name = "fixed|random")]
    pub invariant_depth_mode: Option<InvariantDepthMode>,

    /// Percent of invariant calldata/senders generated from the dictionary.
    #[arg(long, env = "FOUNDRY_INVARIANT_DICTIONARY_WEIGHT", value_name = "PERCENT")]
    pub invariant_dictionary_weight: Option<u32>,

    /// Maximum invariant dictionary addresses, or `max`.
    #[arg(long, env = "FOUNDRY_INVARIANT_MAX_FUZZ_DICTIONARY_ADDRESSES", value_name = "N|max")]
    pub invariant_dictionary_addresses: Option<String>,

    /// Maximum invariant dictionary values, or `max`.
    #[arg(long, env = "FOUNDRY_INVARIANT_MAX_FUZZ_DICTIONARY_VALUES", value_name = "N|max")]
    pub invariant_dictionary_values: Option<String>,

    /// Maximum invariant dictionary literals, or `max`.
    #[arg(long, env = "FOUNDRY_INVARIANT_MAX_FUZZ_DICTIONARY_LITERALS", value_name = "N|max")]
    pub invariant_dictionary_literals: Option<String>,

    /// Percent chance that coverage-guided invariant fuzzing injects fresh calls while extending
    /// corpus sequences.
    #[arg(long, env = "FOUNDRY_INVARIANT_CORPUS_RANDOM_SEQUENCE_WEIGHT", value_name = "PERCENT")]
    pub invariant_corpus_random_sequence_weight: Option<u32>,

    /// Directory for invariant corpus persistence.
    #[arg(long, env = "FOUNDRY_INVARIANT_CORPUS_DIR", value_name = "PATH", value_hint = ValueHint::DirPath)]
    pub invariant_corpus_dir: Option<PathBuf>,

    /// Directory inherited from `forge fuzz run --frontier-dir` for invariant frontiers.
    #[arg(skip)]
    invariant_frontier_dir: Option<PathBuf>,

    /// Frontier limit inherited from `forge fuzz run --frontier-limit` for invariant frontiers.
    #[arg(skip)]
    invariant_frontier_limit: Option<usize>,

    /// Percent chance that fuzzed payable invariant calls carry non-zero msg.value.
    #[arg(long, env = "FOUNDRY_INVARIANT_PAYABLE_VALUE_WEIGHT", value_name = "PERCENT")]
    pub invariant_payable_value_weight: Option<u32>,

    /// Corpus mutation weight for splice.
    #[arg(long, env = "FOUNDRY_INVARIANT_MUTATION_WEIGHT_SPLICE", value_name = "WEIGHT")]
    pub invariant_mutation_weight_splice: Option<u32>,

    /// Corpus mutation weight for repeat.
    #[arg(long, env = "FOUNDRY_INVARIANT_MUTATION_WEIGHT_REPEAT", value_name = "WEIGHT")]
    pub invariant_mutation_weight_repeat: Option<u32>,

    /// Corpus mutation weight for interleave.
    #[arg(long, env = "FOUNDRY_INVARIANT_MUTATION_WEIGHT_INTERLEAVE", value_name = "WEIGHT")]
    pub invariant_mutation_weight_interleave: Option<u32>,

    /// Corpus mutation weight for prefix replacement.
    #[arg(long, env = "FOUNDRY_INVARIANT_MUTATION_WEIGHT_PREFIX", value_name = "WEIGHT")]
    pub invariant_mutation_weight_prefix: Option<u32>,

    /// Corpus mutation weight for suffix replacement.
    #[arg(long, env = "FOUNDRY_INVARIANT_MUTATION_WEIGHT_SUFFIX", value_name = "WEIGHT")]
    pub invariant_mutation_weight_suffix: Option<u32>,

    /// Corpus mutation weight for ABI argument mutation.
    #[arg(long, env = "FOUNDRY_INVARIANT_MUTATION_WEIGHT_ABI", value_name = "WEIGHT")]
    pub invariant_mutation_weight_abi: Option<u32>,

    /// Corpus mutation weight for comparison-operand mutation.
    #[arg(long, env = "FOUNDRY_INVARIANT_MUTATION_WEIGHT_CMP", value_name = "WEIGHT")]
    pub invariant_mutation_weight_cmp: Option<u32>,

    /// Run symbolic check*/prove*/invariant*/statefulFuzz* tests.
    #[arg(long, env = "FOUNDRY_SYMBOLIC")]
    pub symbolic: bool,

    /// Replay a durable symbolic counterexample artifact emitted by `forge test --symbolic`.
    #[arg(
        long,
        value_name = "PATH",
        value_hint = ValueHint::FilePath,
        conflicts_with_all = [
            "debug",
            "flamegraph",
            "flamechart",
            "rerun",
            "fuzz_input_file",
            "showmap_out",
            "path",
            "test_pattern",
            "test_pattern_inverse",
            "contract_pattern",
            "contract_pattern_inverse",
            "path_pattern",
            "no-match-path",
        ],
    )]
    pub replay_symbolic_artifact: Option<PathBuf>,

    /// Emit Solidity regression tests for confirmed symbolic counterexamples.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_EMIT_REGRESSION")]
    pub emit_regression: bool,

    /// File or directory for generated symbolic regression tests.
    #[arg(
        long,
        env = "FOUNDRY_SYMBOLIC_REGRESSION_OUT",
        value_name = "PATH",
        value_hint = ValueHint::AnyPath,
        requires = "emit_regression"
    )]
    pub regression_out: Option<PathBuf>,

    /// Overwrite existing generated symbolic regression tests.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_REGRESSION_OVERWRITE", requires = "emit_regression")]
    pub regression_overwrite: bool,

    /// Run fuzz tests symbolically and persist non-failing concrete inputs to the fuzz corpus.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_SEED_CORPUS")]
    pub symbolic_seed_corpus: bool,

    /// Run fuzz tests symbolically using existing fuzz corpus entries as path-priority hints.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_USE_FUZZ_CORPUS")]
    pub symbolic_use_fuzz_corpus: bool,

    /// Maximum number of fuzz corpus entries to import for one symbolic test.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_CORPUS_SEED_LIMIT", value_name = "COUNT")]
    pub symbolic_corpus_seed_limit: Option<usize>,

    /// Run targeted symbolic solving from existing fuzz branch frontier artifacts.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_USE_FUZZ_FRONTIERS")]
    pub symbolic_use_fuzz_frontiers: bool,

    /// Maximum number of fuzz branch frontiers to try for one symbolic test.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_FRONTIER_LIMIT", value_name = "COUNT")]
    pub symbolic_frontier_limit: Option<usize>,

    /// Comma-separated fuzz branch frontier artifact IDs to try.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_FRONTIER_IDS", value_name = "IDS", value_delimiter = ',')]
    pub symbolic_frontier_ids: Option<Vec<u64>>,

    /// Comma-separated fuzz branch frontier comparison PCs to try.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_FRONTIER_PCS", value_name = "PCS", value_delimiter = ',')]
    pub symbolic_frontier_pcs: Option<Vec<usize>>,

    /// Comma-separated fuzz branch frontier calldata selectors to try.
    #[arg(
        long,
        env = "FOUNDRY_SYMBOLIC_FRONTIER_SELECTORS",
        value_name = "SELECTORS",
        value_delimiter = ','
    )]
    pub symbolic_frontier_selectors: Option<Vec<String>>,

    /// Solver executable used for symbolic tests.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_SOLVER", value_name = "PATH_OR_NAME")]
    pub symbolic_solver: Option<String>,

    /// Exact solver command used for symbolic tests.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_SOLVER_COMMAND", value_name = "COMMAND")]
    pub symbolic_solver_command: Option<String>,

    /// Comma-separated SMT solver names or commands to race in parallel for symbolic tests.
    #[arg(
        long,
        env = "FOUNDRY_SYMBOLIC_SOLVER_PORTFOLIO",
        value_delimiter = ',',
        value_name = "SOLVER_OR_COMMAND,..."
    )]
    pub symbolic_solver_portfolio: Option<Vec<String>>,

    /// SMT solver timeout in seconds; also bounds symbolic invariant exploration.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_TIMEOUT", value_name = "SECONDS")]
    pub symbolic_timeout: Option<u32>,

    /// Halmos-compatible symbolic loop bound.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_LOOP", value_name = "N")]
    pub symbolic_loop: Option<u32>,

    /// Halmos-compatible symbolic execution depth alias.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_DEPTH", value_name = "N")]
    pub symbolic_depth: Option<u32>,

    /// Halmos-compatible symbolic path width alias.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_WIDTH", value_name = "N")]
    pub symbolic_width: Option<u32>,

    /// Maximum number of opcodes executed along a symbolic path.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_MAX_DEPTH", value_name = "N")]
    pub symbolic_max_depth: Option<u32>,

    /// Maximum number of symbolic paths to explore per test.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_MAX_PATHS", value_name = "N")]
    pub symbolic_max_paths: Option<u32>,

    /// Maximum number of calls in a bounded symbolic invariant sequence.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_INVARIANT_DEPTH", value_name = "N")]
    pub symbolic_invariant_depth: Option<u32>,

    /// Maximum number of solver queries per symbolic test.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_MAX_SOLVER_QUERIES", value_name = "N")]
    pub symbolic_max_solver_queries: Option<u32>,

    /// Default bounded length for symbolic dynamic ABI inputs.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_DEFAULT_DYNAMIC_LENGTH", value_name = "N")]
    pub symbolic_default_dynamic_length: Option<u32>,

    /// Maximum permitted bounded length for symbolic dynamic ABI inputs.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_MAX_DYNAMIC_LENGTH", value_name = "N")]
    pub symbolic_max_dynamic_length: Option<u32>,

    /// Per-dynamic-input symbolic lengths, applied in ABI traversal order.
    #[arg(
        long,
        env = "FOUNDRY_SYMBOLIC_ARRAY_LENGTHS",
        value_delimiter = ',',
        value_name = "N,..."
    )]
    pub symbolic_array_lengths: Option<Vec<u32>>,

    /// Maximum symbolic calldata size in bytes.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_MAX_CALLDATA_BYTES", value_name = "N")]
    pub symbolic_max_calldata_bytes: Option<u32>,

    /// Expand symbolic external call targets over known deployed contracts.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_CALL_TARGETS")]
    pub symbolic_call_targets: bool,

    /// Dump SMT-LIB queries issued by symbolic tests.
    #[arg(long, env = "FOUNDRY_SYMBOLIC_DUMP_SMT")]
    pub symbolic_dump_smt: bool,

    /// Symbolic storage modelling mode.
    #[arg(
        long,
        env = "FOUNDRY_SYMBOLIC_STORAGE_LAYOUT",
        value_name = "solidity|generic",
        value_parser = ["solidity", "generic"]
    )]
    pub symbolic_storage_layout: Option<String>,

    /// Show test execution progress.
    #[arg(long, conflicts_with_all = ["quiet", "json"], help_heading = "Display options")]
    pub show_progress: bool,

    /// Re-run recorded test failures from last run.
    /// If no failure recorded then regular test run is performed.
    #[arg(long)]
    pub rerun: bool,

    /// Print the given opcodes in trace output, with their gas
    /// cost and the storage slot and value, if available.
    ///
    /// Accepts a comma-separated list of opcode names, e.g.
    /// `--opcodes SLOAD,MLOAD,SSTORE`. Names are in uppercase.
    /// Requires `-vvvvv` to render.
    #[arg(long, value_parser = parse_opcode, value_delimiter(','), conflicts_with_all = ["json", "junit", "list", "debug"])]
    pub opcodes: Vec<OpCode>,

    /// Print test summary table.
    #[arg(long, help_heading = "Display options")]
    pub summary: bool,

    /// Print detailed test summary table.
    #[arg(long, help_heading = "Display options", requires = "summary")]
    pub detailed: bool,

    /// Replay the persisted corpus and emit AFL-`afl-showmap`-style coverage
    /// files at the given output directory. Disables the regular fuzz/invariant
    /// campaign and skips unit tests.
    #[arg(
        long,
        value_name = "DIR",
        value_hint = ValueHint::DirPath,
        help_heading = "Showmap replay",
        conflicts_with_all = ["debug", "flamegraph", "flamechart", "evm_profile", "rerun", "fuzz_input_file", "gas_report"],
    )]
    pub showmap_out: Option<PathBuf>,

    /// Emit one showmap file per corpus entry (default: one aggregated file per test).
    #[arg(long, help_heading = "Showmap replay", requires = "showmap_out")]
    pub showmap_per_input: bool,

    /// Coverage domain(s) to dump.
    #[arg(
        long,
        value_enum,
        default_value_t = ShowmapDomainArg::Evm,
        help_heading = "Showmap replay",
        requires = "showmap_out",
    )]
    pub showmap_domain: ShowmapDomainArg,

    /// Approach name (used as a subdirectory of `--showmap-out`).
    #[arg(
        long,
        default_value = "replay",
        help_heading = "Showmap replay",
        requires = "showmap_out"
    )]
    pub showmap_approach: String,

    /// Trial identifier embedded in each showmap filename. Defaults to a unique
    /// `trial-<unix_nanos>` so reruns don't overwrite previous trials.
    #[arg(long, help_heading = "Showmap replay", requires = "showmap_out")]
    pub showmap_trial: Option<String>,

    /// Override the corpus directory to replay (defaults to the per-test
    /// `corpus_dir` resolved from config).
    #[arg(
        long,
        value_name = "PATH",
        value_hint = ValueHint::DirPath,
        help_heading = "Showmap replay",
        requires = "showmap_out",
    )]
    pub showmap_corpus_dir: Option<PathBuf>,

    #[command(flatten)]
    filter: FilterArgs,

    #[command(flatten)]
    evm: EvmArgs,

    #[command(flatten)]
    pub build: BuildOpts,

    #[command(flatten)]
    pub watch: WatchArgs,

    /// Enable mutation testing.
    /// If passed with file paths, only those files will be tested.
    #[arg(long, num_args(0..), value_name = "PATH")]
    pub mutate: Option<Vec<PathBuf>>,

    /// Specify which files to mutate with glob pattern matching.
    ///
    /// Mutually exclusive with passing explicit paths to `--mutate`; either
    /// supply paths to `--mutate` or use this glob filter, not both.
    #[arg(long, value_name = "PATTERN", requires = "mutate", conflicts_with = "mutate_contract")]
    pub mutate_path: Option<GlobMatcher>,

    /// Only mutate contracts whose name matches the specified regex pattern.
    ///
    /// Mutually exclusive with `--mutate-path`.
    #[arg(long, value_name = "REGEX", requires = "mutate")]
    pub mutate_contract: Option<regex::Regex>,

    /// Number of parallel workers for mutation testing.
    /// Defaults to the number of CPU cores.
    #[arg(long, value_name = "JOBS", requires = "mutate")]
    pub mutation_jobs: Option<usize>,

    /// Best-effort per-mutant wall-clock timeout in seconds. Mutants that
    /// exceed it are recorded as "timed out" and cleanup continues in the
    /// background with bounded pending workers.
    ///
    /// Analogous to `--invariant-timeout` for invariant campaigns.
    #[arg(long, value_name = "TIMEOUT", requires = "mutate")]
    pub mutation_timeout: Option<u32>,

    /// Override optimizer runs for mutation testing compile-and-test runs.
    #[arg(long, value_name = "RUNS", requires = "mutate")]
    pub mutation_optimizer_runs: Option<u32>,

    /// Override via-ir for mutation testing compile-and-test runs.
    #[arg(long, default_missing_value = "true", num_args = 0..=1, requires = "mutate")]
    pub mutation_via_ir: Option<bool>,

    /// Enable brutalization mode.
    ///
    /// Catches latent bugs that normal tests miss because the EVM initializes
    /// memory to zero and registers to clean values. Applies source-level
    /// sanitizers before compiling:
    ///
    /// - Dirties unused bits in sub-256-bit type casts (address, uint8, bytes4, etc.) to catch
    ///   assembly code that assumes clean upper bits when using legacy codegen. Via-IR may clean
    ///   these bits before inline assembly observes them.
    /// - Fills scratch space (0x00-0x3f) and memory beyond the free memory pointer with junk to
    ///   catch uninitialized memory reads
    /// - Misaligns the free memory pointer to catch word-alignment assumptions
    ///
    /// If `forge test` passes but `forge test --brutalize` fails, the code has
    /// a robustness issue that could manifest when called in a different context.
    // TODO: evaluate if we can relax the conflict with replay_symbolic_artifact
    #[arg(long, conflicts_with_all = ["mutate", "replay_symbolic_artifact"])]
    pub brutalize: bool,
}

impl TestArgs {
    pub async fn run(mut self) -> Result<TestOutcome> {
        trace!(target: "forge::test", "executing test command");
        self.compile_and_run().await
    }

    pub(crate) fn ensure_mutation_mode_compatible(&self, coverage: bool) -> Result<()> {
        if self.mutate.is_none() {
            return Ok(());
        }
        // Mutation testing has bespoke orchestration that is not compatible with these modes.
        // Run this before compiling when the caller owns the build step so project errors do not
        // mask CLI conflicts.
        let conflicts = enabled_flags([
            (self.list, "--list"),
            (self.debug, "--debug"),
            (self.flamegraph, "--flamegraph"),
            (self.flamechart, "--flamechart"),
            (self.evm_profile.is_some(), "--evm-profile"),
            (self.junit, "--junit"),
            (self.json_file.is_some(), "--json-file"),
            (coverage, "coverage"),
            (self.showmap_out.is_some(), "--showmap-out"),
            (self.replay_symbolic_artifact.is_some(), "--replay-symbolic-artifact"),
        ]);
        if !conflicts.is_empty() {
            bail!(
                "`--mutate` cannot be combined with: {}. Re-run without those flags to use \
                 mutation testing.",
                conflicts.join(", ")
            );
        }
        Ok(())
    }

    pub(crate) fn ensure_coverage_mode_compatible(&self) -> Result<()> {
        self.ensure_mutation_mode_compatible(true)?;
        let conflicts = enabled_flags([
            (shell::is_json(), "--json"),
            (self.junit, "--junit"),
            (self.json_file.is_some(), "--json-file"),
            (self.list, "--list"),
            (self.debug, "--debug"),
            (self.flamegraph, "--flamegraph"),
            (self.flamechart, "--flamechart"),
            (self.evm_profile.is_some(), "--evm-profile"),
            (self.showmap_out.is_some(), "--showmap-out"),
            (self.brutalize, "--brutalize"),
            (self.replay_symbolic_artifact.is_some(), "--replay-symbolic-artifact"),
        ]);
        if !conflicts.is_empty() {
            bail!(
                "`forge coverage` cannot be combined with: {}. Use `--report lcov` for an \
                 interoperable coverage report or `--report attribution` for per-test JSON \
                attribution.",
                conflicts.join(", ")
            );
        }
        Ok(())
    }

    /// Builds a `ShowmapConfig` from the showmap CLI flags, if `--showmap-out` is set.
    fn showmap_config(&self) -> Result<Option<ShowmapConfig>> {
        let showmap = match (&self.showmap_override, &self.showmap_out) {
            (Some(showmap), _) => showmap.clone(),
            (None, Some(out_dir)) => ShowmapConfig {
                out_dir: out_dir.clone(),
                approach: self.showmap_approach.clone(),
                // Default trial id uses nanosecond precision so back-to-back invocations
                // don't collide and overwrite each other's output files.
                trial: self.showmap_trial.clone().unwrap_or_else(|| {
                    let ns = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0);
                    format!("trial-{ns}")
                }),
                per_input: self.showmap_per_input,
                domain: self.showmap_domain.into(),
                corpus_dir: self.showmap_corpus_dir.clone(),
                emit_files: true,
            },
            (None, None) => return Ok(None),
        };
        validate_showmap_config(&showmap)?;
        Ok(Some(showmap))
    }

    /// Restricts this test invocation to fuzz and invariant tests.
    pub(crate) const fn enable_fuzz_only(&mut self) {
        self.fuzz_only = true;
    }

    /// Restricts this test invocation to fuzz and invariant tests and enables a default fuzz corpus
    /// dir after user config is loaded.
    pub(crate) const fn enable_fuzz_only_with_auto_fuzz_corpus(&mut self) {
        self.fuzz_only = true;
        self.auto_fuzz_corpus = true;
    }

    fn apply_test_config_overrides(&self, config: &mut Config) {
        if self.auto_fuzz_corpus && config.fuzz.corpus.corpus_dir.is_none() {
            config.fuzz.corpus.corpus_dir = Some(match &config.fuzz.failure_persist_dir {
                Some(root) => root.join(AUTO_CORPUS_DIR),
                None => config.cache_path.join(AUTO_FUZZ_FAILURE_DIR).join(AUTO_CORPUS_DIR),
            });
        }
        if self.debug && !config.extra_output.contains(&ContractOutputSelection::StorageLayout) {
            config.extra_output.push(ContractOutputSelection::StorageLayout);
        }
    }

    /// Disables gas report sampling unless a gas report is requested, in which case isolation is
    /// enabled for more correct gas accounting.
    const fn apply_gas_report_overrides(&self, config: &mut Config, evm_opts: &mut EvmOpts) {
        if self.gas_report {
            evm_opts.isolate = true;
        } else {
            config.fuzz.gas_report_samples = 0;
            config.invariant.gas_report_samples = 0;
        }
    }

    /// Overrides showmap config for callers that reuse replay mode without the
    /// `forge test --showmap-*` CLI flags.
    pub(crate) fn set_showmap_override(&mut self, showmap: ShowmapConfig) {
        self.showmap_override = Some(showmap);
    }

    /// Sets replay-critical options for internal fuzz minimizer callers.
    pub(crate) fn set_fuzz_minimize_replay_options(
        &mut self,
        global: GlobalArgs,
        evm: EvmArgs,
        build: BuildOpts,
        filter: FilterArgs,
    ) {
        self.global = global;
        self.evm = evm;
        self.build = build;
        self.filter = filter;
    }

    /// Replays persisted fuzz failures without running a new fuzz campaign.
    pub(crate) const fn enable_fuzz_failure_replay(&mut self) {
        self.fuzz_failure_replay = true;
    }

    fn warn_unsupported_engine_flags(
        &self,
        output: &ProjectCompileOutput,
        config: &Config,
        inline_config: &InlineConfig,
        filter: &ProjectPathsAwareFilter,
        multi_network: &MultiNetworkConfig,
    ) -> Result<()> {
        if !self.fuzz_only {
            return Ok(());
        }
        let matcher = TestFunctionMatcher::new(config, inline_config, None);
        let (mut fuzz, mut invariant) = (0, 0);
        for (id, _, abi) in matching_test_contracts(output, config, &matcher, filter) {
            let (f, i) = matcher.count_fuzz_engine_targets(filter, &id, abi, multi_network);
            fuzz += f;
            invariant += i;
        }
        let unused: &[(bool, &str, &str)] = if fuzz == 0 && invariant > 0 {
            &[
                (
                    self.fuzz_frontier_dir.is_some() && self.invariant_frontier_dir.is_none(),
                    "--frontier-dir",
                    "fuzz",
                ),
                (
                    self.fuzz_frontier_limit.is_some() && self.invariant_frontier_limit.is_none(),
                    "--frontier-limit",
                    "fuzz",
                ),
                (self.fuzz_run.is_some(), "--fuzz-run", "fuzz"),
            ]
        } else if invariant == 0 && fuzz > 0 {
            &[
                (self.invariant_depth.is_some(), "--depth", "invariant"),
                (self.invariant_min_depth.is_some(), "--min-depth", "invariant"),
                (self.invariant_depth_mode.is_some(), "--depth-mode", "invariant"),
                (self.invariant_workers.is_some(), "--workers", "invariant"),
            ]
        } else {
            &[]
        };
        for (set, flag, engine) in unused {
            if *set {
                sh_warn!(
                    "`{flag}` only applies to {engine} tests; no matched {engine} tests were found."
                )?;
            }
        }
        Ok(())
    }

    /// Builds the delegated `forge test` invocation for `forge fuzz run`.
    pub(crate) fn from_fuzz_run(args: FuzzRunArgs) -> Self {
        let campaign = args.campaign;
        Self {
            fuzz_only: true,
            global: args.global,
            path: args.path,
            gas_report: args.gas_report,
            allow_failure: args.allow_failure,
            junit: args.junit,
            fail_fast: args.fail_fast,
            etherscan_api_key: args.etherscan_api_key,
            list: args.list,
            fuzz_input_file: args.fuzz_input_file,
            show_progress: args.show_progress,
            rerun: args.rerun,
            showmap_out: args.showmap_out,
            showmap_per_input: args.showmap_per_input,
            showmap_domain: args.showmap_domain,
            showmap_approach: args.showmap_approach,
            showmap_trial: args.showmap_trial,
            showmap_corpus_dir: args.showmap_corpus_dir,
            filter: args.filter,
            evm: args.evm,
            build: args.build,
            fuzz_seed: campaign.seed,
            fuzz_runs: campaign.runs,
            invariant_runs_override: campaign.runs,
            fuzz_timeout: campaign.timeout.map(u64::from),
            invariant_timeout_override: campaign.timeout,
            fuzz_dictionary_weight: campaign.dictionary_weight,
            invariant_dictionary_weight: campaign.dictionary_weight,
            fuzz_dictionary_addresses: campaign.dictionary_addresses.clone(),
            invariant_dictionary_addresses: campaign.dictionary_addresses,
            fuzz_dictionary_values: campaign.dictionary_values.clone(),
            invariant_dictionary_values: campaign.dictionary_values,
            fuzz_dictionary_literals: campaign.dictionary_literals.clone(),
            invariant_dictionary_literals: campaign.dictionary_literals,
            fuzz_corpus_random_sequence_weight: campaign.corpus_random_sequence_weight,
            invariant_corpus_random_sequence_weight: campaign.corpus_random_sequence_weight,
            fuzz_corpus_dir: campaign.corpus_dir.clone(),
            invariant_corpus_dir: campaign.corpus_dir,
            fuzz_payable_value_weight: campaign.payable_value_weight,
            invariant_payable_value_weight: campaign.payable_value_weight,
            fuzz_mutation_weight_splice: campaign.mutation_weight_splice,
            invariant_mutation_weight_splice: campaign.mutation_weight_splice,
            fuzz_mutation_weight_repeat: campaign.mutation_weight_repeat,
            invariant_mutation_weight_repeat: campaign.mutation_weight_repeat,
            fuzz_mutation_weight_interleave: campaign.mutation_weight_interleave,
            invariant_mutation_weight_interleave: campaign.mutation_weight_interleave,
            fuzz_mutation_weight_prefix: campaign.mutation_weight_prefix,
            invariant_mutation_weight_prefix: campaign.mutation_weight_prefix,
            fuzz_mutation_weight_suffix: campaign.mutation_weight_suffix,
            invariant_mutation_weight_suffix: campaign.mutation_weight_suffix,
            fuzz_mutation_weight_abi: campaign.mutation_weight_abi,
            invariant_mutation_weight_abi: campaign.mutation_weight_abi,
            fuzz_mutation_weight_cmp: campaign.mutation_weight_cmp,
            invariant_mutation_weight_cmp: campaign.mutation_weight_cmp,
            fuzz_frontier_dir: campaign.frontier_dir.clone(),
            invariant_frontier_dir: campaign.frontier_dir,
            fuzz_frontier_limit: campaign.frontier_limit,
            invariant_frontier_limit: campaign.frontier_limit,
            invariant_depth: campaign.depth,
            invariant_min_depth: campaign.min_depth,
            invariant_depth_mode: campaign.depth_mode,
            invariant_workers: campaign.workers,
            ..Self::default()
        }
    }

    fn load_symbolic_artifact_replay(&self) -> Result<Option<SymbolicArtifactReplayConfig>> {
        let Some(path) = &self.replay_symbolic_artifact else {
            return Ok(None);
        };
        if !self.filter.is_empty() || self.path.is_some() {
            bail!(
                "symbolic artifact mode cannot be combined with test selection filters; \
                 the artifact selects its original target"
            );
        }

        let display = path.display();
        let value = fs::read_json_file::<serde_json::Value>(path)
            .wrap_err(format!("failed to read symbolic counterexample artifact {display}"))?;
        let schema_version =
            value.get("schema_version").and_then(serde_json::Value::as_u64).ok_or_else(|| {
                eyre::eyre!(
                    "symbolic counterexample artifact {display} is missing numeric schema_version"
                )
            })?;
        if schema_version != 1 {
            bail!(
                "unsupported symbolic counterexample artifact schema version {schema_version} in {display}"
            );
        }
        let schema = value.get("schema").and_then(serde_json::Value::as_str).ok_or_else(|| {
            eyre::eyre!("symbolic counterexample artifact {display} is missing string schema")
        })?;
        if schema != SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA {
            bail!("unsupported symbolic counterexample artifact schema `{schema}` in {display}");
        }
        let artifact = serde_json::from_value::<SymbolicCounterexampleArtifact>(value)
            .wrap_err(format!("failed to parse symbolic counterexample artifact {display}"))?;
        if artifact.calls.is_empty() {
            bail!("symbolic counterexample artifact {display} has no calls");
        }
        if artifact.replay.status != SymbolicReplayStatus::Confirmed {
            bail!(
                "symbolic counterexample artifact {display} replay status must be confirmed, got {:?}",
                artifact.replay.status,
            );
        }
        let contract = &artifact.test.contract;
        if !matches!(contract.rsplit_once(':'), Some((p, n)) if !p.is_empty() && !n.is_empty()) {
            bail!(
                "symbolic counterexample artifact {display} test.contract must be `path:Contract`, got `{contract}`"
            );
        }
        Ok(Some(SymbolicArtifactReplayConfig { artifact, path: path.clone() }))
    }

    fn load_fuzz_input(
        &self,
        output: &ProjectCompileOutput,
        config: &Config,
        inline_config: &InlineConfig,
        filter: &ProjectPathsAwareFilter,
    ) -> Result<Option<FuzzFailureReplayConfig>> {
        let Some(path) = &self.fuzz_input_file else {
            return Ok(None);
        };
        let failure = fs::read_json_file::<BaseCounterExample>(path)?;
        let Some(selector) = failure.calldata.get(..4) else {
            bail!(
                "fuzz input file {} contains calldata shorter than a 4-byte selector",
                path.display()
            );
        };
        let targets =
            matching_fuzz_replay_targets(output, config, inline_config, filter, selector)?;
        let [(contract, test)] = targets.as_slice() else {
            if targets.is_empty() {
                bail!(
                    "fuzz input file {} does not match any selected stateless fuzz test",
                    path.display()
                );
            }
            bail!(
                "fuzz input file {} matches {} selected stateless fuzz tests; replay requires exactly one target",
                path.display(),
                targets.len()
            );
        };
        Ok(Some(FuzzFailureReplayConfig {
            failure: Arc::new(failure),
            contract: contract.clone(),
            test: test.clone(),
        }))
    }

    /// Returns a list of files that need to be compiled in order to run all the tests that match
    /// the given filter, and the inline config parsed from the ABI-only compilation when one was
    /// needed.
    ///
    /// For filtered runs, this includes all configured source roots, non-test fixture roots, and
    /// runnable tests that match the filter. Imported dependencies remain attached to those roots
    /// so compiler profiles and restrictions apply to the complete source graph.
    #[instrument(target = "forge::test", skip_all)]
    fn get_sources_to_compile(
        &self,
        config: &Config,
        test_filter: &ProjectPathsAwareFilter,
        symbolic_artifact_replay: Option<&SymbolicArtifactReplayConfig>,
    ) -> Result<(BTreeSet<PathBuf>, Option<Arc<InlineConfig>>)> {
        let src_files = || source_files_iter(&config.src, MultiCompilerLanguage::FILE_EXTENSIONS);
        let test_files = || source_files_iter(&config.test, MultiCompilerLanguage::FILE_EXTENSIONS);

        // An empty filter doesn't filter out anything.
        // We can still optimize slightly by excluding scripts.
        if test_filter.is_empty() {
            return Ok((src_files().chain(test_files()).collect(), None));
        }

        let mut project = config.create_project(true, true)?;
        let sources = src_files()
            .chain(
                // Preserve path-filter behavior for conventional test files while still
                // scanning non-test fixtures under the test root.
                test_files().filter(|path| !path.is_sol_test() || test_filter.matches_path(path)),
            )
            .collect::<BTreeSet<_>>();
        let output = compile_abi_project(
            &mut project,
            ProjectCompiler::new()
                .files(sources.iter().cloned())
                .dynamic_test_linking(config.dynamic_test_linking)
                .quiet(true),
        )?;
        if output.has_compiler_errors() {
            sh_println!("{output}")?;
            bail!("Compilation failed");
        }

        let inline_config = Arc::new(InlineConfig::new_parsed(&output, config)?);
        let test_matcher =
            TestFunctionMatcher::new(config, &inline_config, symbolic_artifact_replay);
        let paths = config.project_paths::<MultiCompilerLanguage>();
        let empty_filter = EmptyTestFilter::default();
        let filter_args = test_filter.args();
        let has_contract_or_test_filter = filter_args.test_pattern.is_some()
            || filter_args.test_pattern_inverse.is_some()
            || filter_args.contract_pattern.is_some()
            || filter_args.contract_pattern_inverse.is_some();

        // `MultiContractRunner::build` strips the root prefix from artifact source paths so the
        // identifiers it constructs are project-relative. Match that here for the filter check
        // (notably for the `--rerun` failure list, which is persisted relative) but return the
        // original absolute source paths so downstream compilation can locate them.
        let files = output
            .artifact_ids()
            .filter_map(|(id, artifact)| artifact.abi.as_ref().map(|abi| (id, abi)))
            // Imported dependencies must remain attached to their roots so compilation restrictions
            // apply to the entire source graph instead of compiling dependencies with default
            // settings.
            .filter(|(id, _)| sources.contains(&id.source))
            .filter(|(id, abi)| {
                if id.source.starts_with(&paths.sources) {
                    return true;
                }
                if paths.is_script(&id.source) && !paths.is_test(&id.source) {
                    return false;
                }
                let stripped = id.clone().with_stripped_file_prefixes(&config.root);
                // ABI-only compilation can omit test functions with invalid bodies, so preserve the
                // existing filter behavior for conventional test files instead of treating them as
                // fixtures.
                if stripped.source.is_sol_test() {
                    return if has_contract_or_test_filter {
                        test_matcher.matches_contract(test_filter, &stripped, abi)
                    } else {
                        test_filter.matches_path(&stripped.source)
                    };
                }
                !test_matcher.matches_contract(&empty_filter, &stripped, abi)
                    || test_matcher.matches_contract(test_filter, &stripped, abi)
            })
            .map(|(id, _)| id.source)
            .collect();
        Ok((files, Some(inline_config)))
    }

    /// Executes all the tests in the project.
    ///
    /// This will trigger the build process first. On success all test contracts that match the
    /// configured filter will be executed
    ///
    /// Returns the test results for all matching tests.
    pub async fn compile_and_run(&mut self) -> Result<TestOutcome> {
        self.ensure_mutation_mode_compatible(false)?;
        let compiled = self.compile_project().await?;
        self.run_tests(
            &compiled.project_root,
            compiled.config,
            compiled.evm_opts,
            &compiled.output,
            &compiled.filter,
            TestExecutionOptions {
                replay_symbolic_artifact: compiled.replay_symbolic_artifact,
                selected_sources: compiled.selected_sources,
                ..TestExecutionOptions::default_run(compiled.inline_config)
            },
        )
        .await
    }

    /// Copies the project into a temporary workspace, brutalizes its sources and rebases `config`
    /// onto that workspace.
    fn brutalize_workspace(&self, config: &mut Config) -> Result<TempDir> {
        let silent = shell::is_json();
        let temp_dir = TempDir::with_prefix("forge_brutalize_")?;
        let temp_path = temp_dir.path();

        if config.via_ir && !silent {
            sh_warn!(
                "--brutalize value cast dirty-bits checks are ineffective with via-IR; memory and free-memory-pointer checks still apply"
            )?;
        }
        if !silent {
            sh_status!("Brutalizing source files...")?;
        }
        workspace::copy_project(config, temp_path)?;
        let count = brutalizer::brutalize_project(config, temp_path)?;
        if !silent {
            sh_status!("Brutalized {count} source files, compiling from temp workspace...")?;
        }

        let test_failures_file = config.test_failures_file.clone();
        *config = workspace::rebase_config_paths(config, temp_path).sanitized();
        config.test_failures_file = test_failures_file;
        Ok(temp_dir)
    }

    async fn compile_project(&mut self) -> Result<CompiledTestProject> {
        // Merge all configs.
        let (mut config, evm_opts) = self.load_config_and_evm_opts()?;

        if install::install_missing_dependencies(&mut config).await && config.auto_detect_remappings
        {
            // need to re-configure here to also catch additional remappings
            config = self.load_config()?;
        }
        let brutalized_workspace =
            if self.brutalize { Some(self.brutalize_workspace(&mut config)?) } else { None };
        let should_mutate = self.mutate.is_some();
        if should_mutate {
            // Force dyn test linking and cache usage for mutation testing after any config reload.
            config.dynamic_test_linking = true;
            config.cache = true;
            apply_mutation_compiler_overrides(&mut config);
        }
        self.apply_test_config_overrides(&mut config);

        // Set up the project.
        let mut project = config.project()?;
        let project_root = project.paths.root.clone();

        let replay_symbolic_artifact = self.load_symbolic_artifact_replay()?;
        let mut filter = self.filter(&config)?;
        if let Some(replay) = &replay_symbolic_artifact {
            let filter_args = filter.args_mut();
            filter_args.test_pattern_inverse = None;
            filter_args.contract_pattern_inverse = None;
            filter_args.path_pattern_inverse = None;
            let contract = replay.artifact.test.contract.as_str();
            let (path, contract) = contract.rsplit_once(':').unwrap_or(("", contract));
            filter_args.test_pattern =
                Some(Regex::new(&format!("^{}$", regex::escape(&replay.artifact.test.test)))?);
            filter_args.contract_pattern =
                Some(Regex::new(&format!("^{}$", regex::escape(contract)))?);
            if !path.is_empty() {
                filter_args.path_pattern = Some(globset::escape(path).parse::<GlobMatcher>()?);
            }
        }
        trace!(target: "forge::test", ?filter, "using filter");

        let compiler = ProjectCompiler::new()
            .dynamic_test_linking(config.dynamic_test_linking)
            .quiet(shell::is_json() || self.junit);
        let (output, selected_sources, inline_config) = if self.list {
            // Only the ABI is needed to list tests, so skip the full compile when possible.
            let compiler = if filter.args().path_pattern.is_some()
                && config.extra_output.is_empty()
                && config.extra_output_files.is_empty()
                && !config.build_info
            {
                let files = project
                    .paths
                    .input_files_iter()
                    .filter(|path| filter.matches_path(path))
                    .collect::<Vec<_>>();
                if files.is_empty() { compiler } else { compiler.files(files) }
            } else {
                compiler
            };
            (compile_abi_project(&mut project, compiler)?, BTreeSet::new(), None)
        } else {
            let (files, inline_config) =
                self.get_sources_to_compile(&config, &filter, replay_symbolic_artifact.as_ref())?;
            let output = compiler.files(files.clone()).compile(&project);
            let output = if should_mutate {
                output.wrap_err(
                    "Mutation testing compiler profile failed to compile before applying mutations",
                )?
            } else {
                output?
            };
            (output, files, inline_config)
        };
        let inline_config = match inline_config {
            Some(inline_config) => inline_config,
            None => Arc::new(InlineConfig::new_parsed(&output, &config)?),
        };

        Ok(CompiledTestProject {
            project_root,
            config,
            evm_opts,
            output,
            filter,
            inline_config,
            replay_symbolic_artifact,
            selected_sources,
            _brutalized_workspace: brutalized_workspace,
        })
    }

    pub(crate) async fn prepare_fuzz_minimize_replay(
        &mut self,
        corpus_dir: &Path,
    ) -> Result<FuzzMinimizeReplaySession> {
        let CompiledTestProject { mut config, mut evm_opts, output, filter, inline_config, .. } =
            self.compile_project().await?;

        if config.fuzz.run == Some(0) {
            bail!("`fuzz.run` must be greater than 0");
        }
        self.apply_gas_report_overrides(&mut config, &mut evm_opts);
        for corpus in [&mut config.fuzz.corpus, &mut config.invariant.corpus] {
            corpus.corpus_dir.get_or_insert_with(|| corpus_dir.to_path_buf());
        }
        config.fuzz.seed = config.fuzz.seed.or(Some(U256::ZERO));

        evm_opts.infer_network_from_fork().await?;
        config.networks = evm_opts.networks;

        let override_networks = inline_config.referenced_override_networks(&config.profile);
        let (default_pass, override_passes) = network_passes(config, evm_opts, &override_networks);
        let mut passes = Vec::new();
        for NetworkPass { config, evm_opts, multi_network } in
            std::iter::once(default_pass).chain(override_passes)
        {
            let execution = TestExecutionOptions {
                multi_network,
                ..TestExecutionOptions::default_run(inline_config.clone())
            };
            let config = Arc::new(config);
            passes.push(dispatch_network!(&evm_opts, |Net| {
                let runner = self
                    .build_runner::<Net>(
                        config,
                        evm_opts,
                        &output,
                        execution,
                        None,
                        ExecutorBuilder::<Net>::new(),
                    )
                    .await?;
                fuzz_minimize_pass(runner, &filter)
            }));
        }

        if passes.iter().all(|pass| pass.target_count == 0) {
            bail!("fuzz minimization requires at least one matched fuzz or invariant test");
        }
        Ok(FuzzMinimizeReplaySession { filter, passes })
    }

    /// Executes all the tests in the project.
    ///
    /// See [`Self::compile_and_run`] for more details.
    pub(crate) async fn run_tests(
        &mut self,
        project_root: &Path,
        mut config: Config,
        mut evm_opts: EvmOpts,
        output: &ProjectCompileOutput,
        filter: &ProjectPathsAwareFilter,
        mut execution: TestExecutionOptions,
    ) -> Result<TestOutcome> {
        self.ensure_mutation_mode_compatible(execution.coverage)?;

        if config.fuzz.run == Some(0) {
            bail!("`fuzz.run` must be greater than 0");
        }

        if self.list {
            return list_from_output(
                output,
                &config,
                &execution.inline_config,
                filter,
                self.fuzz_only,
                execution.replay_symbolic_artifact.as_ref(),
            );
        }

        execution.fuzz_input =
            self.load_fuzz_input(output, &config, &execution.inline_config, filter)?;
        self.warn_unsupported_engine_flags(
            output,
            &config,
            &execution.inline_config,
            filter,
            &execution.multi_network,
        )?;

        let mut filter = filter.clone();
        self.apply_gas_report_overrides(&mut config, &mut evm_opts);

        // Generate a random fuzz seed if none provided, for reproducibility.
        config.fuzz.seed = config
            .fuzz
            .seed
            .or_else(|| Some(U256::from_be_bytes(rand::rng().random::<[u8; 32]>())));

        let trace_output = if self.flamegraph {
            Some(TraceOutputKind::Flamegraph)
        } else if self.flamechart {
            Some(TraceOutputKind::Flamechart)
        } else {
            self.evm_profile.map(TraceOutputKind::EvmProfile)
        };

        // Determine executor verbosity.
        if evm_opts.verbosity < 3 && (self.gas_report || trace_output.is_some()) {
            evm_opts.verbosity = 3;
        }

        // Enable internal tracing for more informative flamegraph/profile. Simple tracing is
        // upgraded to full tracing in `run_tests_inner` when exactly one test matches.
        config.tracing = self.tracing.resolve(&config.tracing, evm_opts.verbosity);
        let json_trace_depth = config.tracing.trace_depth;
        execution.decode_internal = if config.tracing.decode_internal || trace_output.is_some() {
            InternalTraceMode::Simple
        } else {
            InternalTraceMode::None
        };

        // Auto-detect network from fork chain ID when not explicitly configured.
        evm_opts.infer_network_from_fork().await?;
        // Inline configuration starts from this base config. Materialize the inferred execution
        // network so unrelated inline overrides cannot erase the fork's EVM family.
        config.networks = evm_opts.networks;
        let verbosity = evm_opts.verbosity;

        // Clone config and evm_opts before dispatch (needed for mutation testing).
        let config_for_mutation = config.clone();
        let evm_opts_for_mutation = evm_opts.clone();
        let mutation_fork =
            if self.mutate.is_some() { evm_opts.resolve_fork().await? } else { None };

        // Run each distinct per-test network annotation as a separate pass and merge results.
        let override_networks =
            execution.inline_config.referenced_override_networks(&config.profile);
        let is_multi_pass = !override_networks.is_empty();
        let multi_pass_timer = Instant::now();
        let (default_pass, override_passes) = network_passes(config, evm_opts, &override_networks);
        let (libraries, mut outcome) = self
            .run_network_pass(
                default_pass,
                output,
                &mut filter,
                execution.clone(),
                mutation_fork.as_ref(),
            )
            .await?;
        for pass in override_passes {
            let (_, pass_outcome) =
                self.run_network_pass(pass, output, &mut filter, execution.clone(), None).await?;
            merge_outcomes(&mut outcome, pass_outcome);
        }
        if is_multi_pass {
            // Per-pass summaries are suppressed in `run_tests_inner`.
            self.print_summary(&outcome, multi_pass_timer.elapsed())?;
        }

        if let Some(replay) = &execution.replay_symbolic_artifact {
            let target = &replay.artifact.test;
            match outcome.tests().count() {
                0 => bail!(
                    "symbolic artifact target `{}::{}` was not found",
                    target.contract,
                    target.test
                ),
                1 => {}
                replayed => bail!(
                    "symbolic artifact target `{}::{}` matched {replayed} tests; replay requires exactly one target",
                    target.contract,
                    target.test
                ),
            }
        }

        if let Some(path) = &self.json_file {
            let mut results =
                outcome.json_file_results.take().unwrap_or_else(|| outcome.results.clone());
            prepare_results_for_json(&mut results, verbosity, json_trace_depth);
            fs::write_json_file(path, &results)?;
        }

        if let Some(trace_output) = trace_output {
            self.render_trace_output(trace_output, &mut outcome).await?;
        }

        if self.debug {
            // Get first non-empty suite result. We will have only one such entry.
            let (_, _, test_result) =
                outcome.remove_first().ok_or_eyre("no tests were executed")?;
            let sources =
                ContractSources::from_project_output(output, project_root, Some(&libraries))?;

            // Prefer execution traces for normal debug runs, but when execution never starts
            // (for example if `setUp()` reverts), fall back to available setup/deployment traces.
            let mut traces = test_result
                .traces
                .iter()
                .filter(|(kind, _)| kind.is_execution())
                .cloned()
                .collect::<Vec<_>>();
            if traces.is_empty() {
                traces = test_result.traces.clone();
            }
            if let Some(decoder) = &outcome.last_run_decoder {
                for (_, arena) in &mut traces {
                    decode_trace_arena(arena, decoder).await;
                }
            }

            let mut builder = Debugger::builder()
                .traces(traces)
                .sources(sources)
                .breakpoints(test_result.breakpoints)
                .layout(self.debug_layout.unwrap_or_default());
            if let Some(decoder) = &outcome.last_run_decoder {
                builder = builder.decoder(decoder);
            }
            if let Some(known_contracts) = &outcome.known_contracts {
                builder = builder.known_contracts(known_contracts);
            }
            let mut debugger = builder.build();
            if let Some(dump_path) = &self.dump {
                debugger.dump_to_file(dump_path)?;
            } else {
                debugger.try_run_tui()?;
            }
        }

        // All tests have been run once before reaching this point
        if let Some(mutate) = &self.mutate {
            if outcome.failed() > 0 {
                bail!(
                    "Mutation testing compiler profile failed its unmutated baseline run; \
                     adjust `--mutation-via-ir` / `--mutation-optimizer-runs` or fix the tests \
                     before running mutation testing"
                );
            }
            // A green baseline that ran zero non-skipped tests would report every compileable
            // mutant as `Alive`, so hard-error instead of producing a misleading report.
            if outcome.successes().next().is_none() {
                bail!(
                    "Mutation testing requires at least one passing baseline test; the current \
                     filter/path selection matched zero non-skipped tests. Loosen `--match-test` / \
                     `--match-contract` / `--match-path` or check the project layout."
                );
            }
            // Clap can't express this conflict because `--mutate` takes an optional list of paths.
            if !mutate.is_empty() && self.mutate_path.is_some() {
                bail!(
                    "`--mutate-path <PATTERN>` cannot be combined with explicit paths passed to `--mutate`; pass either paths or a glob pattern, not both"
                );
            }
            // The mutation runner builds a single-pass `MultiContractRunner` and does not honor
            // inline per-test network annotations, which would silently run tests on the wrong
            // network and produce false survivors / kills.
            if is_multi_pass {
                bail!(
                    "Mutation testing does not yet support inline per-test network overrides \
                     (found {} annotated network(s)). Re-run without `--mutate` or remove the \
                     per-test network annotations.",
                    override_networks.len()
                );
            }
            ensure_mutation_workspace_safe(&config_for_mutation)?;

            let json_output = shell::is_json();
            let selected_sources_relative = execution
                .selected_sources
                .iter()
                .filter_map(|path| {
                    path.strip_prefix(&config_for_mutation.root).ok().map(PathBuf::from)
                })
                .collect::<Vec<_>>();
            let mutation_config = MutationRunConfig {
                mutate_paths: mutate.clone(),
                mutate_path_pattern: self.mutate_path.clone(),
                mutate_contract_pattern: self.mutate_contract.clone(),
                num_workers: self.mutation_jobs.unwrap_or(0),
                show_progress: self.show_progress,
                json_output,
                // Carry the filter the baseline actually used (positional path shorthand folded
                // into `path_pattern`, `--rerun` failures injected into `test_pattern`) and its
                // isolation flag so every mutant exercises the exact same test set.
                filter_args: filter.args().clone(),
                rerun_failures: filter.rerun_failures().map(<[RerunFailure]>::to_vec),
                selected_sources_relative,
                isolate: evm_opts_for_mutation.isolate,
            };
            let result = run_mutation_testing(
                Arc::new(config_for_mutation),
                output,
                evm_opts_for_mutation,
                mutation_fork,
                mutation_config,
            )
            .await?;
            if result.cancelled {
                std::process::exit(130);
            }
            if json_output {
                let json_output = result.summary.to_json_output(result.duration_secs);
                sh_println!("{}", serde_json::to_string(&json_output)?)?;
            }
            outcome = TestOutcome::empty(None, true);
        }

        Ok(outcome)
    }

    /// Renders the flamegraph, flamechart or EVM profile of the single executed test.
    async fn render_trace_output(
        &self,
        trace_output: TraceOutputKind,
        outcome: &mut TestOutcome,
    ) -> Result<()> {
        let label = trace_output.label();
        let no_tests = match trace_output {
            TraceOutputKind::EvmProfile(_) => "cannot generate EVM profile: no tests were executed",
            TraceOutputKind::Flamegraph | TraceOutputKind::Flamechart => "no tests were executed",
        };
        if outcome.tests().next().is_none() {
            bail!("{no_tests}");
        }
        let decoder = outcome
            .last_run_decoder
            .clone()
            .ok_or_else(|| eyre::eyre!("cannot generate {label}: missing trace decoder"))?;
        let (suite_name, test_name, test_result) =
            outcome
                .results
                .iter_mut()
                .find_map(|(suite_name, suite)| {
                    suite.test_results.iter_mut().next().map(|(test_name, result)| {
                        (suite_name.as_str(), test_name.as_str(), result)
                    })
                })
                .ok_or_else(|| eyre::eyre!("{no_tests}"))?;
        let contract = suite_name.split(':').next_back().unwrap();
        let test_name = test_name.trim_end_matches("()");
        let (_, arena) = test_result
            .traces
            .iter_mut()
            .find(|(kind, _)| *kind == TraceKind::Execution)
            .ok_or_else(|| {
                eyre::eyre!(
                    "cannot generate {label} for {contract}::{test_name}: no execution trace \
                     (test may have failed in setUp/constructor or been skipped)"
                )
            })?;
        decode_trace_arena(arena, &decoder).await;

        match trace_output {
            TraceOutputKind::Flamegraph | TraceOutputKind::Flamechart => {
                let mut folded_stack_trace = folded_stack_trace::build(arena, self.evm.isolate);
                let flame_chart = trace_output == TraceOutputKind::Flamechart;
                if flame_chart {
                    folded_stack_trace.reverse();
                }
                let file_name = format!("cache/{label}_{contract}_{test_name}.svg");
                let file = std::fs::File::create(&file_name).wrap_err("failed to create file")?;
                let mut options = inferno::flamegraph::Options::default();
                options.title = format!("{label} {contract}::{test_name}");
                options.count_name = "gas".to_string();
                options.flame_chart = flame_chart;
                inferno::flamegraph::from_lines(
                    &mut options,
                    folded_stack_trace.iter().map(String::as_str),
                    std::io::BufWriter::new(file),
                )
                .wrap_err("failed to write svg")?;
                sh_println!("Saved to {file_name}")?;
                if !self.no_open
                    && let Err(e) = opener::open(&file_name)
                {
                    sh_err!("Failed to open {file_name}; please open it manually: {e}")?;
                }
            }
            TraceOutputKind::EvmProfile(EvmProfileFormat::Speedscope) => {
                let profile =
                    speedscope::builder::build(arena, test_name, contract, self.evm.isolate);
                let profile_json = serde_json::to_vec(&profile)?;
                let profile_path = format!("cache/evm_profile_{contract}_{test_name}.json");
                fs::write(&profile_path, &profile_json)?;
                sh_println!("Profile saved to {profile_path}")?;
                if !self.no_open {
                    evm_profile_server::serve_and_open(profile_json, test_name, contract).await?;
                }
            }
        }
        Ok(())
    }

    /// Builds the test runner for the network selected by `evm_opts`.
    async fn build_runner<FEN: FoundryEvmNetwork>(
        &self,
        config: Arc<Config>,
        evm_opts: EvmOpts,
        output: &ProjectCompileOutput,
        execution: TestExecutionOptions,
        resolved_fork: Option<&ResolvedFork>,
        executor_builder: ExecutorBuilder<FEN>,
    ) -> Result<MultiContractRunner<FEN>> {
        let (evm_env, tx_env, fork) = if let Some(fork) = resolved_fork {
            let (evm_env, tx_env) = evm_opts
                .env_with_resolved_fork::<SpecFor<FEN>, BlockEnvFor<FEN>, TxEnvFor<FEN>>(Some(fork))
                .await?;
            (evm_env, tx_env, Some(fork.clone()))
        } else {
            evm_opts.env_resolved::<SpecFor<FEN>, BlockEnvFor<FEN>, TxEnvFor<FEN>>().await?
        };
        let fork_context = fork.as_ref().map(|fork| fork.context());
        let create2_deployer_available =
            evm_opts.can_use_create2_deployer_resolved(fork.as_ref()).await?;

        MultiContractRunnerBuilder::new(config.clone(), execution.inline_config)
            .set_debug(self.debug)
            .set_decode_internal(execution.decode_internal)
            .set_record_all_steps(self.evm_profile.is_some())
            .initial_balance(evm_opts.initial_balance)
            .sender(evm_opts.sender)
            .with_fork(evm_opts.get_fork_resolved(&config, evm_env.cfg_env.chain_id, fork.as_ref()))
            .with_fork_chain_id(fork_context.map(|context| context.source_chain_id))
            .with_fork_hardfork(fork_context.and_then(|context| context.hardfork))
            .enable_isolation(evm_opts.isolate)
            .fail_fast(self.fail_fast)
            .set_coverage(execution.coverage)
            .with_multi_network(execution.multi_network)
            .with_showmap(self.showmap_config()?)
            .with_fuzz_only(self.fuzz_only)
            .with_fuzz_failure_replay(self.fuzz_failure_replay)
            .with_fuzz_input(execution.fuzz_input)
            .with_symbolic_artifact_replay(execution.replay_symbolic_artifact)
            .with_create2_deployer_available(create2_deployer_available)
            .build::<FEN, MultiCompiler>(output, evm_env, tx_env, evm_opts, executor_builder)
    }

    /// Builds the runner for one network pass and runs its tests.
    async fn run_network_pass(
        &self,
        pass: NetworkPass,
        output: &ProjectCompileOutput,
        filter: &mut ProjectPathsAwareFilter,
        execution: TestExecutionOptions,
        resolved_fork: Option<&ResolvedFork>,
    ) -> Result<(Libraries, TestOutcome)> {
        let NetworkPass { config, evm_opts, multi_network } = pass;
        let execution = TestExecutionOptions { multi_network, ..execution };
        let verbosity = evm_opts.verbosity;
        let config = Arc::new(config);
        dispatch_network!(&evm_opts, |Net| {
            let runner = self
                .build_runner::<Net>(
                    config.clone(),
                    evm_opts,
                    output,
                    execution,
                    resolved_fork,
                    ExecutorBuilder::<Net>::new(),
                )
                .await?;
            let libraries = runner.libraries.clone();
            let outcome = self.run_tests_inner(runner, config, verbosity, filter, output).await?;
            Ok((libraries, outcome))
        })
    }

    /// Emits symbolic regression tests for the counterexample artifacts in `results` when
    /// `--emit-regression` is set, and attaches them to the results.
    fn emit_symbolic_regressions(
        &self,
        config: &Config,
        known_contracts: &ContractsByArtifact,
        results: &mut BTreeMap<String, SuiteResult>,
    ) -> Result<Vec<SymbolicRegression>> {
        if !self.emit_regression {
            return Ok(Vec::new());
        }
        let regression = SymbolicRegressionConfig {
            out: self
                .regression_out
                .clone()
                .map(|path| if path.is_relative() { config.root.join(path) } else { path }),
            overwrite: self.regression_overwrite,
        };
        let artifacts = collect_symbolic_artifacts_from_suites(results.values());
        let regressions =
            emit_symbolic_regressions(config, &regression, known_contracts, &artifacts)?;
        attach_symbolic_regressions_to_suites(results.values_mut(), &regressions);
        Ok(regressions)
    }

    /// Prints the run summary, or the detailed summary table when `--summary` is set.
    fn print_summary(&self, outcome: &TestOutcome, duration: Duration) -> Result<()> {
        if !self.summary && !shell::is_json() {
            sh_println!("{}", outcome.summary(duration))?;
        }
        if self.summary && !outcome.results.is_empty() {
            sh_println!("{}", TestSummaryReport::new(self.detailed, outcome))?;
        }
        Ok(())
    }

    /// Run all tests that matches the filter predicate from a test runner
    async fn run_tests_inner<FEN: FoundryEvmNetwork>(
        &self,
        mut runner: MultiContractRunner<FEN>,
        config: Arc<Config>,
        verbosity: u8,
        filter: &mut ProjectPathsAwareFilter,
        output: &ProjectCompileOutput,
    ) -> Result<TestOutcome> {
        let fuzz_seed = config.fuzz.seed;

        trace!(target: "forge::test", "running all tests");

        // If we need to render to a serialized format, we should not print anything else to stdout.
        let silent = shell::is_json() && (self.gas_report || self.summary || self.mutate.is_some());
        let tracing = &config.tracing;
        let trace_verbosity = tracing.verbosity;

        let mut num_filtered = runner.matching_test_functions(filter).count();

        if !self.opcodes.is_empty() && trace_verbosity < 5 {
            sh_eprintln!()?;
            bail!("Not enough verbosity. Use -vvvvv to show opcodes.");
        }

        if num_filtered == 0 {
            let total_tests = if filter.is_empty() {
                num_filtered
            } else {
                runner.matching_test_functions(&EmptyTestFilter::default()).count()
            };
            if total_tests == 0 {
                sh_warn!(
                    "No tests found in project! Forge looks for functions that start with `test`"
                )?;
            } else {
                let mut msg = format!("no tests match the provided pattern:\n{filter}");
                // Try to suggest a test when there's no match.
                if let Some(test_pattern) = &filter.args().test_pattern {
                    // Filter contracts but not test functions.
                    let candidates = runner.all_test_functions(filter).map(|f| &f.name);
                    if let Some(suggestion) =
                        utils::did_you_mean(test_pattern.as_str(), candidates).pop()
                    {
                        write!(msg, "\nDid you mean `{suggestion}`?")?;
                    }
                }
                sh_warn!("{msg}")?;
            }
            return Ok(TestOutcome::empty(Some(runner.known_contracts.clone()), false));
        }

        let debug_selection_term = Term::stderr();
        let interactive_debug_selection = self.debug
            && num_filtered != 1
            && tui_mode().is_interactive()
            && debug_selection_term.is_term();
        let mut matching_debug_tests = if interactive_debug_selection {
            collect_matching_debug_tests(&runner.list_signatures(filter))
        } else if self.debug && num_filtered != 1 {
            collect_matching_debug_tests(&runner.list(filter))
        } else {
            Vec::new()
        };
        if interactive_debug_selection {
            ctrlc::set_handler(|| {
                let _ = Term::stderr().show_cursor();
                std::process::exit(130);
            })?;

            let Some(selected) = Select::new()
                .with_prompt("Select a test to debug")
                .items(
                    matching_debug_tests
                        .iter()
                        .map(|test| format!("{}.{}", test.contract, test.test)),
                )
                .max_length(DEBUGGER_MATCHING_TESTS_DISPLAY_LIMIT)
                .interact_on_opt(&debug_selection_term)?
            else {
                bail!("Debugger test selection cancelled");
            };

            filter.set_rerun_failures(vec![matching_debug_tests.swap_remove(selected)]);
            num_filtered = 1;
        }

        if num_filtered != 1
            && (self.debug || self.flamegraph || self.flamechart || self.evm_profile.is_some())
        {
            let action = if self.flamegraph {
                "generate a flamegraph"
            } else if self.flamechart {
                "generate a flamechart"
            } else if self.evm_profile.is_some() {
                "generate an EVM profile"
            } else {
                "run the debugger"
            };
            let filter_hint = if filter.is_empty() {
                String::new()
            } else {
                format!("\n\nFilter used:\n{filter}")
            };
            let matching_tests_hint = if self.debug {
                format_matching_debug_tests(&matching_debug_tests)
            } else {
                String::new()
            };
            let narrowing_hint = if self.debug {
                "Use --match-test <TEST_NAME>, --match-contract, and --match-path to further limit the search."
            } else {
                "Use --match-contract and --match-path to further limit the search."
            };
            bail!(
                "{num_filtered} tests matched your criteria, but exactly 1 test must match in order to {action}.{matching_tests_hint}\n\n\
                 {narrowing_hint}{filter_hint}",
            );
        }

        // If exactly one test matched, we enable full tracing.
        if num_filtered == 1 && runner.decode_internal != InternalTraceMode::None {
            runner.decode_internal = InternalTraceMode::Full;
        }

        // Run tests in a non-streaming fashion and collect results for serialization.
        let serialize_json =
            self.mutate.is_none() && !self.gas_report && !self.summary && shell::is_json();
        if serialize_json || self.junit {
            let mut results = runner.test_collect(filter)?;
            if serialize_json {
                prepare_results_for_json(&mut results, verbosity, tracing.trace_depth);
            }
            self.emit_symbolic_regressions(&config, &runner.known_contracts, &mut results)?;
            let rendered = if serialize_json {
                serde_json::to_string(&results)?
            } else {
                junit_xml_report(&results, verbosity).to_string()?
            };
            sh_println!("{rendered}")?;
            return Ok(TestOutcome::new(
                Some(runner.known_contracts),
                results,
                self.allow_failure,
                fuzz_seed,
            ));
        }

        let remote_chain = runner
            .fork
            .is_some()
            .then(|| runner.tcfg.fork_chain_id.or(runner.tx_env.chain_id()))
            .flatten()
            .map(Into::into);
        let known_contracts = runner.known_contracts.clone();
        let libraries = runner.libraries.clone();

        // Capture multi-pass state before moving `runner` into the spawn task.
        // In multi-pass mode the per-pass summary is suppressed; the merged summary is
        // printed once by the caller after all passes complete.
        let is_multi_pass = !runner.tcfg.multi_network.all_override_networks.is_empty();
        let resolved_hardfork = runner.tcfg.hardfork;
        let networks = runner.tcfg.evm_opts.networks;
        let extra_cheatcode_addresses = runner.tcfg.executor_builder.extra_cheatcode_addresses();
        let decode_internal = runner.decode_internal != InternalTraceMode::None;

        // Run tests in a streaming fashion.
        let (tx, rx) = channel::<(String, SuiteResult)>();
        let timer = Instant::now();
        let show_progress = config.show_progress;
        let handle = tokio::task::spawn_blocking({
            let filter = filter.clone();
            move || runner.test(&filter, tx, show_progress).map(|()| runner)
        });

        // Set up trace identifiers.
        let mut identifier = TraceIdentifiers::new().with_local(&known_contracts);

        // Avoid using external identifiers for gas report as we decode more traces and this will be
        // expensive. Also skip external identifiers for local tests (no remote chain) to avoid
        // unnecessary Etherscan API calls that significantly slow down test execution.
        if !self.gas_report && remote_chain.is_some() {
            identifier = identifier.with_external(&config, remote_chain)?;
        }

        // Build the trace decoder.
        let mut builder = CallTraceDecoderBuilder::new()
            .with_tracing_config(tracing)
            .with_known_contracts(&known_contracts)
            .with_networks(networks)
            .with_chain_id(remote_chain.map(|c| c.id()))
            .with_hardfork(resolved_hardfork);
        // Signatures are of no value for gas reports.
        if !self.gas_report {
            builder =
                builder.with_signature_identifier(SignaturesIdentifier::from_config(&config)?);
        }
        if decode_internal {
            let sources =
                ContractSources::from_project_output(output, &config.root, Some(&libraries))?;
            builder = builder.with_debug_identifier(DebugTraceIdentifier::new(sources));
        }
        let mut decoder = builder.build();

        let mut gas_report = self.gas_report.then(|| {
            GasReport::new(
                config.gas_reports.clone(),
                config.gas_reports_ignore.clone(),
                config.gas_reports_include_tests,
                extra_cheatcode_addresses.iter().copied(),
            )
        });

        let mut gas_snapshots = BTreeMap::<String, BTreeMap<String, String>>::new();

        let mut outcome = TestOutcome::empty(None, self.allow_failure);
        outcome.fuzz_seed = fuzz_seed;

        // Some outputs need trace identities even if the textual trace is not rendered.
        let always_identify_traces = self.gas_report
            || self.debug
            || self.flamegraph
            || self.flamechart
            || self.evm_profile.is_some();

        let mut any_test_failed = false;
        let mut backtrace_builder = None;
        while let Ok((contract_name, mut suite_result)) = rx.recv() {
            let len = suite_result.len();
            let tests = &mut suite_result.test_results;
            let has_tests = !tests.is_empty();

            // In multi-pass (per-test network override) mode, skip suites that contributed no
            // tests to this pass so we don't emit a stray blank line in the suite header or
            // pollute the outcome with empty entries.
            if is_multi_pass && !has_tests && suite_result.warnings.is_empty() {
                continue;
            }

            // Clear the addresses and labels from previous test.
            decoder.clear_addresses();

            // Print suite header.
            if !silent {
                sh_println!()?;
                for warning in &suite_result.warnings {
                    sh_warn!("{warning}")?;
                }
                if has_tests {
                    let tests = if len > 1 { "tests" } else { "test" };
                    sh_println!("Ran {len} {tests} for {contract_name}")?;
                }
            }

            // Process individual test results, printing logs and traces when necessary.
            for (name, result) in tests {
                let test_failed = result.status.is_failure();
                let show_traces = !self.suppress_successful_traces || test_failed;
                // Trace verbosity.
                // - 0..3: nothing.
                // - 3: only display traces for failed tests.
                // - 4: also display the setup trace for failed tests.
                // - 5..: display all traces for all tests, including storage changes.
                let should_include_trace = |kind: &TraceKind| match kind {
                    TraceKind::Execution => {
                        (trace_verbosity == 3 && test_failed) || trace_verbosity >= 4
                    }
                    TraceKind::Setup => {
                        (trace_verbosity == 4 && test_failed) || trace_verbosity >= 5
                    }
                    TraceKind::Deployment => false,
                };
                let renders_trace = !silent
                    && show_traces
                    && result.traces.iter().any(|(kind, _)| should_include_trace(kind));
                let identify_addresses = always_identify_traces || renders_trace;

                if !silent {
                    sh_println!("{}", result.short_result_with_suite(name, &contract_name))?;
                    for artifact in &result.counterexample_artifacts {
                        sh_warn!("Counterexample artifact: {}", artifact.path.display())?;
                    }

                    if let TestKind::Invariant { metrics, .. } = &result.kind
                        && !metrics.is_empty()
                    {
                        let _ = sh_println!("\n{}\n", format_invariant_metrics_table(metrics));
                    }

                    // We only display logs at level 2 and above
                    if verbosity >= 2 && show_traces {
                        // We only decode logs from Hardhat and DS-style console events
                        let console_logs = decode_console_logs(&result.logs);
                        if !console_logs.is_empty() {
                            sh_println!("Logs:")?;
                            for log in console_logs {
                                sh_println!("  {log}")?;
                            }
                            sh_println!()?;
                        }
                    }
                }

                // We shouldn't break out of the outer loop directly here so that we finish
                // processing the remaining tests and print the suite summary.
                any_test_failed |= result.status == TestStatus::Failure;

                // Clear the addresses and labels from previous runs.
                decoder.clear_addresses();
                if identify_addresses {
                    decoder.labels.extend(result.labels.iter().map(|(k, v)| (*k, v.clone())));
                }

                // Identify addresses and decode traces.
                let mut decoded_traces = Vec::new();
                if identify_addresses {
                    for (kind, arena) in &mut result.traces {
                        if self.debug && !result.debug_bytecodes.is_empty() {
                            let mut local_identifier = TraceIdentifiers::new()
                                .with_local_and_bytecodes(
                                    &known_contracts,
                                    &result.debug_bytecodes,
                                );
                            decoder.identify(arena, &mut local_identifier);
                        }
                        decoder.identify(arena, &mut identifier);

                        if renders_trace && should_include_trace(kind) {
                            decoder.opcodes = self.opcodes.clone();
                            decode_trace_arena(arena, &decoder).await;
                            let rendered = match tracing.trace_depth {
                                Some(trace_depth) => {
                                    let mut arena = arena.clone();
                                    prune_trace_depth(&mut arena, trace_depth);
                                    render_trace_arena_inner(&arena, false, trace_verbosity > 4)
                                }
                                None => render_trace_arena_inner(arena, false, trace_verbosity > 4),
                            };
                            decoded_traces.push(rendered);
                        }
                    }
                }

                if !silent && show_traces && !decoded_traces.is_empty() {
                    sh_println!("Traces:")?;
                    for trace in &decoded_traces {
                        sh_println!("{trace}")?;
                    }
                }

                // Extract and display backtrace for failed tests when trace verbosity >= 3.
                // At trace verbosity 3-4 backtraces show contract/function names only.
                // At trace verbosity 5 backtraces include source file locations.
                if !silent
                    && test_failed
                    && trace_verbosity >= 3
                    && let Some((_, arena)) =
                        result.traces.iter().find(|(kind, _)| matches!(kind, TraceKind::Execution))
                {
                    let builder = backtrace_builder.get_or_insert_with(|| {
                        BacktraceBuilder::new(
                            output,
                            config.root.clone(),
                            config.parsed_libraries().ok(),
                            config.via_ir,
                        )
                    });
                    let backtrace = builder.from_traces(arena);
                    if !backtrace.is_empty() {
                        sh_println!("{}", backtrace)?;
                    }
                }

                if let Some(gas_report) = &mut gas_report {
                    gas_report.analyze(result.traces.iter().map(|(_, a)| &a.arena), &decoder).await;

                    for trace in &result.gas_report_traces {
                        decoder.clear_addresses();

                        // Re-execute setup and deployment traces to collect identities created in
                        // setUp and constructor.
                        for (kind, arena) in &result.traces {
                            if !matches!(kind, TraceKind::Execution) {
                                decoder.identify_scoped(arena, &mut identifier);
                            }
                        }

                        for arena in trace {
                            decoder.identify_scoped(arena, &mut identifier);
                            gas_report.analyze([arena], &decoder).await;
                        }
                    }
                }

                if shell::is_json()
                    && let Some(trace_depth) = tracing.trace_depth
                {
                    for (_, arena) in &mut result.traces {
                        *arena = trace_arena_at_depth(arena, trace_depth);
                    }
                }
                // Clear memory.
                result.gas_report_traces = Default::default();

                // Collect and merge gas snapshots.
                for (group, new_snapshots) in &result.gas_snapshots {
                    gas_snapshots.entry(group.clone()).or_default().extend(new_snapshots.clone());
                }
            }

            if !gas_snapshots.is_empty() {
                self.check_and_write_gas_snapshots(&config, &gas_snapshots)?;
            }

            // Print suite summary.
            if !silent && has_tests {
                sh_println!("{}", suite_result.summary())?;
            }

            // Add the suite result to the outcome.
            outcome.results.insert(contract_name, suite_result);

            // Stop processing the remaining suites if any test failed and `fail_fast` is set.
            if self.fail_fast && any_test_failed {
                break;
            }
        }
        let regressions =
            self.emit_symbolic_regressions(&config, &known_contracts, &mut outcome.results)?;
        if !silent {
            for regression in regressions {
                sh_warn!(
                    "Regression test: {} (from {})",
                    regression.path.display(),
                    regression.artifact.display()
                )?;
            }
        }
        outcome.last_run_decoder = Some(decoder);
        let duration = timer.elapsed();

        trace!(target: "forge::test", len=outcome.results.len(), %any_test_failed, "done with results");

        if let Some(gas_report) = gas_report {
            let finalized = gas_report.finalize();
            sh_println!("{finalized}")?;
            outcome.gas_report = Some(finalized);
        }

        if !is_multi_pass {
            self.print_summary(&outcome, duration)?;
        }

        // Keep the receiver alive only when its queued results are needed for the JSON file.
        let json_results_rx = self.json_file.is_some().then_some(rx);

        // Reattach the task.
        match handle.await {
            Ok(result) => {
                let runner = result?;
                outcome.known_contracts = Some(runner.known_contracts);
            }
            Err(e) => match e.try_into_panic() {
                Ok(payload) => std::panic::resume_unwind(payload),
                Err(e) => return Err(e.into()),
            },
        }

        // Include suites that completed after fail-fast stopped console output in the JSON file.
        if let Some(rx) = json_results_rx {
            let mut results = outcome.results.clone();
            for (contract_name, suite_result) in rx.try_iter() {
                if is_multi_pass
                    && suite_result.test_results.is_empty()
                    && suite_result.warnings.is_empty()
                {
                    continue;
                }
                results.insert(contract_name, suite_result);
            }
            outcome.json_file_results = Some(results);
        }

        // Persist test run failures to enable replaying.
        persist_run_failures(&config, &outcome);

        Ok(outcome)
    }

    /// Compares the collected gas snapshots against the ones on disk and writes them back,
    /// depending on `--gas-snapshot-check` / `--gas-snapshot-emit` and their config defaults.
    ///
    /// The CLI flags override the config and environment, so `--gas-snapshot-check=false` disables
    /// a check enabled in the config. Exits with code 1 if differences are found.
    fn check_and_write_gas_snapshots(
        &self,
        config: &Config,
        gas_snapshots: &BTreeMap<String, BTreeMap<String, String>>,
    ) -> Result<()> {
        if self.gas_snapshot_check.unwrap_or(config.gas_snapshot_check) {
            let mut differences_found = false;
            for (group, snapshots) in gas_snapshots {
                let path = config.snapshots.join(format!("{group}.json"));
                // If the snapshot file doesn't exist, we can't compare so we skip.
                if !path.exists() {
                    continue;
                }
                let previous_snapshots: BTreeMap<String, String> =
                    fs::read_json_file(&path).expect("Failed to read snapshots from disk");
                let diff = snapshots
                    .iter()
                    .filter_map(|(k, v)| {
                        previous_snapshots
                            .get(k)
                            .filter(|previous| *previous != v)
                            .map(|p| (k, p, v))
                    })
                    .collect::<Vec<_>>();
                if diff.is_empty() {
                    continue;
                }
                let _ = sh_eprintln!(
                    "{}",
                    format!("\n[{group}] Failed to match snapshots:").red().bold()
                );
                for (key, previous_snapshot, snapshot) in diff {
                    let _ = sh_eprintln!(
                        "{}",
                        format!("- [{key}] {previous_snapshot} → {snapshot}").red()
                    );
                }
                differences_found = true;
            }
            if differences_found {
                sh_eprintln!()?;
                bail!("Snapshots differ from previous run");
            }
        }

        if self.gas_snapshot_emit.unwrap_or(config.gas_snapshot_emit) {
            fs::create_dir_all(&config.snapshots)?;
            for (group, snapshots) in gas_snapshots {
                fs::write_pretty_json_file(
                    &config.snapshots.join(format!("{group}.json")),
                    &snapshots,
                )
                .expect("Failed to write gas snapshots to disk");
            }
        }
        Ok(())
    }

    /// Returns the flattened [`FilterArgs`] arguments merged with [`Config`].
    /// Loads and applies filter from file if only last test run failures performed.
    pub fn filter(&self, config: &Config) -> Result<ProjectPathsAwareFilter> {
        let mut filter = self.filter.clone();
        let rerun_failures = if self.rerun {
            let failures = last_run_failures(config);
            filter.test_pattern = failures.test_pattern;
            failures.failures
        } else {
            None
        };
        if filter.path_pattern.is_some() {
            if self.path.is_some() {
                bail!("Can not supply both --match-path and |path|");
            }
        } else {
            filter.path_pattern = self.path.clone();
        }
        let mut filter = filter.merge_with_config(config);
        if let Some(failures) = rerun_failures {
            filter.set_rerun_failures(failures);
        }
        Ok(filter)
    }

    /// Returns whether `BuildArgs` was configured with `--watch`
    pub const fn is_watch(&self) -> bool {
        self.watch.watch.is_some()
    }

    /// Returns the [`watchexec::Config`] necessary to bootstrap a new watch loop.
    pub(crate) fn watchexec_config(&self) -> Result<watchexec::Config> {
        self.watch.watchexec_config(|| {
            let config = self.load_config()?;
            Ok([config.src, config.test])
        })
    }
}

/// Returns the names of the enabled flags.
fn enabled_flags<const N: usize>(flags: [(bool, &'static str); N]) -> Vec<&'static str> {
    flags.into_iter().filter_map(|(enabled, name)| enabled.then_some(name)).collect()
}

fn prepare_results_for_json(
    results: &mut BTreeMap<String, SuiteResult>,
    verbosity: u8,
    trace_depth: Option<usize>,
) {
    for test_result in results.values_mut().flat_map(|suite| suite.test_results.values_mut()) {
        if verbosity >= 2 {
            test_result.decoded_logs = decode_console_logs(&test_result.logs);
        } else {
            test_result.logs = Vec::new();
        }
        for (_, arena) in &mut test_result.traces {
            // Discard presentation-only decoding populated by the streaming renderer.
            for node in arena.nodes_mut() {
                node.trace.decoded = None;
                for log in &mut node.logs {
                    log.decoded = None;
                }
                for step in &mut node.trace.steps {
                    step.decoded = None;
                }
            }
            if let Some(trace_depth) = trace_depth {
                *arena = trace_arena_at_depth(arena, trace_depth);
            }
        }
    }
}

/// Bails if mutation testing could corrupt the real dependency trees.
///
/// The mutation runner symlinks dependency directories (`lib`, `node_modules`, `dependencies`)
/// into each per-mutant TempDir for performance (see `workspace::copy_project`). That isolation
/// breaks down if tests can write to those shared trees, either via `vm.writeFile` (broad
/// `fs_permissions`) or arbitrary `ffi` calls.
fn ensure_mutation_workspace_safe(config: &Config) -> Result<()> {
    if config.ffi {
        bail!(
            "Mutation testing is unsafe with `ffi = true`: per-mutant workspaces share \
             symlinked dependency directories, and arbitrary FFI commands run by tests \
             can race or corrupt the real `lib`/`node_modules`/`dependencies` trees. \
             Disable ffi in your foundry.toml to run mutation tests."
        );
    }

    // Only refuse write-capable `fs_permissions` whose path can actually reach one of the
    // symlinked dependency trees. Scoped writes (e.g. `./out`, `./snapshots`) are safe.
    let root = &config.root;
    let canonicalize_through_existing_ancestor = |path: &Path| -> PathBuf {
        let resolved = if path.is_absolute() { path.to_path_buf() } else { root.join(path) };
        if let Ok(canon) = dunce::canonicalize(&resolved) {
            return canon;
        }
        let mut missing = Vec::new();
        let mut ancestor = resolved.as_path();
        while !ancestor.exists() {
            let Some(name) = ancestor.file_name() else { break };
            missing.push(name.to_owned());
            let Some(parent) = ancestor.parent() else { break };
            ancestor = parent;
        }
        let mut canon = dunce::canonicalize(ancestor).unwrap_or_else(|_| ancestor.into());
        canon.extend(missing.iter().rev());
        canon
    };

    let shared_dep_dirs = config
        .libs
        .iter()
        .filter(|p| p.exists())
        .cloned()
        .chain(
            ["node_modules", "dependencies"]
                .into_iter()
                .map(|dep_dir| root.join(dep_dir))
                .filter(|dep_path| dep_path.is_dir()),
        )
        .map(|p| canonicalize_through_existing_ancestor(&p))
        .collect::<Vec<_>>();

    let permissions = &config.fs_permissions.permissions;
    let effective_permission = |path: &Path| -> Option<FsAccessPermission> {
        let mut max_path_len = 0;
        let mut highest_permission = FsAccessPermission::None;
        for perm in permissions {
            let permission_path = canonicalize_through_existing_ancestor(&perm.path);
            if !path.starts_with(&permission_path) {
                continue;
            }
            let path_len = permission_path.components().count();
            if path_len > max_path_len {
                max_path_len = path_len;
                highest_permission = perm.access;
            } else if path_len == max_path_len {
                highest_permission = match (highest_permission, perm.access) {
                    (FsAccessPermission::ReadWrite, _)
                    | (FsAccessPermission::Read, FsAccessPermission::Write)
                    | (FsAccessPermission::Write, FsAccessPermission::Read) => {
                        FsAccessPermission::ReadWrite
                    }
                    (FsAccessPermission::None, perm) => perm,
                    (existing_perm, _) => existing_perm,
                };
            }
        }
        (max_path_len > 0).then_some(highest_permission)
    };
    let grants_write = |path: &Path| {
        matches!(
            effective_permission(path),
            Some(FsAccessPermission::Write | FsAccessPermission::ReadWrite)
        )
    };

    let unsafe_write_paths = permissions
        .iter()
        .filter(|perm| {
            matches!(perm.access, FsAccessPermission::Write | FsAccessPermission::ReadWrite)
        })
        .filter(|perm| {
            let perm_path = canonicalize_through_existing_ancestor(&perm.path);
            shared_dep_dirs.iter().any(|dep| {
                if perm_path.starts_with(dep) {
                    grants_write(&perm_path)
                } else if dep.starts_with(&perm_path) {
                    grants_write(dep)
                } else {
                    false
                }
            })
        })
        .map(|perm| format!("  - {}", perm.path.display()))
        .collect::<Vec<_>>();
    if !unsafe_write_paths.is_empty() {
        bail!(
            "Mutation testing is unsafe with write-capable `fs_permissions` that can \
             reach the symlinked dependency trees (`lib`/`node_modules`/`dependencies`); \
             per-mutant workspaces share those trees, so `vm.writeFile` calls would race \
             against or corrupt your real dependencies. Restrict the following \
             `fs_permissions` entries to read-only or scope them away from dependency \
             paths:\n{}",
            unsafe_write_paths.join("\n")
        );
    }
    Ok(())
}

/// Builds a figment dictionary from `key => Option<value>` pairs, skipping unset values.
macro_rules! dict {
    ($($key:literal => $value:expr),* $(,)?) => {{
        let mut dict = Dict::default();
        $(if let Some(value) = $value {
            dict.insert($key.to_string(), Value::from(value));
        })*
        dict
    }};
}

/// Renders an optional path for the figment provider.
fn path_string(path: &Option<PathBuf>) -> Option<String> {
    path.as_ref().map(|path| path.to_string_lossy().to_string())
}

impl Provider for TestArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Core Build Args Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let fuzz = dict! {
            "seed" => self.fuzz_seed.map(|seed| seed.to_string()),
            "runs" => self.fuzz_runs,
            "run" => self.fuzz_run,
            "worker" => self.fuzz_worker,
            "timeout" => self.fuzz_timeout,
            "dictionary_weight" => self.fuzz_dictionary_weight,
            "max_fuzz_dictionary_addresses" => self.fuzz_dictionary_addresses.clone(),
            "max_fuzz_dictionary_values" => self.fuzz_dictionary_values.clone(),
            "max_fuzz_dictionary_literals" => self.fuzz_dictionary_literals.clone(),
            "corpus_random_sequence_weight" => self.fuzz_corpus_random_sequence_weight,
            "corpus_dir" => path_string(&self.fuzz_corpus_dir),
            "frontier_dir" => path_string(&self.fuzz_frontier_dir),
            "frontier_limit" => self.fuzz_frontier_limit,
            "payable_value_weight" => self.fuzz_payable_value_weight,
            "mutation_weight_splice" => self.fuzz_mutation_weight_splice,
            "mutation_weight_repeat" => self.fuzz_mutation_weight_repeat,
            "mutation_weight_interleave" => self.fuzz_mutation_weight_interleave,
            "mutation_weight_prefix" => self.fuzz_mutation_weight_prefix,
            "mutation_weight_suffix" => self.fuzz_mutation_weight_suffix,
            "mutation_weight_abi" => self.fuzz_mutation_weight_abi,
            "mutation_weight_cmp" => self.fuzz_mutation_weight_cmp,
        };
        let invariant = dict! {
            "runs" => self.invariant_runs_override,
            "depth" => self.invariant_depth,
            "min_depth" => self.invariant_min_depth,
            "depth_mode" => self.invariant_depth_mode.map(Value::serialize).transpose()?,
            "workers" => self.invariant_workers.map(Value::serialize).transpose()?,
            "dictionary_weight" => self.invariant_dictionary_weight,
            "max_fuzz_dictionary_addresses" => self.invariant_dictionary_addresses.clone(),
            "max_fuzz_dictionary_values" => self.invariant_dictionary_values.clone(),
            "max_fuzz_dictionary_literals" => self.invariant_dictionary_literals.clone(),
            "corpus_random_sequence_weight" => self.invariant_corpus_random_sequence_weight,
            "corpus_random_sequence_weight_configured" =>
                self.invariant_corpus_random_sequence_weight.map(|_| true),
            "corpus_dir" => path_string(&self.invariant_corpus_dir),
            "frontier_dir" => path_string(&self.invariant_frontier_dir),
            "frontier_limit" => self.invariant_frontier_limit,
            "payable_value_weight" => self.invariant_payable_value_weight,
            "timeout" => self.invariant_timeout_override,
            "mutation_weight_splice" => self.invariant_mutation_weight_splice,
            "mutation_weight_repeat" => self.invariant_mutation_weight_repeat,
            "mutation_weight_interleave" => self.invariant_mutation_weight_interleave,
            "mutation_weight_prefix" => self.invariant_mutation_weight_prefix,
            "mutation_weight_suffix" => self.invariant_mutation_weight_suffix,
            "mutation_weight_abi" => self.invariant_mutation_weight_abi,
            "mutation_weight_cmp" => self.invariant_mutation_weight_cmp,
        };
        let symbolic = dict! {
            "enabled" => self.symbolic.then_some(true),
            "seed_corpus" => self.symbolic_seed_corpus.then_some(true),
            "use_fuzz_corpus" => self.symbolic_use_fuzz_corpus.then_some(true),
            "corpus_seed_limit" => self.symbolic_corpus_seed_limit,
            "use_fuzz_frontiers" => self.symbolic_use_fuzz_frontiers.then_some(true),
            "frontier_limit" => self.symbolic_frontier_limit,
            "frontier_ids" => self.symbolic_frontier_ids.clone(),
            "frontier_pcs" => self.symbolic_frontier_pcs.clone(),
            "frontier_selectors" => self.symbolic_frontier_selectors.clone(),
            "solver" => self.symbolic_solver.clone(),
            "solver_command" => self.symbolic_solver_command.clone(),
            "solver_portfolio" => self.symbolic_solver_portfolio.clone(),
            "timeout" => self.symbolic_timeout,
            "loop" => self.symbolic_loop,
            "depth" => self.symbolic_depth,
            "width" => self.symbolic_width,
            "max_depth" => self.symbolic_max_depth,
            "max_paths" => self.symbolic_max_paths,
            "invariant_depth" => self.symbolic_invariant_depth,
            "max_solver_queries" => self.symbolic_max_solver_queries,
            "default_dynamic_length" => self.symbolic_default_dynamic_length,
            "max_dynamic_length" => self.symbolic_max_dynamic_length,
            "array_lengths" => self.symbolic_array_lengths.clone(),
            "max_calldata_bytes" => self.symbolic_max_calldata_bytes,
            "symbolic_call_targets" => self.symbolic_call_targets.then_some(true),
            "dump_smt" => self.symbolic_dump_smt.then_some(true),
            "storage_layout" => self.symbolic_storage_layout.clone(),
        };
        let mutation = dict! {
            "timeout" => self.mutation_timeout,
            "optimizer_runs" => self.mutation_optimizer_runs,
            "via_ir" => self.mutation_via_ir,
        };

        let mut dict = dict! {
            "fuzz" => Some(fuzz),
            "invariant" => (!invariant.is_empty()).then_some(invariant),
            "symbolic" => Some(symbolic),
            "etherscan_api_key" =>
                self.etherscan_api_key.as_ref().filter(|s| !s.trim().is_empty()).cloned(),
            "show_progress" => self.show_progress.then_some(true),
        };
        // Mutation-testing CLI overrides
        if !mutation.is_empty() {
            dict.insert("mutation".to_string(), mutation.into());
        }
        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

fn parse_opcode(s: &str) -> Result<OpCode, String> {
    OpCode::parse(s).ok_or_else(|| format!("invalid opcode: {s}"))
}

const fn apply_mutation_compiler_overrides(config: &mut Config) {
    if let Some(optimizer_runs) = config.mutation.optimizer_runs {
        let default_optimizer_settings =
            matches!(config.optimizer, Some(false)) && matches!(config.optimizer_runs, Some(200));
        config.optimizer_runs = Some(optimizer_runs as usize);
        if default_optimizer_settings {
            config.optimizer = None;
        }
        config.normalize_optimizer_settings();
    }
    if let Some(via_ir) = config.mutation.via_ir {
        config.via_ir = via_ir;
    }
}

/// Returns the deployable contracts in `output` that match `filter`, with project-relative ids.
fn matching_test_contracts<'a>(
    output: &'a ProjectCompileOutput,
    config: &'a Config,
    matcher: &'a TestFunctionMatcher<'a>,
    filter: &'a ProjectPathsAwareFilter,
) -> impl Iterator<Item = (ArtifactId, &'a ConfigurableContractArtifact, &'a JsonAbi)> + 'a {
    output.artifact_ids().filter_map(move |(id, artifact)| {
        let abi = artifact.abi.as_ref()?;
        let id = id.with_stripped_file_prefixes(&config.root);
        let deployable = abi.constructor.as_ref().is_none_or(|c| c.inputs.is_empty());
        (deployable && matcher.matches_contract(filter, &id, abi)).then_some((id, artifact, abi))
    })
}

/// Lists all matching tests without building a runner.
fn list_from_output(
    output: &ProjectCompileOutput,
    config: &Config,
    inline_config: &InlineConfig,
    filter: &ProjectPathsAwareFilter,
    fuzz_only: bool,
    symbolic_artifact_replay: Option<&SymbolicArtifactReplayConfig>,
) -> Result<TestOutcome> {
    let matcher = TestFunctionMatcher::new(config, inline_config, symbolic_artifact_replay);
    let mut results = BTreeMap::<String, BTreeMap<String, Vec<String>>>::new();
    for (id, _, abi) in matching_test_contracts(output, config, &matcher, filter) {
        let identifier = id.identifier();
        let generated_symbolic_regression = is_generated_symbolic_regression_contract(abi);
        let tests = abi
            .functions()
            .filter(|func| {
                let kind =
                    matcher.test_function_kind(&identifier, func, generated_symbolic_regression);
                (!fuzz_only
                    || matches!(
                        kind,
                        TestFunctionKind::FuzzTest { .. } | TestFunctionKind::InvariantTest
                    ))
                    && filter.matches_test_function_kind_in_contract(&identifier, func, kind)
            })
            .map(|func| func.name.clone())
            .collect::<Vec<_>>();
        if !tests.is_empty() {
            results.entry(id.source.display().to_string()).or_default().insert(id.name, tests);
        }
    }

    if shell::is_json() {
        sh_println!("{}", serde_json::to_string(&results)?)?;
    } else {
        for (file, contracts) in &results {
            sh_println!("{file}")?;
            for (contract, tests) in contracts {
                sh_println!("  {contract}")?;
                sh_println!("    {}\n", tests.join("\n    "))?;
            }
        }
    }
    Ok(TestOutcome::empty(None, false))
}

fn matching_fuzz_replay_targets(
    output: &ProjectCompileOutput,
    config: &Config,
    inline_config: &InlineConfig,
    filter: &ProjectPathsAwareFilter,
    selector: &[u8],
) -> Result<Vec<(String, String)>> {
    let matcher = TestFunctionMatcher::new(config, inline_config, None);
    let mut targets = Vec::new();
    for (id, artifact, abi) in matching_test_contracts(output, config, &matcher, filter) {
        let has_creation_code =
            artifact.get_bytecode_object().is_some_and(|object| match object.as_ref() {
                BytecodeObject::Bytecode(bytecode) => !bytecode.is_empty(),
                BytecodeObject::Unlinked(_) => true,
            });
        if !has_creation_code {
            continue;
        }
        let contract = id.identifier();
        let generated_symbolic_regression = is_generated_symbolic_regression_contract(abi);
        for func in abi.functions() {
            let kind = matcher.test_function_kind(&contract, func, generated_symbolic_regression);
            if !matches!(kind, TestFunctionKind::FuzzTest { .. })
                || !filter.matches_test_function_kind_in_contract(&contract, func, kind)
            {
                continue;
            }
            let function_config = inline_config_for(config, inline_config, &contract, Some(func))?;
            if matches!(
                effective_test_function_kind(kind, &function_config, func),
                TestFunctionKind::FuzzTest { .. }
            ) && func.selector() == selector
            {
                targets.push((contract.clone(), func.signature()));
            }
        }
    }
    Ok(targets)
}

/// Merges `other` into `base` by extending suite results.
///
/// For suites that appear in both, test results are combined (function-level pass routing ensures
/// each function appears in exactly one pass, so there are no key conflicts in practice).
fn merge_outcomes(base: &mut TestOutcome, mut other: TestOutcome) {
    if let Some(other_results) = other.json_file_results.take() {
        let base_results = base.json_file_results.get_or_insert_with(|| base.results.clone());
        merge_suite_results(base_results, other_results);
    }
    merge_suite_results(&mut base.results, other.results);
    if let Some(decoder) = other.last_run_decoder {
        base.last_run_decoder = Some(decoder);
    }
}

fn merge_suite_results(
    base: &mut BTreeMap<String, SuiteResult>,
    other: BTreeMap<String, SuiteResult>,
) {
    for (suite_id, other_suite) in other {
        if let Some(base_suite) = base.get_mut(&suite_id) {
            base_suite.test_results.extend(other_suite.test_results);
            base_suite.warnings.extend(other_suite.warnings);
            base_suite.duration = base_suite.duration.max(other_suite.duration);
        } else {
            base.insert(suite_id, other_suite);
        }
    }
}

fn collect_matching_debug_tests(
    matching_tests: &BTreeMap<String, BTreeMap<String, Vec<String>>>,
) -> Vec<RerunFailure> {
    matching_tests
        .iter()
        .flat_map(|(source, contracts)| {
            contracts.iter().flat_map(move |(contract, tests)| {
                tests.iter().map(move |test| RerunFailure {
                    contract: format!("{source}:{contract}"),
                    test: test.clone(),
                })
            })
        })
        .collect()
}

fn format_matching_debug_tests(matching_tests: &[RerunFailure]) -> String {
    if matching_tests.is_empty() {
        return String::new();
    }
    let mut output = String::from("\n\nMatching tests:");
    for test in matching_tests.iter().take(DEBUGGER_MATCHING_TESTS_DISPLAY_LIMIT) {
        write!(output, "\n  {}.{}", test.contract, test.test).unwrap();
    }
    if matching_tests.len() > DEBUGGER_MATCHING_TESTS_DISPLAY_LIMIT {
        write!(
            output,
            "\n  ... and {} more",
            matching_tests.len() - DEBUGGER_MATCHING_TESTS_DISPLAY_LIMIT
        )
        .unwrap();
    }
    output
}

struct LastRunFailures {
    test_pattern: Option<Regex>,
    failures: Option<Vec<RerunFailure>>,
}

/// Load persisted filter (with last test run failures) from file.
fn last_run_failures(config: &Config) -> LastRunFailures {
    let Ok(filter) = fs::read_to_string(&config.test_failures_file) else {
        return LastRunFailures { test_pattern: None, failures: None };
    };

    if let Ok(failures) = serde_json::from_str::<RerunFailures>(&filter) {
        if failures.failures.is_empty() {
            return LastRunFailures { test_pattern: None, failures: None };
        }
        let test_pattern = failures
            .failures
            .iter()
            .map(|failure| regex::escape(&failure.test))
            .collect::<Vec<_>>()
            .join("|");
        let test_pattern = Regex::new(&test_pattern).ok();
        return LastRunFailures { test_pattern, failures: Some(failures.failures) };
    }

    // Legacy format: a plain regex.
    let test_pattern = Regex::new(&filter)
        .inspect_err(|e| {
            _ = sh_warn!("failed to parse test filter from {:?}: {e}", config.test_failures_file)
        })
        .ok();
    LastRunFailures { test_pattern, failures: None }
}

/// Persist filter with last test run failures (only if there's any failure).
fn persist_run_failures(config: &Config, outcome: &TestOutcome) {
    if outcome.failed() > 0 && fs::create_file(&config.test_failures_file).is_ok() {
        let failures = outcome
            .results
            .iter()
            .flat_map(|(contract, suite)| {
                suite.test_results.iter().filter(|(_, result)| result.status.is_failure()).flat_map(
                    move |(test_name, test_result)| {
                        rerun_filter_matches(test_name, test_result)
                            .map(move |test| RerunFailure { contract: contract.clone(), test })
                    },
                )
            })
            .collect::<Vec<_>>();

        if let Ok(output) = serde_json::to_string(&RerunFailures { version: 1, failures }) {
            let _ = fs::write(&config.test_failures_file, output);
        }
    }
}

/// Returns the rerun keys of a failed test: its failed invariant predicates, or the test name when
/// no predicate failed.
fn rerun_filter_matches<'a>(
    test_name: &'a str,
    test_result: &'a TestResult,
) -> impl Iterator<Item = String> + 'a {
    let predicate_failures =
        test_result.invariant_failures.iter().filter_map(|failure| failure.predicate_name());
    let has_predicate_failures = predicate_failures.clone().next().is_some();
    let fallback = test_name.is_any_test().then(|| test_name.split('(').next()).flatten();
    predicate_failures
        .chain(fallback.into_iter().filter(move |_| !has_predicate_failures))
        .map(str::to_owned)
}

/// Generate test report in JUnit XML report format.
fn junit_xml_report(results: &BTreeMap<String, SuiteResult>, verbosity: u8) -> Report {
    let mut total_duration = Duration::default();
    let mut junit_report = Report::new("Test run");
    junit_report.set_timestamp(Utc::now());
    for (suite_name, suite_result) in results {
        let mut test_suite = TestSuite::new(suite_name);
        total_duration += suite_result.duration;
        test_suite.set_time(suite_result.duration);
        test_suite.set_system_out(suite_result.summary());
        for (test_name, test_result) in &suite_result.test_results {
            add_junit_test_cases(&mut test_suite, test_name, test_result, verbosity);
        }
        junit_report.add_test_suite(test_suite);
    }
    junit_report.set_time(total_duration);
    junit_report
}

/// Adds JUnit test cases for a test result.
///
/// Invariant campaigns are expanded into per-predicate and per-handler cases so CI can report
/// contract-level execution without losing failure attribution.
fn add_junit_test_cases(
    test_suite: &mut TestSuite,
    test_name: &str,
    test_result: &TestResult,
    verbosity: u8,
) {
    let output = JunitOutput::new(test_result, verbosity);
    let expanded_invariant = test_result.kind.is_invariant()
        && (!test_result.invariant_predicate_results.is_empty()
            || !test_result.invariant_handler_failures.is_empty());

    if !expanded_invariant {
        add_junit_test_case(
            test_suite,
            test_name,
            test_result.status,
            test_result.reason.as_deref(),
            test_result.duration,
            output.system_out(test_result, test_name),
        );
        return;
    }

    let mut add_expanded_case =
        |name: &str,
         status: TestStatus,
         reason: Option<&str>,
         counterexample: Option<&CounterExample>| {
            add_junit_test_case(
                test_suite,
                name,
                status,
                reason,
                test_result.duration,
                output.case_system_out(status, reason, name, counterexample),
            );
        };

    if test_result.invariant_predicate_results.is_empty() {
        let failure = test_result.invariant_failures.first();
        let status = if failure.is_some() { TestStatus::Failure } else { TestStatus::Success };
        add_expanded_case(
            test_name,
            status,
            failure.map(|failure| failure.reason()),
            failure.and_then(|failure| failure.counterexample()),
        );
    } else {
        for predicate in &test_result.invariant_predicate_results {
            let failure = test_result
                .invariant_failures
                .iter()
                .find(|failure| failure.name() == predicate.name.as_str());
            add_expanded_case(
                &format!("{}()", predicate.name),
                predicate.status,
                predicate.reason.as_deref(),
                failure.and_then(|failure| failure.counterexample()),
            );
        }
    }

    for failure in &test_result.invariant_handler_failures {
        add_expanded_case(
            &format!("handler {}", failure.name()),
            TestStatus::Failure,
            Some(failure.reason()),
            failure.counterexample(),
        );
    }
}

/// Adds a single JUnit test case to the suite.
fn add_junit_test_case(
    test_suite: &mut TestSuite,
    test_name: &str,
    status: TestStatus,
    message: Option<&str>,
    duration: Duration,
    system_out: String,
) {
    let mut test_status = match status {
        TestStatus::Success => TestCaseStatus::success(),
        TestStatus::Failure => TestCaseStatus::non_success(NonSuccessKind::Failure),
        TestStatus::Skipped => TestCaseStatus::skipped(),
    };
    if let Some(message) = message {
        test_status.set_message(message);
    }
    let mut test_case = TestCase::new(test_name, test_status);
    test_case.set_time(duration);
    test_case.set_system_out(system_out);
    test_suite.add_test_case(test_case);
}

/// Helper for assembling JUnit output strings.
struct JunitOutput {
    result_report: TestKindReport,
    logs: Option<Vec<String>>,
}

impl JunitOutput {
    fn new(test_result: &TestResult, verbosity: u8) -> Self {
        Self {
            result_report: test_result.kind.report(),
            logs: (verbosity >= 2 && !test_result.logs.is_empty())
                .then(|| decode_console_logs(&test_result.logs)),
        }
    }

    /// Renders the suite-level `system-out` payload.
    fn system_out(&self, test_result: &TestResult, test_name: &str) -> String {
        let mut sys_out = format!("{test_result} {test_name} {}", self.result_report);
        self.append_logs(&mut sys_out);
        sys_out
    }

    /// Renders the case-level `system-out` payload.
    fn case_system_out(
        &self,
        status: TestStatus,
        message: Option<&str>,
        test_name: &str,
        counterexample: Option<&CounterExample>,
    ) -> String {
        let mut sys_out = match (status, message) {
            (TestStatus::Success, _) => "[PASS]".to_string(),
            (TestStatus::Failure, message) => format!("[FAIL: {}]", message.unwrap_or_default()),
            (TestStatus::Skipped, Some(message)) => format!("[SKIP: {message}]"),
            (TestStatus::Skipped, None) => "[SKIP]".to_string(),
        };
        write!(sys_out, " {test_name} {}", self.result_report).unwrap();
        if let Some(CounterExample::Sequence(original, sequence)) = counterexample {
            writeln!(sys_out, "\n\t[Sequence] (original: {original}, shrunk: {})", sequence.len())
                .unwrap();
            for ex in sequence {
                writeln!(sys_out, "{ex}").unwrap();
            }
        }
        self.append_logs(&mut sys_out);
        sys_out
    }

    /// Appends captured console logs to the output payload.
    fn append_logs(&self, sys_out: &mut String) {
        if let Some(logs) = &self.logs {
            write!(sys_out, "\\nLogs:\\n").unwrap();
            for log in logs {
                write!(sys_out, "  {log}\\n").unwrap();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses `args` with the given environment variables set, restoring the previous values.
    fn parse_with_env(vars: &[(&str, &str)], args: &[&str]) -> TestArgs {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap();
        let previous = vars.iter().map(|(name, _)| std::env::var_os(name)).collect::<Vec<_>>();
        for (name, value) in vars {
            unsafe { std::env::set_var(name, value) };
        }
        let parsed =
            TestArgs::try_parse_from(["foundry-cli"].into_iter().chain(args.iter().copied()));
        for ((name, _), previous) in vars.iter().zip(previous) {
            match previous {
                Some(previous) => unsafe { std::env::set_var(name, previous) },
                None => unsafe { std::env::remove_var(name) },
            }
        }
        parsed.unwrap()
    }

    #[test]
    fn parses_flags_without_cli_coverage() {
        assert!(TestArgs::parse_from(["foundry-cli", "-vw"]).watch.watch.is_some());
        assert!(TestArgs::parse_from(["foundry-cli", "--compact-labels"]).tracing.compact_labels);
        // <https://github.com/foundry-rs/foundry/issues/5913>
        let args =
            TestArgs::parse_from(["foundry-cli", "-vvv", "--gas-report", "--fuzz-seed", "0x10"]);
        assert!(args.fuzz_seed.is_some());
        let args = TestArgs::parse_from(["foundry-cli", "--invariant-workers", "auto"]);
        assert_eq!(args.invariant_workers, Some(InvariantWorkers::Auto));
        assert_eq!(
            figment::Figment::from(&args)
                .extract_inner::<InvariantWorkers>("invariant.workers")
                .unwrap(),
            InvariantWorkers::Auto
        );
    }

    #[test]
    fn parses_env_vars() {
        let args = parse_with_env(
            &[
                ("FOUNDRY_INVARIANT_WORKERS", "auto"),
                ("FOUNDRY_FUZZ_CORPUS_DIR", "env_fuzz_corpus"),
                ("FOUNDRY_INVARIANT_CORPUS_DIR", "env_invariant_corpus"),
            ],
            &[],
        );
        assert_eq!(args.invariant_workers, Some(InvariantWorkers::Auto));
        assert_eq!(args.fuzz_corpus_dir, Some(PathBuf::from("env_fuzz_corpus")));
        assert_eq!(args.invariant_corpus_dir, Some(PathBuf::from("env_invariant_corpus")));
    }

    #[test]
    fn showmap_override_validates_path_component_names() {
        let mut args = TestArgs::parse_from(["foundry-cli"]);
        args.set_showmap_override(ShowmapConfig {
            out_dir: PathBuf::from("showmap"),
            approach: "../outside".to_string(),
            trial: "trial".to_string(),
            per_input: false,
            domain: ShowmapDomain::Evm,
            corpus_dir: None,
            emit_files: false,
        });

        let err = args.showmap_config().unwrap_err().to_string();
        assert!(err.contains("expected a single file-name component"), "{err}");
    }

    #[test]
    fn debugger_test_candidates_preserve_exact_suite_ids() {
        let matching = BTreeMap::from([(
            "test/Counter.t.sol".to_string(),
            BTreeMap::from([(
                "CounterTest".to_string(),
                vec!["testFuzz_SetNumber(uint256)".to_string(), "test_Increment()".to_string()],
            )]),
        )]);

        let candidates = collect_matching_debug_tests(&matching);

        assert_eq!(candidates[0].contract, "test/Counter.t.sol:CounterTest");
        assert_eq!(candidates[0].test, "testFuzz_SetNumber(uint256)");
        assert_eq!(candidates[1].test, "test_Increment()");
        assert_eq!(
            format_matching_debug_tests(&candidates),
            "\n\nMatching tests:\n  test/Counter.t.sol:CounterTest.testFuzz_SetNumber(uint256)\n  test/Counter.t.sol:CounterTest.test_Increment()"
        );
    }

    #[test]
    fn fuzz_run_adapter_writes_unified_campaign_dials_sparsely() {
        let args = FuzzRunArgs::parse_from([
            "foundry-cli",
            "--runs",
            "9",
            "--timeout",
            "3",
            "--seed",
            "0x10",
            "--depth",
            "7",
            "--workers",
            "2",
            "--frontier-dir",
            "frontiers",
            "--frontier-limit",
            "17",
        ]);
        let args = TestArgs::from_fuzz_run(args);
        let figment = figment::Figment::from(&args);

        assert_eq!(figment.extract_inner::<u64>("fuzz.runs").unwrap(), 9);
        assert_eq!(figment.extract_inner::<u64>("fuzz.timeout").unwrap(), 3);
        assert_eq!(figment.extract_inner::<String>("fuzz.seed").unwrap(), "16");
        assert_eq!(figment.extract_inner::<u64>("invariant.runs").unwrap(), 9);
        assert_eq!(figment.extract_inner::<u32>("invariant.timeout").unwrap(), 3);
        assert_eq!(figment.extract_inner::<u32>("invariant.depth").unwrap(), 7);
        assert_eq!(
            figment.extract_inner::<PathBuf>("fuzz.frontier_dir").unwrap(),
            PathBuf::from("frontiers")
        );
        assert_eq!(figment.extract_inner::<usize>("fuzz.frontier_limit").unwrap(), 17);
        assert_eq!(
            figment.extract_inner::<PathBuf>("invariant.frontier_dir").unwrap(),
            PathBuf::from("frontiers")
        );
        assert_eq!(figment.extract_inner::<usize>("invariant.frontier_limit").unwrap(), 17);
        assert_eq!(
            figment.extract_inner::<InvariantWorkers>("invariant.workers").unwrap(),
            InvariantWorkers::Fixed(std::num::NonZeroUsize::new(2).unwrap())
        );
    }

    #[test]
    fn fuzz_run_adapter_writes_invariant_workers_sparsely() {
        let args = TestArgs::from_fuzz_run(FuzzRunArgs::parse_from(["foundry-cli"]));
        let figment = figment::Figment::from(&args);

        assert_eq!(args.invariant_workers, None);
        assert!(figment.extract_inner::<InvariantWorkers>("invariant.workers").is_err());
    }

    #[test]
    fn mutation_compiler_overrides_are_extracted() {
        let args = TestArgs::parse_from([
            "foundry-cli",
            "--mutate",
            "--mutation-optimizer-runs",
            "1",
            "--mutation-via-ir",
            "false",
        ]);
        let figment = figment::Figment::from(&args);
        assert_eq!(figment.extract_inner::<u32>("mutation.optimizer_runs").unwrap(), 1);
        assert!(!figment.extract_inner::<bool>("mutation.via_ir").unwrap());
    }

    #[test]
    fn mutation_compiler_overrides_update_only_mutation_config_clone() {
        let mut config = Config {
            optimizer_runs: Some(999),
            via_ir: true,
            mutation: foundry_config::MutationConfig {
                optimizer_runs: Some(1),
                via_ir: Some(false),
                ..Default::default()
            },
            ..Default::default()
        };

        apply_mutation_compiler_overrides(&mut config);

        assert_eq!(config.optimizer_runs, Some(1));
        assert!(!config.via_ir);
    }

    #[test]
    fn mutation_optimizer_runs_normalize_default_optimizer_settings() {
        let mut config = Config {
            optimizer: Some(false),
            optimizer_runs: Some(200),
            mutation: foundry_config::MutationConfig {
                optimizer_runs: Some(1),
                ..Default::default()
            },
            ..Default::default()
        };

        apply_mutation_compiler_overrides(&mut config);

        assert_eq!(config.optimizer, Some(true));
        assert_eq!(config.optimizer_runs, Some(1));
    }

    #[test]
    fn auto_fuzz_corpus_defaults_to_cache_failure_layout() {
        let mut args = TestArgs::parse_from(["foundry-cli"]);
        args.enable_fuzz_only_with_auto_fuzz_corpus();
        let mut config = Config::default();

        args.apply_test_config_overrides(&mut config);

        assert_eq!(
            config.fuzz.corpus.corpus_dir,
            Some(config.cache_path.join(AUTO_FUZZ_FAILURE_DIR).join(AUTO_CORPUS_DIR))
        );
        assert_eq!(config.invariant.corpus.corpus_dir, None);
    }

    #[test]
    fn auto_fuzz_corpus_uses_configured_failure_persist_dirs() {
        let mut args = TestArgs::parse_from(["foundry-cli"]);
        args.enable_fuzz_only_with_auto_fuzz_corpus();
        let mut config = Config::default();
        config.fuzz.failure_persist_dir = Some(PathBuf::from("custom_fuzz_failures"));

        args.apply_test_config_overrides(&mut config);

        assert_eq!(
            config.fuzz.corpus.corpus_dir,
            Some(PathBuf::from("custom_fuzz_failures").join(AUTO_CORPUS_DIR))
        );
        assert_eq!(config.invariant.corpus.corpus_dir, None);
    }

    #[test]
    fn auto_fuzz_corpus_preserves_configured_corpus_dirs() {
        let mut args = TestArgs::parse_from(["foundry-cli"]);
        args.enable_fuzz_only_with_auto_fuzz_corpus();
        let mut config = Config::default();
        config.fuzz.corpus.corpus_dir = Some(PathBuf::from("configured_fuzz_corpus"));
        config.invariant.corpus.corpus_dir = Some(PathBuf::from("configured_invariant_corpus"));

        args.apply_test_config_overrides(&mut config);

        assert_eq!(config.fuzz.corpus.corpus_dir, Some(PathBuf::from("configured_fuzz_corpus")));
        assert_eq!(
            config.invariant.corpus.corpus_dir,
            Some(PathBuf::from("configured_invariant_corpus"))
        );
    }

    #[test]
    fn fuzz_only_does_not_enable_auto_fuzz_corpus() {
        let mut args = TestArgs::parse_from(["foundry-cli"]);
        args.enable_fuzz_only();
        let mut config = Config::default();

        args.apply_test_config_overrides(&mut config);

        assert_eq!(config.fuzz.corpus.corpus_dir, None);
        assert_eq!(config.invariant.corpus.corpus_dir, None);
    }

    #[test]
    fn debug_brutalize_includes_storage_layout_output() {
        let args = TestArgs::parse_from(["foundry-cli", "--debug", "--brutalize"]);
        let mut config = Config::default();

        args.apply_test_config_overrides(&mut config);

        assert_eq!(config.extra_output, vec![ContractOutputSelection::StorageLayout]);
    }

    #[test]
    fn fuzz_and_invariant_config_flags() {
        let args = TestArgs::parse_from([
            "foundry-cli",
            "--fuzz-dictionary-weight",
            "35",
            "--fuzz-dictionary-addresses",
            "max",
            "--fuzz-dictionary-values",
            "1234",
            "--fuzz-dictionary-literals",
            "4321",
            "--fuzz-corpus-random-sequence-weight",
            "55",
            "--fuzz-corpus-dir",
            "fuzz_corpus",
            "--fuzz-frontier-dir",
            "fuzz_frontiers",
            "--fuzz-frontier-limit",
            "7",
            "--fuzz-payable-value-weight",
            "12",
            "--fuzz-mutation-weight-splice",
            "4",
            "--fuzz-mutation-weight-abi",
            "3",
            "--fuzz-mutation-weight-cmp",
            "5",
            "--symbolic-use-fuzz-frontiers",
            "--symbolic-frontier-limit",
            "3",
            "--symbolic-frontier-ids",
            "4,9",
            "--symbolic-frontier-pcs",
            "123,456",
            "--symbolic-frontier-selectors",
            "0x12345678,deadbeef",
            "--invariant-depth",
            "300",
            "--invariant-min-depth",
            "20",
            "--invariant-depth-mode",
            "random",
            "--invariant-workers",
            "4",
            "--invariant-dictionary-weight",
            "45",
            "--invariant-dictionary-addresses",
            "8765",
            "--invariant-dictionary-values",
            "max",
            "--invariant-dictionary-literals",
            "6789",
            "--invariant-corpus-random-sequence-weight",
            "25",
            "--invariant-corpus-dir",
            "invariant_corpus",
            "--invariant-payable-value-weight",
            "34",
            "--invariant-mutation-weight-splice",
            "2",
            "--invariant-mutation-weight-cmp",
            "7",
        ]);

        let config = Config::default().merge_inline_provider(&args).unwrap();
        assert_eq!(config.fuzz.dictionary.dictionary_weight, 35);
        assert_eq!(config.fuzz.dictionary.max_fuzz_dictionary_addresses, usize::MAX);
        assert_eq!(config.fuzz.dictionary.max_fuzz_dictionary_values, 1234);
        assert_eq!(config.fuzz.dictionary.max_fuzz_dictionary_literals, 4321);
        assert_eq!(config.fuzz.corpus.corpus_random_sequence_weight, 55);
        assert_eq!(config.fuzz.corpus.corpus_dir, Some(PathBuf::from("fuzz_corpus")));
        assert_eq!(config.fuzz.corpus.frontier_dir, Some(PathBuf::from("fuzz_frontiers")));
        assert_eq!(config.fuzz.corpus.frontier_limit, 7);
        assert_eq!(config.fuzz.corpus.payable_value_weight, 12);
        assert_eq!(config.fuzz.corpus.mutation_weights.mutation_weight_splice, 4);
        assert_eq!(config.fuzz.corpus.mutation_weights.mutation_weight_abi, 3);
        assert_eq!(config.fuzz.corpus.mutation_weights.mutation_weight_cmp, 5);
        assert!(config.symbolic.use_fuzz_frontiers);
        assert_eq!(config.symbolic.frontier_limit, 3);
        assert_eq!(config.symbolic.frontier_ids, vec![4, 9]);
        assert_eq!(config.symbolic.frontier_pcs, vec![123, 456]);
        assert_eq!(config.symbolic.frontier_selectors, vec!["0x12345678", "deadbeef"]);
        assert_eq!(config.invariant.depth, 300);
        assert_eq!(config.invariant.min_depth, 20);
        assert_eq!(config.invariant.depth_mode, InvariantDepthMode::Random);
        assert_eq!(config.invariant.dictionary.dictionary_weight, 45);
        assert_eq!(config.invariant.dictionary.max_fuzz_dictionary_addresses, 8765);
        assert_eq!(config.invariant.dictionary.max_fuzz_dictionary_values, usize::MAX);
        assert_eq!(config.invariant.dictionary.max_fuzz_dictionary_literals, 6789);
        assert_eq!(config.invariant.corpus.corpus_random_sequence_weight, 25);
        assert_eq!(config.invariant.corpus.corpus_dir, Some(PathBuf::from("invariant_corpus")));
        assert!(config.invariant.corpus_random_sequence_weight_configured);
        assert_eq!(
            config.invariant.workers,
            InvariantWorkers::Fixed(std::num::NonZeroUsize::new(4).unwrap())
        );
        assert!(config.invariant.workers_configured);
        assert_eq!(config.invariant.corpus.payable_value_weight, 34);
        assert_eq!(config.invariant.corpus.mutation_weights.mutation_weight_splice, 2);
        assert_eq!(config.invariant.corpus.mutation_weights.mutation_weight_cmp, 7);
    }
}
