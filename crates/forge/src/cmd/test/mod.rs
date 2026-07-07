use super::{install, watch::WatchArgs};
use crate::{
    MultiContractRunner, MultiContractRunnerBuilder, brutalizer,
    decode::decode_console_logs,
    gas_report::GasReport,
    multi_runner::{
        FuzzMinimizeConfig, FuzzMinimizeEdgeIndices, FuzzMinimizeObservation, MultiNetworkConfig,
        ShowmapConfig, SymbolicArtifactReplayConfig, TestFunctionMatcher,
        is_generated_symbolic_regression_contract,
    },
    mutation::{MutationRunConfig, run_mutation_testing},
    result::{
        SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA, SuiteResult, SymbolicCounterexampleArtifact,
        SymbolicReplayStatus, TestKindReport, TestOutcome, TestResult, TestStatus,
    },
    symbolic_regression::{
        SymbolicRegressionConfig, attach_symbolic_regressions_to_suites,
        collect_symbolic_artifacts_from_suites, emit_symbolic_regressions,
    },
    traces::{
        CallTraceDecoderBuilder, InternalTraceMode, TraceKind,
        debug::{ContractSources, DebugTraceIdentifier},
        decode_trace_arena, folded_stack_trace,
        identifier::SignaturesIdentifier,
        speedscope,
    },
    workspace,
};
use alloy_primitives::U256;
use chrono::Utc;
use clap::{Parser, ValueEnum, ValueHint};
use eyre::{Context, OptionExt, Result, bail};
use foundry_cli::{
    opts::{BuildOpts, EvmArgs, GlobalArgs},
    utils::{self, LoadConfig},
};
use foundry_common::{
    EmptyTestFilter, TestFilter, TestFunctionExt, TestFunctionKind,
    compile::{ProjectCompiler, compile_abi_project},
    fs, sh_status, sh_warn, shell,
};
use foundry_compilers::{
    ProjectCompileOutput,
    artifacts::Libraries,
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
#[cfg(feature = "optimism")]
use foundry_evm::core::evm::OpEvmNetwork;
use foundry_evm::{
    core::evm::{
        BlockEnvFor, EthEvmNetwork, FoundryEvmNetwork, SpecFor, TempoEvmNetwork, TxEnvFor,
    },
    executors::ShowmapDomain,
    fuzz::{BasicTxDetails, CounterExample},
    hardforks::TempoHardfork,
    opts::EvmOpts,
    traces::{backtrace::BacktraceBuilder, identifier::TraceIdentifiers, prune_trace_depth},
};
use rand::Rng;
use regex::Regex;
use revm::context::Transaction;
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
use crate::{
    result::TestKind,
    runner::{count_runnable_invariant_campaign_anchors, function_matches_network_pass},
    traces::render_trace_arena_inner,
};
pub use filter::{FilterArgs, ProjectPathsAwareFilter};
use filter::{RerunFailure, RerunFailures};
use quick_junit::{NonSuccessKind, Report, TestCase, TestCaseStatus, TestSuite};
use summary::{TestSummaryReport, format_invariant_metrics_table};

const DEBUGGER_MATCHING_TESTS_DISPLAY_LIMIT: usize = 12;

// Loads project's figment and merges the build cli arguments into it
foundry_config::merge_impl_figment_convert!(TestArgs, build, evm);

fn validate_showmap_name(kind: &str, name: &str) -> Result<()> {
    let path = Path::new(name);
    if name.is_empty()
        || path.is_absolute()
        || path.components().count() != 1
        || name.contains(['/', '\\'])
        || matches!(name, "." | "..")
    {
        bail!(
            "invalid {kind} `{name}`: expected a single file-name component without path separators"
        );
    }
    Ok(())
}

fn validate_showmap_config(showmap: &ShowmapConfig) -> Result<()> {
    validate_showmap_name("showmap approach", &showmap.approach)?;
    validate_showmap_name("showmap trial", &showmap.trial)
}

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
    ) -> Result<Vec<FuzzMinimizeObservation>> {
        let observations = Arc::new(Mutex::new(Vec::new()));
        let fuzz_minimize = FuzzMinimizeConfig {
            input: sequence.into(),
            evm_edge_indices,
            observations: observations.clone(),
        };

        for pass in &self.passes {
            if pass.target_count == 0 {
                continue;
            }
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

fn replay_with_runner<FEN: FoundryEvmNetwork>(
    runner: &MultiContractRunner<FEN>,
    filter: &ProjectPathsAwareFilter,
    fuzz_minimize: FuzzMinimizeConfig,
) -> Result<()> {
    let mut runner = runner.clone();
    runner.tcfg.fuzz_minimize = Some(fuzz_minimize);
    let results = runner.test_collect(filter)?;
    for (suite, suite_result) in results {
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
}

fn fuzz_minimize_replay<FEN: FoundryEvmNetwork>(
    runner: MultiContractRunner<FEN>,
    filter: &ProjectPathsAwareFilter,
) -> FuzzMinimizeReplayPass {
    let target_count = count_fuzz_minimize_targets(&runner, filter);
    FuzzMinimizeReplayPass {
        target_count,
        replay: Box::new(move |filter, fuzz_minimize| {
            replay_with_runner(&runner, filter, fuzz_minimize)
        }),
    }
}

fn count_fuzz_minimize_targets<FEN: FoundryEvmNetwork>(
    runner: &MultiContractRunner<FEN>,
    filter: &dyn TestFilter,
) -> usize {
    runner
        .matching_contracts(filter)
        .map(|(id, contract)| {
            let contract_name = id.identifier();
            let fuzz_targets = contract
                .abi
                .functions()
                .filter(|func| func.is_fuzz_test())
                .filter(|func| filter.matches_test_function_in_contract(&contract_name, func))
                .filter(|func| {
                    function_matches_network_pass(
                        &runner.tcfg.multi_network.all_override_networks,
                        runner.tcfg.multi_network.pass_network.as_ref(),
                        runner.tcfg.inline_config.network_for(
                            &runner.tcfg.config.profile,
                            &contract_name,
                            &func.name,
                        ),
                    )
                })
                .count();
            let invariant_targets = count_runnable_invariant_campaign_anchors(
                &contract.abi,
                filter,
                crate::runner::InvariantCampaignScope {
                    config: &runner.tcfg.config,
                    inline_config: &runner.tcfg.inline_config,
                    contract_name: &contract_name,
                    all_override_networks: &runner.tcfg.multi_network.all_override_networks,
                    pass_network: runner.tcfg.multi_network.pass_network.as_ref(),
                },
            );
            fuzz_targets + invariant_targets
        })
        .sum()
}

#[derive(Clone, Copy)]
enum NetworkDispatchKind {
    Tempo,
    #[cfg(feature = "optimism")]
    Optimism,
    Eth,
}

const fn network_dispatch_kind(evm_opts: &EvmOpts) -> NetworkDispatchKind {
    if evm_opts.networks.is_tempo() {
        return NetworkDispatchKind::Tempo;
    }

    #[cfg(feature = "optimism")]
    if evm_opts.networks.is_optimism() {
        return NetworkDispatchKind::Optimism;
    }

    NetworkDispatchKind::Eth
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
    pub(crate) should_debug: bool,
    pub(crate) decode_internal: InternalTraceMode,
    pub(crate) multi_network: MultiNetworkConfig,
    pub(crate) replay_symbolic_artifact: Option<SymbolicArtifactReplayConfig>,
    pub(crate) inline_config: Arc<InlineConfig>,
}

impl TestExecutionOptions {
    pub(crate) fn default_run(inline_config: Arc<InlineConfig>) -> Self {
        Self {
            coverage: false,
            should_debug: false,
            decode_internal: InternalTraceMode::None,
            multi_network: MultiNetworkConfig::default(),
            replay_symbolic_artifact: None,
            inline_config,
        }
    }

    pub(crate) fn coverage(inline_config: Arc<InlineConfig>) -> Self {
        Self { coverage: true, ..Self::default_run(inline_config) }
    }
}

#[derive(Clone)]
struct FuzzMinimizeNetworkPassOptions {
    inline_config: Arc<InlineConfig>,
    multi_network: MultiNetworkConfig,
}

fn sources_to_compile_from_artifacts(
    config: &Config,
    test_filter: &ProjectPathsAwareFilter,
    artifacts: &ProjectCompileOutput,
    test_matcher: &TestFunctionMatcher<'_>,
) -> BTreeSet<PathBuf> {
    // `MultiContractRunner::build` strips the root prefix from artifact source paths so the
    // identifiers it constructs are project-relative. Match that here for the filter check
    // (notably for the `--rerun` failure list, which is persisted relative) but return the
    // original absolute source paths so downstream compilation can locate them.
    artifacts
        .artifact_ids()
        .filter_map(|(id, artifact)| artifact.abi.as_ref().map(|abi| (id, abi)))
        .filter(|(id, abi)| {
            if id.source.starts_with(&config.src) {
                return true;
            }
            let stripped = id.clone().with_stripped_file_prefixes(&config.root);
            test_matcher.matches_contract(test_filter, &stripped, abi)
        })
        .map(|(id, _)| id.source)
        .collect()
}

/// CLI arguments for `forge test`.
#[derive(Clone, Debug, Parser)]
#[command(next_help_heading = "Test options")]
pub struct TestArgs {
    /// Internal mode used by `forge fuzz`.
    #[arg(skip)]
    pub(crate) fuzz_only: bool,

    /// Internal showmap/replay override used by `forge fuzz replay`.
    #[arg(skip)]
    pub(crate) showmap_override: Option<ShowmapConfig>,

    /// Internal mode used by `forge fuzz replay` to replay persisted fuzz failures.
    #[arg(skip)]
    pub(crate) fuzz_failure_replay: bool,

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
    #[arg(long, conflicts_with_all = ["flamechart", "evm_profile", "json", "junit", "list"])]
    flamegraph: bool,

    /// Generate a flamechart for a single test. Implies `--decode-internal`.
    ///
    /// A flame chart shows the gas usage over time, illustrating when each function is
    /// called (execution order) and how much gas it consumes at each point in the timeline.
    #[arg(long, conflicts_with_all = ["flamegraph", "evm_profile", "json", "junit", "list"])]
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
        conflicts_with_all = ["flamegraph", "flamechart", "json", "junit", "list"]
    )]
    evm_profile: Option<EvmProfileFormat>,

    /// Don't open the profile in the browser (for `--evm-profile`).
    ///
    /// The profile is saved to disk without starting the local viewer server.
    #[arg(long, requires = "evm_profile")]
    no_open: bool,

    /// Identify internal functions in traces.
    ///
    /// This will trace internal functions and decode stack parameters.
    ///
    /// Parameters stored in memory (such as bytes or arrays) are currently decoded only when a
    /// single function is matched, similarly to `--debug`, for performance reasons.
    #[arg(long)]
    decode_internal: bool,

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
    #[arg(long, short, env = "FORGE_SUPPRESS_SUCCESSFUL_TRACES", help_heading = "Display options")]
    suppress_successful_traces: bool,

    /// Defines the depth of a trace
    #[arg(long)]
    trace_depth: Option<usize>,

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
    #[arg(long)]
    pub fuzz_input_file: Option<String>,

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

    /// Print test summary table.
    #[arg(long, help_heading = "Display options")]
    pub summary: bool,

    /// Print detailed test summary table.
    #[arg(long, help_heading = "Display options", requires = "summary")]
    pub detailed: bool,

    /// Disables the labels in the traces.
    #[arg(long, help_heading = "Display options")]
    pub disable_labels: bool,

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

    /// Builds a `ShowmapConfig` from the showmap CLI flags, if `--showmap-out` is set.
    fn showmap_config(&self) -> Result<Option<ShowmapConfig>> {
        if let Some(showmap) = self.showmap_override.clone() {
            validate_showmap_config(&showmap)?;
            return Ok(Some(showmap));
        }

        // Default trial id uses nanosecond precision so back-to-back invocations
        // don't collide and overwrite each other's output files.
        let trial = self.showmap_trial.clone().unwrap_or_else(|| {
            let ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            format!("trial-{ns}")
        });
        let Some(out_dir) = self.showmap_out.clone() else { return Ok(None) };
        let showmap = ShowmapConfig {
            out_dir,
            approach: self.showmap_approach.clone(),
            trial,
            per_input: self.showmap_per_input,
            domain: self.showmap_domain.into(),
            corpus_dir: self.showmap_corpus_dir.clone(),
            emit_files: true,
        };
        validate_showmap_config(&showmap)?;
        Ok(Some(showmap))
    }

    /// Restricts this test invocation to fuzz and invariant tests.
    pub(crate) const fn enable_fuzz_only(&mut self) {
        self.fuzz_only = true;
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

        let value = foundry_common::fs::read_json_file::<serde_json::Value>(path).wrap_err(
            format!("failed to read symbolic counterexample artifact {}", path.display()),
        )?;
        let schema_version =
            value.get("schema_version").and_then(serde_json::Value::as_u64).ok_or_else(|| {
                eyre::eyre!(
                    "symbolic counterexample artifact {} is missing numeric schema_version",
                    path.display()
                )
            })?;
        if schema_version != 1 {
            bail!(
                "unsupported symbolic counterexample artifact schema version {} in {}",
                schema_version,
                path.display()
            );
        }
        let schema = value.get("schema").and_then(serde_json::Value::as_str).ok_or_else(|| {
            eyre::eyre!(
                "symbolic counterexample artifact {} is missing string schema",
                path.display()
            )
        })?;
        if schema != SYMBOLIC_COUNTEREXAMPLE_ARTIFACT_SCHEMA {
            bail!(
                "unsupported symbolic counterexample artifact schema `{}` in {}",
                schema,
                path.display()
            );
        }
        let artifact = serde_json::from_value::<SymbolicCounterexampleArtifact>(value).wrap_err(
            format!("failed to parse symbolic counterexample artifact {}", path.display()),
        )?;
        if artifact.calls.is_empty() {
            bail!("symbolic counterexample artifact {} has no calls", path.display());
        }
        if artifact.replay.status != SymbolicReplayStatus::Confirmed {
            bail!(
                "symbolic counterexample artifact {} replay status must be confirmed, got {:?}",
                path.display(),
                artifact.replay.status,
            );
        }
        let Some((artifact_path, contract_name)) = artifact.test.contract.rsplit_once(':') else {
            bail!(
                "symbolic counterexample artifact {} test.contract must be `path:Contract`, got `{}`",
                path.display(),
                artifact.test.contract,
            );
        };
        if artifact_path.is_empty() || contract_name.is_empty() {
            bail!(
                "symbolic counterexample artifact {} test.contract must be `path:Contract`, got `{}`",
                path.display(),
                artifact.test.contract,
            );
        }

        Ok(Some(SymbolicArtifactReplayConfig { artifact, path: path.clone() }))
    }

    /// Returns a list of files that need to be compiled in order to run all the tests that match
    /// the given filter.
    ///
    /// This means that it will return all sources that are not test contracts or that match the
    /// filter. We want to compile all non-test sources always because tests might depend on them
    /// dynamically through cheatcodes.
    #[instrument(target = "forge::test", skip_all)]
    fn get_sources_to_compile(
        &self,
        config: &Config,
        test_filter: &ProjectPathsAwareFilter,
        inline_config: Option<Arc<InlineConfig>>,
        symbolic_artifact_replay: Option<&SymbolicArtifactReplayConfig>,
    ) -> Result<(BTreeSet<PathBuf>, Option<Arc<InlineConfig>>)> {
        // An empty filter doesn't filter out anything.
        // We can still optimize slightly by excluding scripts.
        if test_filter.is_empty() {
            return Ok((
                source_files_iter(&config.src, MultiCompilerLanguage::FILE_EXTENSIONS)
                    .chain(source_files_iter(&config.test, MultiCompilerLanguage::FILE_EXTENSIONS))
                    .collect(),
                None,
            ));
        }

        let filter_args = test_filter.args();
        let has_contract_or_test_filter = filter_args.test_pattern.is_some()
            || filter_args.test_pattern_inverse.is_some()
            || filter_args.contract_pattern.is_some()
            || filter_args.contract_pattern_inverse.is_some();
        if !has_contract_or_test_filter {
            return Ok((
                source_files_iter(&config.src, MultiCompilerLanguage::FILE_EXTENSIONS)
                    .chain(
                        source_files_iter(&config.test, MultiCompilerLanguage::FILE_EXTENSIONS)
                            .filter(|path| test_filter.matches_path(path)),
                    )
                    .collect(),
                None,
            ));
        }

        let mut project = config.create_project(true, true)?;
        let output = compile_abi_project(&mut project, ProjectCompiler::new().quiet(true))?;
        if output.has_compiler_errors() {
            sh_println!("{output}")?;
            eyre::bail!("Compilation failed");
        }

        let inline_config = match inline_config {
            Some(inline_config) => inline_config,
            None => Arc::new(InlineConfig::new_parsed(&output, config)?),
        };
        let test_matcher =
            TestFunctionMatcher::new(config, &inline_config, symbolic_artifact_replay);
        let files = sources_to_compile_from_artifacts(config, test_filter, &output, &test_matcher);

        Ok((files, Some(inline_config)))
    }

    /// Executes all the tests in the project.
    ///
    /// This will trigger the build process first. On success all test contracts that match the
    /// configured filter will be executed
    ///
    /// Returns the test results for all matching tests.
    pub async fn compile_and_run(&mut self) -> Result<TestOutcome> {
        if self.brutalize {
            return self.compile_and_run_brutalized().await;
        }

        let (
            project_root,
            config,
            evm_opts,
            output,
            filter,
            inline_config,
            replay_symbolic_artifact,
        ) = self.compile_project().await?;
        self.run_tests(
            &project_root,
            config,
            evm_opts,
            &output,
            &filter,
            TestExecutionOptions {
                replay_symbolic_artifact,
                ..TestExecutionOptions::default_run(inline_config)
            },
        )
        .await
    }

    /// Compile and run tests with brutalization applied to source files.
    async fn compile_and_run_brutalized(&mut self) -> Result<TestOutcome> {
        let (mut config, evm_opts) = self.load_config_and_evm_opts()?;

        if install::install_missing_dependencies(&mut config).await && config.auto_detect_remappings
        {
            config = self.load_config()?;
        }

        let rerun_failures = self.rerun.then(|| last_run_failures(&config));
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

        workspace::copy_project(&config, temp_path)?;
        let count = brutalizer::brutalize_project(&config, temp_path)?;

        if !silent {
            sh_status!("Brutalized {count} source files, compiling from temp workspace...")?;
        }

        let test_failures_file = config.test_failures_file.clone();
        let mut config = workspace::rebase_config_paths(&config, temp_path).sanitized();
        config.test_failures_file = test_failures_file;
        let project = config.project()?;
        let project_root = project.paths.root.clone();
        let replay_symbolic_artifact = self.load_symbolic_artifact_replay()?;
        let filter = self.filter_with_rerun_failures(&config, rerun_failures)?;

        let (files, inline_config) =
            self.get_sources_to_compile(&config, &filter, None, replay_symbolic_artifact.as_ref())?;
        let output = ProjectCompiler::new()
            .dynamic_test_linking(config.dynamic_test_linking)
            .quiet(shell::is_json() || self.junit)
            .files(files)
            .compile(&project)?;
        let inline_config = match inline_config {
            Some(inline_config) => inline_config,
            None => Arc::new(InlineConfig::new_parsed(&output, &config)?),
        };

        self.run_tests(
            &project_root,
            config,
            evm_opts,
            &output,
            &filter,
            TestExecutionOptions {
                replay_symbolic_artifact,
                ..TestExecutionOptions::default_run(inline_config)
            },
        )
        .await
    }

    async fn compile_project(
        &mut self,
    ) -> Result<(
        PathBuf,
        Config,
        EvmOpts,
        ProjectCompileOutput,
        ProjectPathsAwareFilter,
        Arc<InlineConfig>,
        Option<SymbolicArtifactReplayConfig>,
    )> {
        // Merge all configs.
        let (mut config, evm_opts) = self.load_config_and_evm_opts()?;

        let should_mutate = self.mutate.is_some();

        // Force dyn test linking for mutation testing
        if should_mutate {
            config.dynamic_test_linking = true;
            config.cache = true;
        }

        if install::install_missing_dependencies(&mut config).await && config.auto_detect_remappings
        {
            // need to re-configure here to also catch additional remappings
            config = self.load_config()?;
        }

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
            let (path, contract) = replay
                .artifact
                .test
                .contract
                .rsplit_once(':')
                .map_or(("", replay.artifact.test.contract.as_str()), |(path, contract)| {
                    (path, contract)
                });
            filter_args.test_pattern =
                Some(Regex::new(&format!("^{}$", regex::escape(&replay.artifact.test.test)))?);
            filter_args.contract_pattern =
                Some(Regex::new(&format!("^{}$", regex::escape(contract)))?);
            if !path.is_empty() {
                filter_args.path_pattern = Some(globset::escape(path).parse::<GlobMatcher>()?);
            }
        }
        trace!(target: "forge::test", ?filter, "using filter");

        let dynamic_test_linking = config.dynamic_test_linking;
        let quiet = shell::is_json() || self.junit;

        if self.list {
            let output = compile_abi_project(
                &mut project,
                ProjectCompiler::new().dynamic_test_linking(dynamic_test_linking).quiet(quiet),
            )?;
            let inline_config = Arc::new(InlineConfig::new_parsed(&output, &config)?);
            return Ok((
                project_root,
                config,
                evm_opts,
                output,
                filter,
                inline_config,
                replay_symbolic_artifact,
            ));
        }

        let compile = |files| {
            ProjectCompiler::new()
                .dynamic_test_linking(dynamic_test_linking)
                .quiet(quiet)
                .files(files)
                .compile(&project)
        };

        let (files, inline_config) =
            self.get_sources_to_compile(&config, &filter, None, replay_symbolic_artifact.as_ref())?;
        let output = compile(files)?;
        let inline_config = match inline_config {
            Some(inline_config) => inline_config,
            None => Arc::new(InlineConfig::new_parsed(&output, &config)?),
        };

        Ok((
            project_root,
            config,
            evm_opts,
            output,
            filter,
            inline_config,
            replay_symbolic_artifact,
        ))
    }

    pub(crate) async fn prepare_fuzz_minimize_replay(
        &mut self,
        corpus_dir: &Path,
    ) -> Result<FuzzMinimizeReplaySession> {
        let (_, mut config, mut evm_opts, output, filter, inline_config, _) =
            self.compile_project().await?;

        if config.fuzz.run == Some(0) {
            bail!("`fuzz.run` must be greater than 0");
        }

        if self.gas_report {
            evm_opts.isolate = true;
        } else {
            config.fuzz.gas_report_samples = 0;
            config.invariant.gas_report_samples = 0;
        }
        if config.fuzz.corpus.corpus_dir.is_none() {
            config.fuzz.corpus.corpus_dir = Some(corpus_dir.to_path_buf());
        }
        if config.invariant.corpus.corpus_dir.is_none() {
            config.invariant.corpus.corpus_dir = Some(corpus_dir.to_path_buf());
        }

        config.fuzz.seed = config.fuzz.seed.or(Some(U256::ZERO));

        evm_opts.infer_network_from_fork().await;

        let override_networks = inline_config.referenced_override_networks(&config.profile);
        let mut passes = Vec::new();

        if override_networks.is_empty() {
            passes.push(
                self.dispatch_fuzz_minimize_network(
                    &evm_opts,
                    config,
                    evm_opts.clone(),
                    &output,
                    FuzzMinimizeNetworkPassOptions {
                        inline_config: inline_config.clone(),
                        multi_network: MultiNetworkConfig::default(),
                    },
                    &filter,
                )
                .await?,
            );
        } else {
            let all_override_networks = override_networks.clone();
            passes.push(
                self.dispatch_fuzz_minimize_network(
                    &evm_opts,
                    config.clone(),
                    evm_opts.clone(),
                    &output,
                    FuzzMinimizeNetworkPassOptions {
                        inline_config: inline_config.clone(),
                        multi_network: MultiNetworkConfig {
                            all_override_networks: all_override_networks.clone(),
                            pass_network: None,
                        },
                    },
                    &filter,
                )
                .await?,
            );

            for &network in &override_networks {
                let mut pass_evm_opts = evm_opts.clone();
                pass_evm_opts.networks = network.into();
                passes.push(
                    self.dispatch_fuzz_minimize_network(
                        &pass_evm_opts,
                        config.clone(),
                        pass_evm_opts.clone(),
                        &output,
                        FuzzMinimizeNetworkPassOptions {
                            inline_config: inline_config.clone(),
                            multi_network: MultiNetworkConfig {
                                all_override_networks: all_override_networks.clone(),
                                pass_network: Some(network),
                            },
                        },
                        &filter,
                    )
                    .await?,
                );
            }
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
        if config.fuzz.run == Some(0) {
            bail!("`fuzz.run` must be greater than 0");
        }

        // Mutation testing has bespoke orchestration (per-mutant temp
        // workspaces, baseline + N mutants, aggregated mutation report). It is
        // not compatible with the single-run debug / flame / list / junit
        // modes — running them together would either mix incompatible output
        // formats, or run the secondary mode against the baseline tests and
        // then silently continue into mutation testing. Reject up front with a
        // clear error rather than do the wrong thing.
        if self.mutate.is_some() {
            let mut conflicts = Vec::new();
            if self.list {
                conflicts.push("--list");
            }
            if self.debug {
                conflicts.push("--debug");
            }
            if self.flamegraph {
                conflicts.push("--flamegraph");
            }
            if self.flamechart {
                conflicts.push("--flamechart");
            }
            if self.evm_profile.is_some() {
                conflicts.push("--evm-profile");
            }
            if self.junit {
                conflicts.push("--junit");
            }
            if execution.coverage {
                conflicts.push("coverage");
            }
            if self.showmap_out.is_some() {
                conflicts.push("--showmap-out");
            }
            if self.replay_symbolic_artifact.is_some() {
                conflicts.push("--replay-symbolic-artifact");
            }
            if !conflicts.is_empty() {
                bail!(
                    "`--mutate` cannot be combined with: {}. Re-run without those flags to use \
                     mutation testing.",
                    conflicts.join(", ")
                );
            }
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

        // Explicitly enable isolation for gas reports for more correct gas accounting.
        if self.gas_report {
            evm_opts.isolate = true;
        } else {
            // Do not collect gas report traces if gas report is not enabled.
            config.fuzz.gas_report_samples = 0;
            config.invariant.gas_report_samples = 0;
        }

        // Generate a random fuzz seed if none provided, for reproducibility.
        config.fuzz.seed = config
            .fuzz
            .seed
            .or_else(|| Some(U256::from_be_bytes(rand::rng().random::<[u8; 32]>())));

        // Create test options from general project settings and compiler output.
        execution.should_debug = self.debug;
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

        // Enable internal tracing for more informative flamegraph/profile.
        if !self.decode_internal && trace_output.is_some() {
            self.decode_internal = true;
        }

        // Choose the internal function tracing mode, if --decode-internal is provided.
        let decode_internal = if self.decode_internal {
            // If more than one function matched, we enable simple tracing.
            // If only one function matched, we enable full tracing. This is done in `run_tests`.
            InternalTraceMode::Simple
        } else {
            InternalTraceMode::None
        };

        // Auto-detect network from fork chain ID when not explicitly configured.
        evm_opts.infer_network_from_fork().await;

        // Clone config and evm_opts before dispatch (needed for mutation testing).
        let config_for_mutation = config.clone();
        let evm_opts_for_mutation = evm_opts.clone();

        // Detect per-test network annotations.
        let override_networks =
            execution.inline_config.referenced_override_networks(&config.profile);

        let (libraries, mut outcome) = if override_networks.is_empty() {
            // Single-pass: no per-test network overrides, use global network setting.
            execution.decode_internal = decode_internal;
            execution.multi_network = MultiNetworkConfig::default();
            self.dispatch_network(
                &evm_opts,
                config,
                evm_opts.clone(),
                output,
                filter,
                execution.clone(),
            )
            .await?
        } else {
            // Multi-pass: run each distinct network separately and merge results.
            let all_override_networks = override_networks.clone();
            let multi_pass_timer = Instant::now();

            // Default pass: global network, runs tests without an explicit network annotation.
            let (libraries, mut outcome) = self
                .dispatch_network(
                    &evm_opts,
                    config.clone(),
                    evm_opts.clone(),
                    output,
                    filter,
                    TestExecutionOptions {
                        decode_internal,
                        multi_network: MultiNetworkConfig {
                            all_override_networks: all_override_networks.clone(),
                            pass_network: None,
                        },
                        ..execution.clone()
                    },
                )
                .await?;

            // Override passes: one per annotated network.
            for &network in &override_networks {
                let mut pass_evm_opts = evm_opts.clone();
                pass_evm_opts.networks = network.into();
                let (_, pass_outcome) = self
                    .dispatch_network(
                        &pass_evm_opts,
                        config.clone(),
                        pass_evm_opts.clone(),
                        output,
                        filter,
                        TestExecutionOptions {
                            decode_internal,
                            multi_network: MultiNetworkConfig {
                                all_override_networks: all_override_networks.clone(),
                                pass_network: Some(network),
                            },
                            ..execution.clone()
                        },
                    )
                    .await?;
                merge_outcomes(&mut outcome, pass_outcome);
            }

            // Print the merged summary (per-pass summaries are suppressed in `run_tests_inner`).
            if !self.summary && !shell::is_json() {
                sh_println!("{}", outcome.summary(multi_pass_timer.elapsed()))?;
            }
            if self.summary && !outcome.results.is_empty() {
                let summary_report = TestSummaryReport::new(self.detailed, outcome.clone());
                sh_println!("{}", &summary_report)?;
            }

            (libraries, outcome)
        };

        if let Some(replay) = &execution.replay_symbolic_artifact {
            let replayed = outcome.tests().count();
            if replayed == 0 {
                bail!(
                    "symbolic artifact target `{}::{}` was not found",
                    replay.artifact.test.contract,
                    replay.artifact.test.test
                );
            }
            if replayed > 1 {
                bail!(
                    "symbolic artifact target `{}::{}` matched {} tests; replay requires exactly one target",
                    replay.artifact.test.contract,
                    replay.artifact.test.test,
                    replayed
                );
            }
        }

        if let Some(trace_output) = trace_output {
            enum RenderedTraceOutput {
                Flame {
                    file_name: String,
                    title: String,
                    flame_chart: bool,
                    folded_stack_trace: Vec<String>,
                },
                EvmProfile {
                    profile_json: Vec<u8>,
                    test_name: String,
                    contract: String,
                },
            }

            let rendered = {
                let output_label = trace_output.label();
                let no_tests = match trace_output {
                    TraceOutputKind::EvmProfile(_) => {
                        "cannot generate EVM profile: no tests were executed"
                    }
                    TraceOutputKind::Flamegraph | TraceOutputKind::Flamechart => {
                        "no tests were executed"
                    }
                };
                if !outcome.results.values().any(|suite| !suite.test_results.is_empty()) {
                    return Err(eyre::eyre!("{no_tests}"));
                }
                let decoder = outcome.last_run_decoder.clone().ok_or_else(|| {
                    eyre::eyre!("cannot generate {output_label}: missing trace decoder")
                })?;
                let (suite_name, test_name, test_result) = outcome
                    .results
                    .iter_mut()
                    .find_map(|(suite_name, suite)| {
                        suite.test_results.iter_mut().next().map(|(test_name, result)| {
                            (suite_name.as_str(), test_name.as_str(), result)
                        })
                    })
                    .ok_or_else(|| eyre::eyre!("{no_tests}"))?;
                let contract = suite_name.split(':').next_back().unwrap();
                let test_name_trimmed = test_name.trim_end_matches("()");

                let (_, arena) = test_result
                    .traces
                    .iter_mut()
                    .find(|(kind, _)| *kind == TraceKind::Execution)
                    .ok_or_else(|| {
                        eyre::eyre!(
                            "cannot generate {output_label} for {contract}::{test_name_trimmed}: \
                             no execution trace (test may have failed in setUp/constructor or been \
                             skipped)"
                        )
                    })?;

                // Decode traces.
                decode_trace_arena(arena, &decoder).await;

                match trace_output {
                    TraceOutputKind::Flamegraph | TraceOutputKind::Flamechart => {
                        let mut folded_stack_trace =
                            folded_stack_trace::build(arena, self.evm.isolate);
                        let flame_chart = matches!(trace_output, TraceOutputKind::Flamechart);
                        if flame_chart {
                            folded_stack_trace.reverse();
                        }
                        let label = trace_output.label();
                        RenderedTraceOutput::Flame {
                            file_name: format!("cache/{label}_{contract}_{test_name_trimmed}.svg"),
                            title: format!("{label} {contract}::{test_name_trimmed}"),
                            flame_chart,
                            folded_stack_trace,
                        }
                    }
                    TraceOutputKind::EvmProfile(EvmProfileFormat::Speedscope) => {
                        let profile = speedscope::builder::build(
                            arena,
                            test_name_trimmed,
                            contract,
                            self.evm.isolate,
                        );
                        RenderedTraceOutput::EvmProfile {
                            profile_json: serde_json::to_vec(&profile)?,
                            test_name: test_name_trimmed.to_string(),
                            contract: contract.to_string(),
                        }
                    }
                }
            };

            match rendered {
                RenderedTraceOutput::Flame {
                    file_name,
                    title,
                    flame_chart,
                    folded_stack_trace,
                } => {
                    let file =
                        std::fs::File::create(&file_name).wrap_err("failed to create file")?;
                    let file = std::io::BufWriter::new(file);

                    let mut options = inferno::flamegraph::Options::default();
                    options.title = title;
                    options.count_name = "gas".to_string();
                    options.flame_chart = flame_chart;

                    inferno::flamegraph::from_lines(
                        &mut options,
                        folded_stack_trace.iter().map(String::as_str),
                        file,
                    )
                    .wrap_err("failed to write svg")?;
                    sh_println!("Saved to {file_name}")?;

                    if let Err(e) = opener::open(&file_name) {
                        sh_err!("Failed to open {file_name}; please open it manually: {e}")?;
                    }
                }
                RenderedTraceOutput::EvmProfile { profile_json, test_name, contract } => {
                    let profile_path = format!("cache/evm_profile_{contract}_{test_name}.json");
                    fs::write(&profile_path, &profile_json)?;

                    sh_println!("Profile saved to {profile_path}")?;

                    if self.no_open {
                        return Ok(outcome);
                    }

                    evm_profile_server::serve_and_open(profile_json, &test_name, &contract).await?;
                }
            }
        }

        if execution.should_debug {
            // Get first non-empty suite result. We will have only one such entry.
            let (_, _, test_result) =
                outcome.remove_first().ok_or_eyre("no tests were executed")?;

            let sources =
                ContractSources::from_project_output(output, project_root, Some(&libraries))?;

            // Prefer execution traces for normal debug runs, but when execution never starts
            // (for example if `setUp()` reverts), fall back to available setup/deployment traces.
            let mut traces = {
                let execution = test_result
                    .traces
                    .iter()
                    .filter(|(kind, _)| kind.is_execution())
                    .cloned()
                    .collect::<Vec<_>>();
                if execution.is_empty() { test_result.traces.clone() } else { execution }
            };
            if let Some(decoder) = &outcome.last_run_decoder {
                for (_, arena) in &mut traces {
                    decode_trace_arena(arena, decoder).await;
                }
            }

            // Run the debugger.
            let mut builder = Debugger::builder()
                .traces(traces)
                .sources(sources)
                .breakpoints(test_result.breakpoints)
                .layout(self.debug_layout.unwrap_or_default());

            if let Some(decoder) = &outcome.last_run_decoder {
                builder = builder.decoder(decoder);
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
            // Check outcome here, stop if any test failed
            if outcome.failed() > 0 {
                eyre::bail!("Cannot run mutation testing with failed tests");
            }

            // A green baseline that ran zero non-skipped tests is not useful:
            // every compileable mutant would be reported as `Alive` (no test
            // failed, so nothing killed it), which produces a wildly
            // misleading mutation report. Hard-error so users get an actual
            // signal that their filter / path / setup matched nothing.
            if outcome.successes().next().is_none() {
                eyre::bail!(
                    "Mutation testing requires at least one passing baseline test; the current \
                     filter/path selection matched zero non-skipped tests. Loosen `--match-test` / \
                     `--match-contract` / `--match-path` or check the project layout."
                );
            }

            // Explicit paths on --mutate cannot be combined with the --mutate-path
            // glob filter: clap can't express this directly because --mutate takes
            // an optional list of paths.
            if !mutate.is_empty() && self.mutate_path.is_some() {
                eyre::bail!(
                    "`--mutate-path <PATTERN>` cannot be combined with explicit paths passed to `--mutate`; pass either paths or a glob pattern, not both"
                );
            }

            // The mutation runner builds a single-pass `MultiContractRunner`
            // (`runner.rs::compile_and_test_inner`) and does not honor inline
            // per-test network annotations. If the project declares network
            // overrides, running mutation testing would silently execute those
            // tests on the wrong network and produce false survivors / kills.
            // Bail with a clear error rather than do the wrong thing silently.
            if !override_networks.is_empty() {
                eyre::bail!(
                    "Mutation testing does not yet support inline per-test network overrides \
                     (found {} annotated network(s)). Re-run without `--mutate` or remove the \
                     per-test network annotations.",
                    override_networks.len()
                );
            }

            // The mutation runner symlinks dependency directories (`lib`,
            // `node_modules`, `dependencies`) into each per-mutant TempDir for
            // performance — see `workspace::copy_project`. That isolation
            // breaks down if tests can write to those shared trees, either via
            // `vm.writeFile` (broad `fs_permissions`) or arbitrary `ffi` calls.
            // Detect both up front so users aren't surprised by races or
            // corruption of their real dependency tree.
            if config_for_mutation.ffi {
                eyre::bail!(
                    "Mutation testing is unsafe with `ffi = true`: per-mutant workspaces share \
                     symlinked dependency directories, and arbitrary FFI commands run by tests \
                     can race or corrupt the real `lib`/`node_modules`/`dependencies` trees. \
                     Disable ffi in your foundry.toml to run mutation tests."
                );
            }

            // Only refuse write-capable `fs_permissions` whose path can actually
            // reach one of the symlinked dependency trees. Scoped writes (e.g.
            // `./out`, `./snapshots`) are safe because they target paths that
            // never resolve into the shared `lib`/`node_modules`/`dependencies`
            // trees.
            let root = &config_for_mutation.root;
            let canonicalize_through_existing_ancestor = |path: &Path| -> PathBuf {
                let resolved =
                    if path.is_absolute() { path.to_path_buf() } else { root.join(path) };
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
                for component in missing.iter().rev() {
                    canon.push(component);
                }
                canon
            };

            let mut shared_dep_dirs: Vec<PathBuf> = config_for_mutation
                .libs
                .iter()
                .filter(|p| p.exists())
                .map(|p| canonicalize_through_existing_ancestor(p))
                .collect();
            for dep_dir in ["node_modules", "dependencies"] {
                let dep_path = root.join(dep_dir);
                if dep_path.exists() && dep_path.is_dir() {
                    shared_dep_dirs.push(canonicalize_through_existing_ancestor(&dep_path));
                }
            }

            let effective_permission = |path: &Path| -> Option<FsAccessPermission> {
                let mut max_path_len = 0;
                let mut highest_permission = FsAccessPermission::None;

                for perm in &config_for_mutation.fs_permissions.permissions {
                    let permission_path = canonicalize_through_existing_ancestor(&perm.path);
                    if path.starts_with(&permission_path) {
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
                }

                (max_path_len > 0).then_some(highest_permission)
            };

            let grants_write = |path: &Path| {
                matches!(
                    effective_permission(path),
                    Some(FsAccessPermission::Write | FsAccessPermission::ReadWrite)
                )
            };

            let unsafe_write_paths: Vec<&Path> = config_for_mutation
                .fs_permissions
                .permissions
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
                .map(|p| p.path.as_path())
                .collect();

            if !unsafe_write_paths.is_empty() {
                let paths = unsafe_write_paths
                    .iter()
                    .map(|p| format!("  - {}", p.display()))
                    .collect::<Vec<_>>()
                    .join("\n");
                eyre::bail!(
                    "Mutation testing is unsafe with write-capable `fs_permissions` that can \
                     reach the symlinked dependency trees (`lib`/`node_modules`/`dependencies`); \
                     per-mutant workspaces share those trees, so `vm.writeFile` calls would race \
                     against or corrupt your real dependencies. Restrict the following \
                     `fs_permissions` entries to read-only or scope them away from dependency \
                     paths:\n{paths}"
                );
            }

            let mut config_for_mutation = config_for_mutation;
            apply_mutation_compiler_overrides(&mut config_for_mutation);

            let json_output = shell::is_json();
            let (selected_sources, _) = self.get_sources_to_compile(
                &config_for_mutation,
                filter,
                Some(execution.inline_config.clone()),
                execution.replay_symbolic_artifact.as_ref(),
            )?;
            let selected_sources_relative = selected_sources
                .into_iter()
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
                // Carry the same filter args (--match-test, --match-contract,
                // --match-path, positional path shorthand, --rerun, ...) and
                // isolation flag the baseline actually used, so every mutant
                // exercises the exact same test set under the same execution
                // model. We pull from the materialized `filter`, not the raw
                // CLI flags on `self`, because the baseline applies extras:
                // the positional `forge test <path>` shorthand is folded into
                // `path_pattern`, and `--rerun` injects last-run failures
                // into `test_pattern`. Using `self.filter.clone()` would lose
                // those and let mutant runs silently diverge from baseline.
                filter_args: filter.args().clone(),
                selected_sources_relative,
                isolate: evm_opts_for_mutation.isolate,
            };

            let result = run_mutation_testing(
                Arc::new(config_for_mutation.clone()),
                output,
                evm_opts_for_mutation.clone(),
                mutation_config,
            )
            .await?;

            if result.cancelled {
                std::process::exit(130);
            }

            // Output JSON if requested
            if json_output {
                let json_output = result.summary.to_json_output(result.duration_secs);
                sh_println!("{}", serde_json::to_string(&json_output)?)?;
            }

            outcome = TestOutcome::empty(None, true);
        }

        Ok(outcome)
    }

    /// Build the test runner and execute tests for a specific network type.
    async fn build_and_run_tests<FEN: FoundryEvmNetwork>(
        &self,
        config: Config,
        evm_opts: EvmOpts,
        output: &ProjectCompileOutput,
        filter: &ProjectPathsAwareFilter,
        execution: TestExecutionOptions,
    ) -> eyre::Result<(Libraries, TestOutcome)> {
        let verbosity = evm_opts.verbosity;
        let (evm_env, tx_env, fork_block) =
            evm_opts.env::<SpecFor<FEN>, BlockEnvFor<FEN>, TxEnvFor<FEN>>().await?;

        let config = Arc::new(config);
        let showmap = self.showmap_config()?;
        let runner = MultiContractRunnerBuilder::new(config.clone(), execution.inline_config)
            .set_debug(execution.should_debug)
            .set_decode_internal(execution.decode_internal)
            .set_record_all_steps(self.evm_profile.is_some())
            .initial_balance(evm_opts.initial_balance)
            .sender(evm_opts.sender)
            .with_fork(evm_opts.get_fork(&config, evm_env.cfg_env.chain_id, fork_block))
            .enable_isolation(evm_opts.isolate)
            .fail_fast(self.fail_fast)
            .set_coverage(execution.coverage)
            .with_multi_network(execution.multi_network)
            .with_showmap(showmap)
            .with_fuzz_only(self.fuzz_only)
            .with_fuzz_failure_replay(self.fuzz_failure_replay)
            .with_symbolic_artifact_replay(execution.replay_symbolic_artifact)
            .build::<FEN, MultiCompiler>(output, evm_env, tx_env, evm_opts)?;

        let libraries = runner.libraries.clone();
        let outcome = self.run_tests_inner(runner, config, verbosity, filter, output).await?;
        Ok((libraries, outcome))
    }

    async fn build_fuzz_minimize_runner<FEN: FoundryEvmNetwork>(
        &self,
        config: Config,
        evm_opts: EvmOpts,
        output: &ProjectCompileOutput,
        options: FuzzMinimizeNetworkPassOptions,
    ) -> eyre::Result<MultiContractRunner<FEN>> {
        let (evm_env, tx_env, fork_block) =
            evm_opts.env::<SpecFor<FEN>, BlockEnvFor<FEN>, TxEnvFor<FEN>>().await?;

        let config = Arc::new(config);
        MultiContractRunnerBuilder::new(config.clone(), options.inline_config)
            .initial_balance(evm_opts.initial_balance)
            .sender(evm_opts.sender)
            .with_fork(evm_opts.get_fork(&config, evm_env.cfg_env.chain_id, fork_block))
            .enable_isolation(evm_opts.isolate)
            .fail_fast(self.fail_fast)
            .with_multi_network(options.multi_network)
            .with_fuzz_only(self.fuzz_only)
            .with_fuzz_failure_replay(self.fuzz_failure_replay)
            .build::<FEN, MultiCompiler>(output, evm_env, tx_env, evm_opts)
    }

    /// Dispatches `build_and_run_tests` to the correct network type based on `evm_opts.networks`.
    async fn dispatch_network(
        &self,
        dispatch_opts: &EvmOpts,
        config: Config,
        evm_opts: EvmOpts,
        output: &ProjectCompileOutput,
        filter: &ProjectPathsAwareFilter,
        execution: TestExecutionOptions,
    ) -> eyre::Result<(Libraries, TestOutcome)> {
        match network_dispatch_kind(dispatch_opts) {
            NetworkDispatchKind::Tempo => {
                self.build_and_run_tests::<TempoEvmNetwork>(
                    config, evm_opts, output, filter, execution,
                )
                .await
            }
            #[cfg(feature = "optimism")]
            NetworkDispatchKind::Optimism => {
                self.build_and_run_tests::<OpEvmNetwork>(
                    config, evm_opts, output, filter, execution,
                )
                .await
            }
            NetworkDispatchKind::Eth => {
                self.build_and_run_tests::<EthEvmNetwork>(
                    config, evm_opts, output, filter, execution,
                )
                .await
            }
        }
    }

    async fn dispatch_fuzz_minimize_network(
        &self,
        dispatch_opts: &EvmOpts,
        config: Config,
        evm_opts: EvmOpts,
        output: &ProjectCompileOutput,
        options: FuzzMinimizeNetworkPassOptions,
        filter: &ProjectPathsAwareFilter,
    ) -> eyre::Result<FuzzMinimizeReplayPass> {
        match network_dispatch_kind(dispatch_opts) {
            NetworkDispatchKind::Tempo => self
                .build_fuzz_minimize_runner::<TempoEvmNetwork>(config, evm_opts, output, options)
                .await
                .map(|runner| fuzz_minimize_replay(runner, filter)),
            #[cfg(feature = "optimism")]
            NetworkDispatchKind::Optimism => self
                .build_fuzz_minimize_runner::<OpEvmNetwork>(config, evm_opts, output, options)
                .await
                .map(|runner| fuzz_minimize_replay(runner, filter)),
            NetworkDispatchKind::Eth => self
                .build_fuzz_minimize_runner::<EthEvmNetwork>(config, evm_opts, output, options)
                .await
                .map(|runner| fuzz_minimize_replay(runner, filter)),
        }
    }

    fn symbolic_regression_config(&self, config: &Config) -> Option<SymbolicRegressionConfig> {
        self.emit_regression.then(|| SymbolicRegressionConfig {
            out: self
                .regression_out
                .clone()
                .map(|path| if path.is_relative() { config.root.join(path) } else { path }),
            overwrite: self.regression_overwrite,
        })
    }

    /// Run all tests that matches the filter predicate from a test runner
    async fn run_tests_inner<FEN: FoundryEvmNetwork>(
        &self,
        mut runner: MultiContractRunner<FEN>,
        config: Arc<Config>,
        verbosity: u8,
        filter: &ProjectPathsAwareFilter,
        output: &ProjectCompileOutput,
    ) -> eyre::Result<TestOutcome> {
        let fuzz_seed = config.fuzz.seed;
        if self.list {
            return list(runner, filter);
        }
        let symbolic_regression = self.symbolic_regression_config(&config);

        trace!(target: "forge::test", "running all tests");

        // If we need to render to a serialized format, we should not print anything else to stdout.
        let silent = self.gas_report && shell::is_json()
            || self.summary && shell::is_json()
            || self.mutate.is_some() && shell::is_json();

        let num_filtered = runner.matching_test_functions(filter).count();

        if num_filtered == 0 {
            let total_tests = if filter.is_empty() {
                num_filtered
            } else {
                runner.matching_test_functions(&EmptyTestFilter::default()).count()
            };
            if total_tests == 0 {
                sh_println!(
                    "No tests found in project! Forge looks for functions that start with `test`"
                )?;
            } else {
                let mut msg = format!("no tests match the provided pattern:\n{filter}");
                // Try to suggest a test when there's no match.
                if let Some(test_pattern) = &filter.args().test_pattern {
                    let test_name = test_pattern.as_str();
                    // Filter contracts but not test functions.
                    let candidates = runner.all_test_functions(filter).map(|f| &f.name);
                    if let Some(suggestion) = utils::did_you_mean(test_name, candidates).pop() {
                        write!(msg, "\nDid you mean `{suggestion}`?")?;
                    }
                }
                sh_warn!("{msg}")?;
            }
            return Ok(TestOutcome::empty(Some(runner.known_contracts.clone()), false));
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
                format_matching_debug_tests(&runner.list(filter)).unwrap_or_default()
            } else {
                String::new()
            };
            let narrowing_hint = if self.debug {
                "Use --match-test <TEST_NAME>, --match-contract, and --match-path to further limit the search."
            } else {
                "Use --match-contract and --match-path to further limit the search."
            };
            eyre::bail!(
                "{num_filtered} tests matched your criteria, but exactly 1 test must match in order to {action}.{matching_tests_hint}\n\n\
                 {narrowing_hint}{filter_hint}",
            );
        }

        // If exactly one test matched, we enable full tracing.
        if num_filtered == 1 && self.decode_internal {
            runner.decode_internal = InternalTraceMode::Full;
        }

        // Run tests in a non-streaming fashion and collect results for serialization.
        if self.mutate.is_none() && !self.gas_report && !self.summary && shell::is_json() {
            let mut results = runner.test_collect(filter)?;
            for suite_result in results.values_mut() {
                for test_result in suite_result.test_results.values_mut() {
                    if verbosity >= 2 {
                        // Decode logs at level 2 and above.
                        test_result.decoded_logs = decode_console_logs(&test_result.logs);
                    } else {
                        // Empty logs for non verbose runs.
                        test_result.logs = vec![];
                    }
                }
            }
            if let Some(regression) = &symbolic_regression {
                let artifacts = collect_symbolic_artifacts_from_suites(results.values());
                let regressions = emit_symbolic_regressions(
                    &config,
                    regression,
                    &runner.known_contracts,
                    &artifacts,
                )?;
                attach_symbolic_regressions_to_suites(results.values_mut(), &regressions);
            }
            sh_println!("{}", serde_json::to_string(&results)?)?;
            let kc = runner.known_contracts.clone();
            return Ok(TestOutcome::new(Some(kc), results, self.allow_failure, fuzz_seed));
        }

        if self.junit {
            let mut results = runner.test_collect(filter)?;
            if let Some(regression) = &symbolic_regression {
                let artifacts = collect_symbolic_artifacts_from_suites(results.values());
                let regressions = emit_symbolic_regressions(
                    &config,
                    regression,
                    &runner.known_contracts,
                    &artifacts,
                )?;
                attach_symbolic_regressions_to_suites(results.values_mut(), &regressions);
            }
            sh_println!("{}", junit_xml_report(&results, verbosity).to_string()?)?;
            let kc = runner.known_contracts.clone();
            return Ok(TestOutcome::new(Some(kc), results, self.allow_failure, fuzz_seed));
        }

        let remote_chain =
            if runner.fork.is_some() { runner.tx_env.chain_id().map(Into::into) } else { None };
        let known_contracts = runner.known_contracts.clone();

        let libraries = runner.libraries.clone();

        // Capture multi-pass state before moving `runner` into the spawn task.
        // In multi-pass mode the per-pass summary is suppressed; the merged summary is
        // printed once by the caller after all passes complete.
        let is_multi_pass = !runner.tcfg.multi_network.all_override_networks.is_empty();
        let is_tempo_network = runner.tcfg.evm_opts.networks.is_tempo();

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
            .with_known_contracts(&known_contracts)
            .with_label_disabled(self.disable_labels)
            .with_verbosity(verbosity)
            .with_chain_id(remote_chain.map(|c| c.id()))
            .with_tempo_hardfork(
                (is_tempo_network || remote_chain.is_some_and(|chain| chain.is_tempo()))
                    .then(|| config.evm_spec_id::<TempoHardfork>()),
            );
        // Signatures are of no value for gas reports.
        if !self.gas_report {
            builder =
                builder.with_signature_identifier(SignaturesIdentifier::from_config(&config)?);
        }

        if self.decode_internal {
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
            )
        });

        let mut gas_snapshots = BTreeMap::<String, BTreeMap<String, String>>::new();

        let mut outcome = TestOutcome::empty(None, self.allow_failure);
        outcome.fuzz_seed = fuzz_seed;

        let mut any_test_failed = false;
        let mut backtrace_builder = None;
        for (contract_name, mut suite_result) in rx {
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

            // Some outputs need trace identities even if the textual trace is not rendered.
            let always_identify_traces = self.gas_report
                || self.debug
                || self.flamegraph
                || self.flamechart
                || self.evm_profile.is_some();

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
                let render_trace_output = should_render_trace_output(silent, show_traces);
                let should_include_trace = |kind: &TraceKind| match kind {
                    TraceKind::Execution => (verbosity == 3 && test_failed) || verbosity >= 4,
                    TraceKind::Setup => (verbosity == 4 && test_failed) || verbosity >= 5,
                    TraceKind::Deployment => false,
                };
                let renders_trace = render_trace_output
                    && result.traces.iter().any(|(kind, _)| should_include_trace(kind));
                let identify_addresses = always_identify_traces || renders_trace;

                if !silent {
                    sh_println!("{}", result.short_result_with_suite(name, &contract_name))?;
                    for artifact in &result.counterexample_artifacts {
                        sh_warn!("Counterexample artifact: {}", artifact.path.display())?;
                    }

                    // Display invariant metrics if invariant kind.
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
                let mut decoded_traces = if renders_trace {
                    Vec::with_capacity(result.traces.len())
                } else {
                    Vec::new()
                };
                if identify_addresses || renders_trace {
                    for (kind, arena) in &mut result.traces {
                        if identify_addresses {
                            if self.debug && !result.debug_bytecodes.is_empty() {
                                let mut local_identifier = TraceIdentifiers::new()
                                    .with_local_and_bytecodes(
                                        &known_contracts,
                                        &result.debug_bytecodes,
                                    );
                                decoder.identify(arena, &mut local_identifier);
                            }
                            decoder.identify(arena, &mut identifier);
                        }

                        // verbosity:
                        // - 0..3: nothing
                        // - 3: only display traces for failed tests
                        // - 4: also display the setup trace for failed tests
                        // - 5..: display all traces for all tests, including storage changes
                        let should_include = should_include_trace(kind);

                        if renders_trace && should_include {
                            decode_trace_arena(arena, &decoder).await;

                            if let Some(trace_depth) = self.trace_depth {
                                prune_trace_depth(arena, trace_depth);
                            }

                            decoded_traces.push(render_trace_arena_inner(
                                arena,
                                false,
                                verbosity > 4,
                            ));
                        }
                    }
                }

                if !silent && show_traces && !decoded_traces.is_empty() {
                    sh_println!("Traces:")?;
                    for trace in &decoded_traces {
                        sh_println!("{trace}")?;
                    }
                }

                // Extract and display backtrace for failed tests when verbosity >= 3.
                // At verbosity 3-4 backtraces show contract/function names only.
                // At verbosity 5 backtraces include source file locations.
                if !silent
                    && result.status.is_failure()
                    && verbosity >= 3
                    && !result.traces.is_empty()
                    && let Some((_, arena)) =
                        result.traces.iter().find(|(kind, _)| matches!(kind, TraceKind::Execution))
                {
                    // Lazily initialize the backtrace builder on first failure
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
                                decoder.identify(arena, &mut identifier);
                            }
                        }

                        for arena in trace {
                            decoder.identify(arena, &mut identifier);
                            gas_report.analyze([arena], &decoder).await;
                        }
                    }
                }
                // Clear memory.
                result.gas_report_traces = Default::default();

                // Collect and merge gas snapshots.
                for (group, new_snapshots) in &result.gas_snapshots {
                    gas_snapshots.entry(group.clone()).or_default().extend(new_snapshots.clone());
                }
            }

            // Write gas snapshots to disk if any were collected.
            if !gas_snapshots.is_empty() {
                // By default `gas_snapshot_check` is set to `false` in the config.
                //
                // The user can either:
                // - Set `FORGE_SNAPSHOT_CHECK=true` in the environment.
                // - Pass `--gas-snapshot-check=true` as a CLI argument.
                // - Set `gas_snapshot_check = true` in the config.
                //
                // If the user passes `--gas-snapshot-check=<bool>` then it will override the config
                // and the environment variable, disabling the check if `false` is passed.
                //
                // Exiting early with code 1 if differences are found.
                if self.gas_snapshot_check.unwrap_or(config.gas_snapshot_check) {
                    let differences_found =
                        gas_snapshots.iter().fold(false, |mut found, (group, snapshots)| {
                            // If the snapshot file doesn't exist, we can't compare so we skip.
                            if !&config.snapshots.join(format!("{group}.json")).exists() {
                                return found;
                            }

                            let previous_snapshots: BTreeMap<String, String> =
                                fs::read_json_file(&config.snapshots.join(format!("{group}.json")))
                                    .expect("Failed to read snapshots from disk");

                            let diff: BTreeMap<_, _> = snapshots
                                .iter()
                                .filter_map(|(k, v)| {
                                    previous_snapshots.get(k).and_then(|previous_snapshot| {
                                        (previous_snapshot != v).then(|| {
                                            (k.clone(), (previous_snapshot.clone(), v.clone()))
                                        })
                                    })
                                })
                                .collect();

                            if !diff.is_empty() {
                                let _ = sh_eprintln!(
                                    "{}",
                                    format!("\n[{group}] Failed to match snapshots:").red().bold()
                                );

                                for (key, (previous_snapshot, snapshot)) in &diff {
                                    let _ = sh_eprintln!(
                                        "{}",
                                        format!("- [{key}] {previous_snapshot} → {snapshot}").red()
                                    );
                                }

                                found = true;
                            }

                            found
                        });

                    if differences_found {
                        sh_eprintln!()?;
                        eyre::bail!("Snapshots differ from previous run");
                    }
                }

                // By default `gas_snapshot_emit` is set to `true` in the config.
                //
                // The user can either:
                // - Set `FORGE_SNAPSHOT_EMIT=false` in the environment.
                // - Pass `--gas-snapshot-emit=false` as a CLI argument.
                // - Set `gas_snapshot_emit = false` in the config.
                //
                // If the user passes `--gas-snapshot-emit=<bool>` then it will override the config
                // and the environment variable, enabling the check if `true` is passed.
                if self.gas_snapshot_emit.unwrap_or(config.gas_snapshot_emit) {
                    // Create `snapshots` directory if it doesn't exist.
                    fs::create_dir_all(&config.snapshots)?;

                    // Write gas snapshots to disk per group.
                    for (group, snapshots) in &gas_snapshots {
                        fs::write_pretty_json_file(
                            &config.snapshots.join(format!("{group}.json")),
                            &snapshots,
                        )
                        .expect("Failed to write gas snapshots to disk");
                    }
                }
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
        if let Some(regression) = &symbolic_regression {
            let artifacts = collect_symbolic_artifacts_from_suites(outcome.results.values());
            let regressions =
                emit_symbolic_regressions(&config, regression, &known_contracts, &artifacts)?;
            attach_symbolic_regressions_to_suites(outcome.results.values_mut(), &regressions);
            if !silent {
                for regression in regressions {
                    sh_warn!(
                        "Regression test: {} (from {})",
                        regression.path.display(),
                        regression.artifact.display()
                    )?;
                }
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

        if !is_multi_pass && !self.summary && !shell::is_json() {
            sh_println!("{}", outcome.summary(duration))?;
        }

        if !is_multi_pass && self.summary && !outcome.results.is_empty() {
            let summary_report = TestSummaryReport::new(self.detailed, outcome.clone());
            sh_println!("{summary_report}")?;
        }

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

        // Persist test run failures to enable replaying.
        persist_run_failures(&config, &outcome);

        Ok(outcome)
    }

    /// Returns the flattened [`FilterArgs`] arguments merged with [`Config`].
    /// Loads and applies filter from file if only last test run failures performed.
    pub fn filter(&self, config: &Config) -> Result<ProjectPathsAwareFilter> {
        self.filter_with_rerun_failures(config, None)
    }

    fn filter_with_rerun_failures(
        &self,
        config: &Config,
        loaded_rerun_failures: Option<LastRunFailures>,
    ) -> Result<ProjectPathsAwareFilter> {
        let mut filter = self.filter.clone();
        let rerun_failures = if self.rerun {
            let failures = loaded_rerun_failures.unwrap_or_else(|| last_run_failures(config));
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

const fn should_render_trace_output(silent: bool, show_traces: bool) -> bool {
    !silent && show_traces
}

impl Provider for TestArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Core Build Args Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut dict = Dict::default();

        let mut fuzz_dict = Dict::default();
        if let Some(fuzz_seed) = self.fuzz_seed {
            fuzz_dict.insert("seed".to_string(), fuzz_seed.to_string().into());
        }
        if let Some(fuzz_runs) = self.fuzz_runs {
            fuzz_dict.insert("runs".to_string(), fuzz_runs.into());
        }
        if let Some(fuzz_run) = self.fuzz_run {
            fuzz_dict.insert("run".to_string(), fuzz_run.into());
        }
        if let Some(fuzz_worker) = self.fuzz_worker {
            fuzz_dict.insert("worker".to_string(), fuzz_worker.into());
        }
        if let Some(fuzz_timeout) = self.fuzz_timeout {
            fuzz_dict.insert("timeout".to_string(), fuzz_timeout.into());
        }
        if let Some(fuzz_dictionary_weight) = self.fuzz_dictionary_weight {
            fuzz_dict.insert("dictionary_weight".to_string(), fuzz_dictionary_weight.into());
        }
        if let Some(fuzz_dictionary_addresses) = self.fuzz_dictionary_addresses.clone() {
            fuzz_dict.insert(
                "max_fuzz_dictionary_addresses".to_string(),
                fuzz_dictionary_addresses.into(),
            );
        }
        if let Some(fuzz_dictionary_values) = self.fuzz_dictionary_values.clone() {
            fuzz_dict
                .insert("max_fuzz_dictionary_values".to_string(), fuzz_dictionary_values.into());
        }
        if let Some(fuzz_dictionary_literals) = self.fuzz_dictionary_literals.clone() {
            fuzz_dict.insert(
                "max_fuzz_dictionary_literals".to_string(),
                fuzz_dictionary_literals.into(),
            );
        }
        if let Some(fuzz_corpus_random_sequence_weight) = self.fuzz_corpus_random_sequence_weight {
            fuzz_dict.insert(
                "corpus_random_sequence_weight".to_string(),
                fuzz_corpus_random_sequence_weight.into(),
            );
        }
        if let Some(fuzz_corpus_dir) = self.fuzz_corpus_dir.clone() {
            fuzz_dict.insert(
                "corpus_dir".to_string(),
                fuzz_corpus_dir.to_string_lossy().to_string().into(),
            );
        }
        if let Some(fuzz_frontier_dir) = self.fuzz_frontier_dir.clone() {
            fuzz_dict.insert(
                "frontier_dir".to_string(),
                fuzz_frontier_dir.to_string_lossy().to_string().into(),
            );
        }
        if let Some(fuzz_frontier_limit) = self.fuzz_frontier_limit {
            fuzz_dict.insert("frontier_limit".to_string(), fuzz_frontier_limit.into());
        }
        if let Some(fuzz_payable_value_weight) = self.fuzz_payable_value_weight {
            fuzz_dict.insert("payable_value_weight".to_string(), fuzz_payable_value_weight.into());
        }
        if let Some(weight) = self.fuzz_mutation_weight_splice {
            fuzz_dict.insert("mutation_weight_splice".to_string(), weight.into());
        }
        if let Some(weight) = self.fuzz_mutation_weight_repeat {
            fuzz_dict.insert("mutation_weight_repeat".to_string(), weight.into());
        }
        if let Some(weight) = self.fuzz_mutation_weight_interleave {
            fuzz_dict.insert("mutation_weight_interleave".to_string(), weight.into());
        }
        if let Some(weight) = self.fuzz_mutation_weight_prefix {
            fuzz_dict.insert("mutation_weight_prefix".to_string(), weight.into());
        }
        if let Some(weight) = self.fuzz_mutation_weight_suffix {
            fuzz_dict.insert("mutation_weight_suffix".to_string(), weight.into());
        }
        if let Some(weight) = self.fuzz_mutation_weight_abi {
            fuzz_dict.insert("mutation_weight_abi".to_string(), weight.into());
        }
        if let Some(weight) = self.fuzz_mutation_weight_cmp {
            fuzz_dict.insert("mutation_weight_cmp".to_string(), weight.into());
        }
        if let Some(fuzz_input_file) = self.fuzz_input_file.clone() {
            fuzz_dict.insert("failure_persist_file".to_string(), fuzz_input_file.into());
        }
        dict.insert("fuzz".to_string(), fuzz_dict.into());

        let mut invariant_dict = Dict::default();
        if let Some(invariant_depth) = self.invariant_depth {
            invariant_dict.insert("depth".to_string(), invariant_depth.into());
        }
        if let Some(invariant_min_depth) = self.invariant_min_depth {
            invariant_dict.insert("min_depth".to_string(), invariant_min_depth.into());
        }
        if let Some(invariant_depth_mode) = self.invariant_depth_mode {
            invariant_dict
                .insert("depth_mode".to_string(), Value::serialize(invariant_depth_mode)?);
        }
        if let Some(invariant_workers) = self.invariant_workers {
            invariant_dict.insert("workers".to_string(), Value::serialize(invariant_workers)?);
        }
        if let Some(invariant_dictionary_weight) = self.invariant_dictionary_weight {
            invariant_dict
                .insert("dictionary_weight".to_string(), invariant_dictionary_weight.into());
        }
        if let Some(invariant_dictionary_addresses) = self.invariant_dictionary_addresses.clone() {
            invariant_dict.insert(
                "max_fuzz_dictionary_addresses".to_string(),
                invariant_dictionary_addresses.into(),
            );
        }
        if let Some(invariant_dictionary_values) = self.invariant_dictionary_values.clone() {
            invariant_dict.insert(
                "max_fuzz_dictionary_values".to_string(),
                invariant_dictionary_values.into(),
            );
        }
        if let Some(invariant_dictionary_literals) = self.invariant_dictionary_literals.clone() {
            invariant_dict.insert(
                "max_fuzz_dictionary_literals".to_string(),
                invariant_dictionary_literals.into(),
            );
        }
        if let Some(invariant_corpus_random_sequence_weight) =
            self.invariant_corpus_random_sequence_weight
        {
            invariant_dict.insert(
                "corpus_random_sequence_weight".to_string(),
                invariant_corpus_random_sequence_weight.into(),
            );
            invariant_dict
                .insert("corpus_random_sequence_weight_configured".to_string(), true.into());
        }
        if let Some(invariant_corpus_dir) = self.invariant_corpus_dir.clone() {
            invariant_dict.insert(
                "corpus_dir".to_string(),
                invariant_corpus_dir.to_string_lossy().to_string().into(),
            );
        }
        if let Some(invariant_payable_value_weight) = self.invariant_payable_value_weight {
            invariant_dict
                .insert("payable_value_weight".to_string(), invariant_payable_value_weight.into());
        }
        if let Some(weight) = self.invariant_mutation_weight_splice {
            invariant_dict.insert("mutation_weight_splice".to_string(), weight.into());
        }
        if let Some(weight) = self.invariant_mutation_weight_repeat {
            invariant_dict.insert("mutation_weight_repeat".to_string(), weight.into());
        }
        if let Some(weight) = self.invariant_mutation_weight_interleave {
            invariant_dict.insert("mutation_weight_interleave".to_string(), weight.into());
        }
        if let Some(weight) = self.invariant_mutation_weight_prefix {
            invariant_dict.insert("mutation_weight_prefix".to_string(), weight.into());
        }
        if let Some(weight) = self.invariant_mutation_weight_suffix {
            invariant_dict.insert("mutation_weight_suffix".to_string(), weight.into());
        }
        if let Some(weight) = self.invariant_mutation_weight_abi {
            invariant_dict.insert("mutation_weight_abi".to_string(), weight.into());
        }
        if let Some(weight) = self.invariant_mutation_weight_cmp {
            invariant_dict.insert("mutation_weight_cmp".to_string(), weight.into());
        }
        if !invariant_dict.is_empty() {
            dict.insert("invariant".to_string(), invariant_dict.into());
        }

        let mut symbolic_dict = Dict::default();
        if self.symbolic {
            symbolic_dict.insert("enabled".to_string(), true.into());
        }
        if self.symbolic_seed_corpus {
            symbolic_dict.insert("seed_corpus".to_string(), true.into());
        }
        if self.symbolic_use_fuzz_corpus {
            symbolic_dict.insert("use_fuzz_corpus".to_string(), true.into());
        }
        if let Some(corpus_seed_limit) = self.symbolic_corpus_seed_limit {
            symbolic_dict.insert("corpus_seed_limit".to_string(), corpus_seed_limit.into());
        }
        if self.symbolic_use_fuzz_frontiers {
            symbolic_dict.insert("use_fuzz_frontiers".to_string(), true.into());
        }
        if let Some(frontier_limit) = self.symbolic_frontier_limit {
            symbolic_dict.insert("frontier_limit".to_string(), frontier_limit.into());
        }
        if let Some(frontier_ids) = self.symbolic_frontier_ids.clone() {
            symbolic_dict.insert("frontier_ids".to_string(), frontier_ids.into());
        }
        if let Some(frontier_pcs) = self.symbolic_frontier_pcs.clone() {
            symbolic_dict.insert("frontier_pcs".to_string(), frontier_pcs.into());
        }
        if let Some(frontier_selectors) = self.symbolic_frontier_selectors.clone() {
            symbolic_dict.insert("frontier_selectors".to_string(), frontier_selectors.into());
        }
        if let Some(solver) = self.symbolic_solver.clone() {
            symbolic_dict.insert("solver".to_string(), solver.into());
        }
        if let Some(solver_command) = self.symbolic_solver_command.clone() {
            symbolic_dict.insert("solver_command".to_string(), solver_command.into());
        }
        if let Some(solver_portfolio) = self.symbolic_solver_portfolio.clone() {
            symbolic_dict.insert("solver_portfolio".to_string(), solver_portfolio.into());
        }
        if let Some(timeout) = self.symbolic_timeout {
            symbolic_dict.insert("timeout".to_string(), timeout.into());
        }
        if let Some(loop_bound) = self.symbolic_loop {
            symbolic_dict.insert("loop".to_string(), loop_bound.into());
        }
        if let Some(depth) = self.symbolic_depth {
            symbolic_dict.insert("depth".to_string(), depth.into());
        }
        if let Some(width) = self.symbolic_width {
            symbolic_dict.insert("width".to_string(), width.into());
        }
        if let Some(max_depth) = self.symbolic_max_depth {
            symbolic_dict.insert("max_depth".to_string(), max_depth.into());
        }
        if let Some(max_paths) = self.symbolic_max_paths {
            symbolic_dict.insert("max_paths".to_string(), max_paths.into());
        }
        if let Some(invariant_depth) = self.symbolic_invariant_depth {
            symbolic_dict.insert("invariant_depth".to_string(), invariant_depth.into());
        }
        if let Some(max_solver_queries) = self.symbolic_max_solver_queries {
            symbolic_dict.insert("max_solver_queries".to_string(), max_solver_queries.into());
        }
        if let Some(default_dynamic_length) = self.symbolic_default_dynamic_length {
            symbolic_dict
                .insert("default_dynamic_length".to_string(), default_dynamic_length.into());
        }
        if let Some(max_dynamic_length) = self.symbolic_max_dynamic_length {
            symbolic_dict.insert("max_dynamic_length".to_string(), max_dynamic_length.into());
        }
        if let Some(array_lengths) = self.symbolic_array_lengths.clone() {
            symbolic_dict.insert("array_lengths".to_string(), array_lengths.into());
        }
        if let Some(max_calldata_bytes) = self.symbolic_max_calldata_bytes {
            symbolic_dict.insert("max_calldata_bytes".to_string(), max_calldata_bytes.into());
        }
        if self.symbolic_call_targets {
            symbolic_dict.insert("symbolic_call_targets".to_string(), true.into());
        }
        if self.symbolic_dump_smt {
            symbolic_dict.insert("dump_smt".to_string(), true.into());
        }
        if let Some(storage_layout) = self.symbolic_storage_layout.clone() {
            symbolic_dict.insert("storage_layout".to_string(), storage_layout.into());
        }
        dict.insert("symbolic".to_string(), symbolic_dict.into());

        if let Some(etherscan_api_key) =
            self.etherscan_api_key.as_ref().filter(|s| !s.trim().is_empty())
        {
            dict.insert("etherscan_api_key".to_string(), etherscan_api_key.clone().into());
        }

        if self.show_progress {
            dict.insert("show_progress".to_string(), true.into());
        }

        // Mutation-testing CLI overrides
        if self.mutation_timeout.is_some()
            || self.mutation_optimizer_runs.is_some()
            || self.mutation_via_ir.is_some()
        {
            let mut mutation_dict = Dict::default();
            if let Some(timeout) = self.mutation_timeout {
                mutation_dict.insert("timeout".to_string(), timeout.into());
            }
            if let Some(optimizer_runs) = self.mutation_optimizer_runs {
                mutation_dict.insert("optimizer_runs".to_string(), optimizer_runs.into());
            }
            if let Some(via_ir) = self.mutation_via_ir {
                mutation_dict.insert("via_ir".to_string(), via_ir.into());
            }
            dict.insert("mutation".to_string(), mutation_dict.into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
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

/// Lists all matching tests
fn list<FEN: FoundryEvmNetwork>(
    runner: MultiContractRunner<FEN>,
    filter: &ProjectPathsAwareFilter,
) -> Result<TestOutcome> {
    let results = runner.list(filter);
    print_list_results(&results)?;
    Ok(TestOutcome::empty(Some(runner.known_contracts), false))
}

fn list_from_output(
    output: &ProjectCompileOutput,
    config: &Config,
    inline_config: &InlineConfig,
    filter: &ProjectPathsAwareFilter,
    fuzz_only: bool,
    symbolic_artifact_replay: Option<&SymbolicArtifactReplayConfig>,
) -> Result<TestOutcome> {
    let matcher = TestFunctionMatcher::new(config, inline_config, symbolic_artifact_replay);
    let results = output
        .artifact_ids()
        .filter_map(|(id, artifact)| {
            let abi = artifact.abi.as_ref()?;
            let id = id.with_stripped_file_prefixes(&config.root);
            let deployable = abi
                .constructor
                .as_ref()
                .map(|constructor| constructor.inputs.is_empty())
                .unwrap_or(true);
            if !deployable || !matcher.matches_contract(filter, &id, abi) {
                return None;
            }
            let source = id.source.as_path().display().to_string();
            let identifier = id.identifier();
            let name = id.name;
            let generated_symbolic_regression = is_generated_symbolic_regression_contract(abi);
            let tests = abi
                .functions()
                .filter(|func| {
                    let kind = matcher.test_function_kind(
                        &identifier,
                        func,
                        generated_symbolic_regression,
                    );
                    (!fuzz_only
                        || matches!(
                            kind,
                            TestFunctionKind::FuzzTest { .. } | TestFunctionKind::InvariantTest
                        ))
                        && filter.matches_test_function_kind_in_contract(&identifier, func, kind)
                })
                .map(|func| func.name.clone())
                .collect::<Vec<_>>();
            (!tests.is_empty()).then_some((source, name, tests))
        })
        .fold(
            BTreeMap::<String, BTreeMap<String, Vec<String>>>::new(),
            |mut acc, (source, name, tests)| {
                acc.entry(source).or_default().insert(name, tests);
                acc
            },
        );

    print_list_results(&results)?;
    Ok(TestOutcome::empty(None, false))
}

fn print_list_results(results: &BTreeMap<String, BTreeMap<String, Vec<String>>>) -> Result<()> {
    if shell::is_json() {
        sh_println!("{}", serde_json::to_string(&results)?)?;
    } else {
        for (file, contracts) in results {
            sh_println!("{file}")?;
            for (contract, tests) in contracts {
                sh_println!("  {contract}")?;
                sh_println!("    {}\n", tests.join("\n    "))?;
            }
        }
    }
    Ok(())
}

/// Merges `other` into `base` by extending suite results.
///
/// For suites that appear in both, test results are combined (function-level pass routing ensures
/// each function appears in exactly one pass, so there are no key conflicts in practice).
fn merge_outcomes(base: &mut TestOutcome, other: TestOutcome) {
    for (suite_id, other_suite) in other.results {
        match base.results.entry(suite_id) {
            std::collections::btree_map::Entry::Vacant(e) => {
                e.insert(other_suite);
            }
            std::collections::btree_map::Entry::Occupied(mut e) => {
                let base_suite = e.get_mut();
                base_suite.test_results.extend(other_suite.test_results);
                base_suite.warnings.extend(other_suite.warnings);
                base_suite.duration = base_suite.duration.max(other_suite.duration);
            }
        }
    }
    if let Some(decoder) = other.last_run_decoder {
        base.last_run_decoder = Some(decoder);
    }
}

fn format_matching_debug_tests(
    matching_tests: &BTreeMap<String, BTreeMap<String, Vec<String>>>,
) -> Option<String> {
    let mut output = String::from("\n\nMatching tests:");
    let mut total = 0;
    let mut shown = 0;

    for (source, contracts) in matching_tests {
        for (contract, tests) in contracts {
            for test in tests {
                total += 1;

                if shown < DEBUGGER_MATCHING_TESTS_DISPLAY_LIMIT {
                    output.push_str("\n  ");
                    output.push_str(source);
                    output.push(':');
                    output.push_str(contract);
                    output.push('.');
                    output.push_str(test);
                    shown += 1;
                }
            }
        }
    }

    if total == 0 {
        return None;
    }

    if total > shown {
        output.push_str(&format!("\n  ... and {} more", total - shown));
    }

    Some(output)
}

struct LastRunFailures {
    test_pattern: Option<regex::Regex>,
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

        let output = serde_json::to_string(&RerunFailures { version: 1, failures });
        if let Ok(output) = output {
            let _ = fs::write(&config.test_failures_file, output);
        }
    }
}

fn rerun_filter_matches<'a>(
    test_name: &'a str,
    test_result: &'a TestResult,
) -> impl Iterator<Item = String> + 'a {
    let has_predicate_failures =
        test_result.invariant_failures.iter().any(|failure| failure.predicate_name().is_some());
    let predicate_failures =
        test_result.invariant_failures.iter().filter_map(|failure| failure.predicate_name());

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
            test_result,
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
                test_result,
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
            let name = format!("{}()", predicate.name);
            add_expanded_case(
                &name,
                predicate.status,
                predicate.reason.as_deref(),
                failure.and_then(|failure| failure.counterexample()),
            );
        }
    }

    for failure in &test_result.invariant_handler_failures {
        let name = format!("handler {}", failure.name());
        add_expanded_case(
            &name,
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
    test_result: &TestResult,
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
    test_case.set_time(test_result.duration);
    test_case.set_system_out(system_out);
    test_suite.add_test_case(test_case);
}

/// Helper for assembling JUnit output strings.
struct JunitOutput {
    result_report: TestKindReport,
    logs: Option<Vec<String>>,
}

impl JunitOutput {
    /// Creates a JUnit output helper for a test result.
    fn new(test_result: &TestResult, verbosity: u8) -> Self {
        Self {
            result_report: test_result.kind.report(),
            logs: (verbosity >= 2 && !test_result.logs.is_empty())
                .then(|| decode_console_logs(&test_result.logs)),
        }
    }

    /// Renders the suite-level `system-out` payload.
    fn system_out(&self, test_result: &TestResult, test_name: &str) -> String {
        let mut sys_out = String::new();
        write!(sys_out, "{test_result} {test_name} {}", self.result_report).unwrap();
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
        let mut sys_out = String::new();
        match status {
            TestStatus::Success => write!(sys_out, "[PASS]").unwrap(),
            TestStatus::Failure => {
                let message = message.unwrap_or_default();
                write!(sys_out, "[FAIL: {message}]").unwrap();
            }
            TestStatus::Skipped => {
                if let Some(message) = message {
                    write!(sys_out, "[SKIP: {message}]").unwrap();
                } else {
                    write!(sys_out, "[SKIP]").unwrap();
                }
            }
        }
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
    use foundry_config::Chain;

    #[test]
    fn watch_parse() {
        let args: TestArgs = TestArgs::parse_from(["foundry-cli", "-vw"]);
        assert!(args.watch.watch.is_some());
    }

    #[test]
    fn fuzz_seed() {
        let args: TestArgs = TestArgs::parse_from(["foundry-cli", "--fuzz-seed", "0x10"]);
        assert!(args.fuzz_seed.is_some());
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
    fn depth_trace() {
        let args: TestArgs = TestArgs::parse_from(["foundry-cli", "--trace-depth", "2"]);
        assert!(args.trace_depth.is_some());
    }

    #[test]
    fn silent_output_disables_trace_rendering() {
        assert!(!should_render_trace_output(true, true));
        assert!(!should_render_trace_output(false, false));
        assert!(should_render_trace_output(false, true));
    }

    // <https://github.com/foundry-rs/foundry/issues/5913>
    #[test]
    fn fuzz_seed_exists() {
        let args: TestArgs =
            TestArgs::parse_from(["foundry-cli", "-vvv", "--gas-report", "--fuzz-seed", "0x10"]);
        assert!(args.fuzz_seed.is_some());
    }

    #[test]
    fn fuzz_run() {
        let args: TestArgs =
            TestArgs::parse_from(["foundry-cli", "--fuzz-run", "10", "--fuzz-worker", "2"]);
        assert_eq!(args.fuzz_run, Some(10));
        assert_eq!(args.fuzz_worker, Some(2));
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
        assert_eq!(args.mutation_optimizer_runs, Some(1));
        assert_eq!(args.mutation_via_ir, Some(false));

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
    fn invariant_workers() {
        let args = TestArgs::parse_from(["foundry-cli", "--invariant-workers", "4"]);
        assert_eq!(
            args.invariant_workers,
            Some(InvariantWorkers::Fixed(std::num::NonZeroUsize::new(4).unwrap()))
        );

        let figment = figment::Figment::from(&args);
        assert_eq!(
            figment.extract_inner::<InvariantWorkers>("invariant.workers").unwrap(),
            InvariantWorkers::Fixed(std::num::NonZeroUsize::new(4).unwrap())
        );
    }

    #[test]
    fn invariant_workers_accepts_auto() {
        let args = TestArgs::parse_from(["foundry-cli", "--invariant-workers", "auto"]);
        assert_eq!(args.invariant_workers, Some(InvariantWorkers::Auto));

        let figment = figment::Figment::from(&args);
        assert_eq!(
            figment.extract_inner::<InvariantWorkers>("invariant.workers").unwrap(),
            InvariantWorkers::Auto
        );
    }

    #[test]
    fn invariant_workers_env_accepts_auto() {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

        let _guard = ENV_LOCK.lock().unwrap();
        let previous = std::env::var_os("FOUNDRY_INVARIANT_WORKERS");
        unsafe {
            std::env::set_var("FOUNDRY_INVARIANT_WORKERS", "auto");
        }

        let args = TestArgs::try_parse_from(["foundry-cli"]);

        unsafe {
            if let Some(previous) = previous {
                std::env::set_var("FOUNDRY_INVARIANT_WORKERS", previous);
            } else {
                std::env::remove_var("FOUNDRY_INVARIANT_WORKERS");
            }
        }

        assert_eq!(args.unwrap().invariant_workers, Some(InvariantWorkers::Auto));
    }

    #[test]
    fn corpus_dir_env_vars_are_parsed() {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

        let _guard = ENV_LOCK.lock().unwrap();
        let previous_fuzz = std::env::var_os("FOUNDRY_FUZZ_CORPUS_DIR");
        let previous_invariant = std::env::var_os("FOUNDRY_INVARIANT_CORPUS_DIR");
        unsafe {
            std::env::set_var("FOUNDRY_FUZZ_CORPUS_DIR", "env_fuzz_corpus");
            std::env::set_var("FOUNDRY_INVARIANT_CORPUS_DIR", "env_invariant_corpus");
        }

        let args = TestArgs::try_parse_from(["foundry-cli"]);

        unsafe {
            if let Some(previous) = previous_fuzz {
                std::env::set_var("FOUNDRY_FUZZ_CORPUS_DIR", previous);
            } else {
                std::env::remove_var("FOUNDRY_FUZZ_CORPUS_DIR");
            }
            if let Some(previous) = previous_invariant {
                std::env::set_var("FOUNDRY_INVARIANT_CORPUS_DIR", previous);
            } else {
                std::env::remove_var("FOUNDRY_INVARIANT_CORPUS_DIR");
            }
        }

        let args = args.unwrap();
        assert_eq!(args.fuzz_corpus_dir, Some(PathBuf::from("env_fuzz_corpus")));
        assert_eq!(args.invariant_corpus_dir, Some(PathBuf::from("env_invariant_corpus")));
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

        let figment = figment::Figment::from(&args);
        assert_eq!(figment.extract_inner::<u32>("fuzz.dictionary_weight").unwrap(), 35);
        assert_eq!(
            figment.extract_inner::<String>("fuzz.max_fuzz_dictionary_addresses").unwrap(),
            "max"
        );
        assert_eq!(
            figment.extract_inner::<String>("fuzz.max_fuzz_dictionary_values").unwrap(),
            "1234"
        );
        assert_eq!(
            figment.extract_inner::<String>("fuzz.max_fuzz_dictionary_literals").unwrap(),
            "4321"
        );
        assert_eq!(figment.extract_inner::<u32>("fuzz.corpus_random_sequence_weight").unwrap(), 55);
        assert_eq!(
            figment.extract_inner::<PathBuf>("fuzz.corpus_dir").unwrap(),
            PathBuf::from("fuzz_corpus")
        );
        assert_eq!(
            figment.extract_inner::<PathBuf>("fuzz.frontier_dir").unwrap(),
            PathBuf::from("fuzz_frontiers")
        );
        assert_eq!(figment.extract_inner::<usize>("fuzz.frontier_limit").unwrap(), 7);
        assert_eq!(figment.extract_inner::<u32>("fuzz.payable_value_weight").unwrap(), 12);
        assert_eq!(figment.extract_inner::<u32>("fuzz.mutation_weight_splice").unwrap(), 4);
        assert_eq!(figment.extract_inner::<u32>("fuzz.mutation_weight_abi").unwrap(), 3);
        assert_eq!(figment.extract_inner::<u32>("fuzz.mutation_weight_cmp").unwrap(), 5);
        assert!(figment.extract_inner::<bool>("symbolic.use_fuzz_frontiers").unwrap());
        assert_eq!(figment.extract_inner::<usize>("symbolic.frontier_limit").unwrap(), 3);
        assert_eq!(figment.extract_inner::<Vec<u64>>("symbolic.frontier_ids").unwrap(), vec![4, 9]);
        assert_eq!(
            figment.extract_inner::<Vec<usize>>("symbolic.frontier_pcs").unwrap(),
            vec![123, 456]
        );
        assert_eq!(
            figment.extract_inner::<Vec<String>>("symbolic.frontier_selectors").unwrap(),
            vec!["0x12345678", "deadbeef"]
        );
        assert_eq!(figment.extract_inner::<u32>("invariant.depth").unwrap(), 300);
        assert_eq!(figment.extract_inner::<u32>("invariant.min_depth").unwrap(), 20);
        assert_eq!(
            figment.extract_inner::<InvariantDepthMode>("invariant.depth_mode").unwrap(),
            InvariantDepthMode::Random
        );
        assert_eq!(figment.extract_inner::<u32>("invariant.dictionary_weight").unwrap(), 45);
        assert_eq!(
            figment.extract_inner::<String>("invariant.max_fuzz_dictionary_addresses").unwrap(),
            "8765"
        );
        assert_eq!(
            figment.extract_inner::<String>("invariant.max_fuzz_dictionary_values").unwrap(),
            "max"
        );
        assert_eq!(
            figment.extract_inner::<String>("invariant.max_fuzz_dictionary_literals").unwrap(),
            "6789"
        );
        assert_eq!(
            figment.extract_inner::<u32>("invariant.corpus_random_sequence_weight").unwrap(),
            25
        );
        assert_eq!(
            figment.extract_inner::<PathBuf>("invariant.corpus_dir").unwrap(),
            PathBuf::from("invariant_corpus")
        );
        assert_eq!(figment.extract_inner::<u32>("invariant.payable_value_weight").unwrap(), 34);
        assert_eq!(figment.extract_inner::<u32>("invariant.mutation_weight_splice").unwrap(), 2);
        assert_eq!(figment.extract_inner::<u32>("invariant.mutation_weight_cmp").unwrap(), 7);

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
        assert_eq!(config.invariant.corpus.payable_value_weight, 34);
        assert_eq!(config.invariant.corpus.mutation_weights.mutation_weight_splice, 2);
        assert_eq!(config.invariant.corpus.mutation_weights.mutation_weight_cmp, 7);
    }

    #[test]
    fn extract_chain() {
        let test = |arg: &str, expected: Chain| {
            let args = TestArgs::parse_from(["foundry-cli", arg]);
            assert_eq!(args.evm.env.chain, Some(expected));
            let (config, evm_opts) = args.load_config_and_evm_opts().unwrap();
            assert_eq!(config.chain, Some(expected));
            assert_eq!(evm_opts.env.chain_id, Some(expected.id()));
        };
        test("--chain-id=1", Chain::mainnet());
        test("--chain-id=42", Chain::from_id(42));
    }
}
