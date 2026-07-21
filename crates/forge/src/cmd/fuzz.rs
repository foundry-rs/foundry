use crate::{
    cmd::test::{CampaignArgs, FilterArgs, FuzzMinimizeReplaySession, ShowmapDomainArg, TestArgs},
    multi_runner::{FuzzMinimizeEdgeIndices, FuzzMinimizeObservation, ShowmapConfig},
    result::TestOutcome,
};
use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{Address, B256, Function as SolFunction, I256, Selector, U256};
use clap::{Parser, Subcommand, ValueEnum, ValueHint};
use eyre::{Context, Result, bail};
use flate2::{Compression, write::GzEncoder};
use foundry_cli::{
    opts::{BuildOpts, EvmArgs, GlobalArgs},
    utils::LoadConfig,
};
use foundry_common::{
    fmt::format_tokens_raw,
    fs, sh_println, sh_status,
    shell::{OutputMode, Shell},
};
use foundry_config::{Config, filter::GlobMatcher};
use foundry_evm::{
    executors::{CorpusDirEntry, ReplayObservation, ShowmapDomain, read_corpus_tree},
    fuzz::BasicTxDetails,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    pin::Pin,
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::{Builder as TempDirBuilder, TempDir};

type FuzzOutcomeFuture = Pin<Box<dyn Future<Output = Result<TestOutcome>>>>;

// Repeated cold-corpus trials on Nerite's unchanged LiquidationsLST fuzz target found the same
// coverage gain within these bounds. Keep bootstrap work deliberately small; users can raise the
// existing campaign dials when a target warrants a larger budget.
const DEFAULT_SEED_WARMUP_RUNS: u64 = 16;
const DEFAULT_SEED_FRONTIER_LIMIT: usize = 32;
const DEFAULT_SEED_SOLVER_TIMEOUT_SECS: u32 = 1;

/// Run and manage Forge fuzzing corpora.
#[derive(Clone, Debug, Parser)]
pub struct FuzzArgs {
    #[command(subcommand)]
    pub command: FuzzSubcommands,
}

impl FuzzArgs {
    pub fn run(self) -> FuzzOutcomeFuture {
        match self.command {
            FuzzSubcommands::Run(args) => {
                let mut test = TestArgs::from_fuzz_run(args);
                test.enable_fuzz_only_with_auto_fuzz_corpus();
                Box::pin(test.run())
            }
            FuzzSubcommands::Seed(args) => Box::pin(args.run()),
            FuzzSubcommands::Replay(args) => Box::pin(args.run()),
            FuzzSubcommands::Show(args) => Box::pin(async move {
                args.run()?;
                Ok(TestOutcome::empty(None, true))
            }),
            FuzzSubcommands::Cmin(args) => Box::pin(async move {
                args.run().await?;
                Ok(TestOutcome::empty(None, true))
            }),
            FuzzSubcommands::Tmin(args) => Box::pin(async move {
                args.run().await?;
                Ok(TestOutcome::empty(None, true))
            }),
        }
    }

    pub const fn is_junit(&self) -> bool {
        match &self.command {
            FuzzSubcommands::Run(args) => args.junit,
            FuzzSubcommands::Seed(args) => args.run.junit,
            FuzzSubcommands::Replay(args) => args.is_junit(),
            FuzzSubcommands::Show(_) | FuzzSubcommands::Cmin(_) | FuzzSubcommands::Tmin(_) => false,
        }
    }
}

#[derive(Clone, Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum FuzzSubcommands {
    /// Run fuzz and invariant campaigns, automatically bootstrapping eligible stale stateless
    /// corpora.
    ///
    /// A stateless target with scalar ABI inputs, at least 2,000,000 configured runs, and no
    /// timeout below 15 minutes gets one bounded symbolic corpus bootstrap before concrete
    /// fuzzing. The prelude tries at most 32 frontiers for one second each with a 256 MB Z3 or
    /// Bitwuzla limit, and persists a passing candidate only after concrete replay adds an EVM
    /// edge. Replay, showmap, list, rerun, gas-report, fork-backed, FFI-enabled, and
    /// persisted-failure targets do not bootstrap.
    Run(FuzzRunArgs),
    /// Force a bounded symbolic bootstrap of a stateless fuzz corpus, then exit.
    ///
    /// This diagnostic form retains every exact branch-flipping concrete replay. Use `forge fuzz
    /// show`, `forge fuzz replay`, and showmap afterward to inspect its value.
    Seed(FuzzSeedArgs),
    /// Replay persisted fuzz failures, or corpus entries with `--corpus-dir`.
    Replay(FuzzReplayArgs),
    /// Print persisted corpus entries.
    Show(FuzzShowArgs),
    /// Minimize a corpus by keeping entries that contribute new coverage.
    Cmin(FuzzCminArgs),
    /// Minimize one corpus entry while preserving its failure or coverage.
    Tmin(FuzzTminArgs),
}

/// Run fuzz and invariant campaigns. Effectively long stateless campaigns automatically bootstrap
/// an empty or stale corpus once before concrete fuzzing.
#[derive(Clone, Debug, Parser)]
#[command(
    long_about = "Run fuzz and invariant campaigns.\n\nFor a stateless target with scalar ABI inputs, at least 2,000,000 configured runs, and no timeout below 15 minutes, Forge automatically gives one empty or stale corpus a bounded symbolic bootstrap before concrete fuzzing. It uses one available memory-bounded built-in solver, concretely replays candidates, and preserves the requested campaign bounds. Replay, showmap, list, rerun, gas-report, fork-backed, FFI-enabled, and persisted-failure targets do not bootstrap."
)]
pub struct FuzzRunArgs {
    #[command(flatten)]
    pub(crate) global: GlobalArgs,

    /// The contract file you want to test, it's a shortcut for --match-path.
    #[arg(value_hint = ValueHint::FilePath)]
    pub(crate) path: Option<GlobMatcher>,

    #[command(flatten)]
    pub(crate) filter: FilterArgs,

    #[command(flatten)]
    pub(crate) campaign: CampaignArgs,

    #[command(flatten)]
    pub(crate) evm: EvmArgs,

    #[command(flatten)]
    pub(crate) build: BuildOpts,

    /// Output test results as JUnit XML report.
    #[arg(long, conflicts_with_all = ["quiet", "json", "gas_report", "list", "show_progress"], help_heading = "Display options")]
    pub(crate) junit: bool,

    /// Exit with code 0 even if a test fails.
    #[arg(long, env = "FORGE_ALLOW_FAILURE")]
    pub(crate) allow_failure: bool,

    /// Stop running tests after the first failure.
    #[arg(long)]
    pub(crate) fail_fast: bool,

    /// Re-run recorded test failures from last run.
    /// If no failure recorded then regular test run is performed.
    #[arg(long)]
    pub(crate) rerun: bool,

    /// Show test execution progress.
    #[arg(long, conflicts_with_all = ["quiet", "json"], help_heading = "Display options")]
    pub(crate) show_progress: bool,

    /// The Etherscan (or equivalent) API key.
    #[arg(long, env = "ETHERSCAN_API_KEY", value_name = "KEY")]
    pub(crate) etherscan_api_key: Option<String>,

    /// List fuzz and invariant tests instead of running them.
    #[arg(long, short, conflicts_with_all = ["show_progress"], help_heading = "Display options")]
    pub(crate) list: bool,

    /// Print a gas report.
    #[arg(long, env = "FORGE_GAS_REPORT")]
    pub(crate) gas_report: bool,

    /// Replay the persisted corpus and emit AFL-`afl-showmap`-style coverage
    /// files at the given output directory.
    #[arg(
        long,
        value_name = "DIR",
        value_hint = ValueHint::DirPath,
        help_heading = "Showmap replay",
        conflicts_with_all = ["rerun", "fuzz_input_file", "gas_report"],
    )]
    pub(crate) showmap_out: Option<PathBuf>,

    /// Emit one showmap file per corpus entry (default: one aggregated file per test).
    #[arg(long, help_heading = "Showmap replay", requires = "showmap_out")]
    pub(crate) showmap_per_input: bool,

    /// Coverage domain(s) to dump.
    #[arg(
        long,
        value_enum,
        default_value_t = ShowmapDomainArg::Evm,
        help_heading = "Showmap replay",
        requires = "showmap_out",
    )]
    pub(crate) showmap_domain: ShowmapDomainArg,

    /// Approach name (used as a subdirectory of `--showmap-out`).
    #[arg(
        long,
        default_value = "replay",
        help_heading = "Showmap replay",
        requires = "showmap_out"
    )]
    pub(crate) showmap_approach: String,

    /// Trial identifier embedded in each showmap filename.
    #[arg(long, help_heading = "Showmap replay", requires = "showmap_out")]
    pub(crate) showmap_trial: Option<String>,

    /// Override the corpus directory to replay.
    #[arg(
        long,
        value_name = "PATH",
        value_hint = ValueHint::DirPath,
        help_heading = "Showmap replay",
        requires = "showmap_out",
    )]
    pub(crate) showmap_corpus_dir: Option<PathBuf>,

    /// File to rerun fuzz failures from.
    #[arg(long)]
    pub(crate) fuzz_input_file: Option<String>,
}

/// Force or inspect the bounded stateless corpus bootstrap without starting a concrete campaign.
#[derive(Clone, Debug, Parser)]
#[command(
    long_about = "Force or inspect the bounded symbolic bootstrap used automatically by eligible long stateless fuzz campaigns, then exit. This diagnostic form retains every exact branch-flipping concrete replay; use show, replay, and showmap afterward to inspect its value."
)]
pub struct FuzzSeedArgs {
    #[command(flatten)]
    run: FuzzRunArgs,

    /// Solver timeout in seconds for each retained branch frontier.
    #[arg(long, default_value_t = DEFAULT_SEED_SOLVER_TIMEOUT_SECS, value_name = "SECONDS")]
    solver_timeout: u32,
}

impl FuzzSeedArgs {
    async fn run(mut self) -> Result<TestOutcome> {
        if self.solver_timeout == 0 {
            bail!("--solver-timeout must be greater than 0");
        }
        if self.run.list {
            bail!("`forge fuzz seed` cannot be combined with --list; use `forge fuzz run --list`");
        }
        if self.run.showmap_out.is_some() {
            bail!("`forge fuzz seed` cannot be combined with --showmap-out");
        }

        let frontier_limit =
            self.run.campaign.frontier_limit.unwrap_or(DEFAULT_SEED_FRONTIER_LIMIT);
        if frontier_limit == 0 {
            bail!("--frontier-limit must be greater than 0");
        }

        let temporary_frontiers = if self.run.campaign.frontier_dir.is_none() {
            let dir = TempDirBuilder::new().prefix("forge-fuzz-seed-frontiers-").tempdir()?;
            self.run.campaign.frontier_dir = Some(dir.path().to_path_buf());
            Some(dir)
        } else {
            None
        };
        self.run.campaign.frontier_limit = Some(frontier_limit);
        self.run.campaign.runs.get_or_insert(DEFAULT_SEED_WARMUP_RUNS);

        sh_status!(
            "Collecting branch frontiers with {} concrete runs per test",
            self.run.campaign.runs.unwrap_or(DEFAULT_SEED_WARMUP_RUNS)
        )?;
        let mut warmup = TestArgs::from_fuzz_run(self.run.clone());
        warmup.enable_fuzz_seed_warmup();
        let outcome = warmup.run().await?;
        if outcome.failures().next().is_some() {
            return Ok(outcome);
        }

        sh_status!(
            "Seeding fuzz corpus from at most {frontier_limit} branch frontiers ({timeout}s each)",
            timeout = self.solver_timeout
        )?;
        self.run.campaign.runs = Some(1);
        let mut seed = TestArgs::from_fuzz_run(self.run);
        seed.enable_fuzz_seed_only();
        seed.symbolic_use_fuzz_frontiers = true;
        seed.symbolic_frontier_limit = Some(frontier_limit);
        seed.symbolic_timeout = Some(self.solver_timeout);

        // The temporary artifact directory must outlive both delegated test invocations.
        let _temporary_frontiers = temporary_frontiers;
        seed.run().await
    }
}

/// Replay persisted fuzz failures, or corpus entries with `--corpus-dir`.
#[derive(Clone, Debug, Parser)]
pub struct FuzzReplayArgs {
    #[command(flatten)]
    run: FuzzRunArgs,
}

impl FuzzReplayArgs {
    async fn run(self) -> Result<TestOutcome> {
        let corpus_dir = self.run.campaign.corpus_dir.clone();
        let mut test = TestArgs::from_fuzz_run(self.run);
        if corpus_dir.is_none() {
            test.enable_fuzz_failure_replay();
            return test.run().await;
        }

        let replay_id =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
        test.set_showmap_override(ShowmapConfig {
            out_dir: std::env::temp_dir().join(format!("forge-fuzz-replay-{replay_id}")),
            approach: "replay".to_string(),
            trial: "replay".to_string(),
            per_input: false,
            domain: ShowmapDomain::Evm,
            corpus_dir,
            emit_files: false,
        });
        test.run().await
    }

    const fn is_junit(&self) -> bool {
        self.run.junit
    }
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum CorpusShowFormat {
    #[default]
    Human,
    Json,
}

/// Print persisted corpus entries.
#[derive(Clone, Debug, Parser)]
pub struct FuzzShowArgs {
    /// Corpus directory or a single corpus file.
    #[arg(value_name = "PATH", value_hint = ValueHint::AnyPath)]
    corpus: PathBuf,
    /// Output format.
    #[arg(long, value_enum, default_value_t)]
    format: CorpusShowFormat,
    /// Maximum number of entries to print.
    #[arg(long, value_name = "N")]
    limit: Option<usize>,
}

impl FuzzShowArgs {
    fn run(&self) -> Result<()> {
        let decoder = CorpusDecoder::load();
        let entries = read_entries(&self.corpus, self.limit, &decoder)?;
        match self.format {
            CorpusShowFormat::Human => {
                for entry in entries {
                    sh_println!("{} ({} txs)", entry.path.display(), entry.sequence.len())?;
                    for (idx, tx) in entry.sequence.iter().enumerate() {
                        if let Some(decoded) = &tx.decoded {
                            let ambiguity = if decoded.ambiguous_contracts.is_empty() {
                                String::new()
                            } else {
                                format!(" ambiguous=[{}]", decoded.ambiguous_contracts.join(","))
                            };
                            sh_println!(
                                "  {idx}: {} sender={} target={} value={}{}",
                                decoded.call,
                                tx.raw.sender,
                                tx.raw.call_details.target,
                                tx.raw
                                    .call_details
                                    .value
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "0".to_string()),
                                ambiguity
                            )?;
                        } else {
                            sh_println!(
                                "  {idx}: target={} sender={} calldata={} value={}",
                                tx.raw.call_details.target,
                                tx.raw.sender,
                                tx.raw.call_details.calldata,
                                tx.raw
                                    .call_details
                                    .value
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "0".to_string())
                            )?;
                        }
                    }
                }
            }
            CorpusShowFormat::Json => sh_println!("{}", serde_json::to_string_pretty(&entries)?)?,
        }
        Ok(())
    }
}

/// Minimize a corpus by keeping entries that contribute new coverage.
#[derive(Clone, Debug, Parser)]
pub struct FuzzCminArgs {
    #[command(flatten)]
    test: FuzzMinimizeTestArgs,
    /// Input corpus directory.
    #[arg(value_name = "CORPUS_DIR", value_hint = ValueHint::DirPath)]
    corpus_dir: PathBuf,
    /// Output corpus directory.
    #[arg(long = "corpus-out", value_name = "DIR", value_hint = ValueHint::DirPath)]
    out: PathBuf,
}

impl FuzzCminArgs {
    async fn run(self) -> Result<()> {
        if cmin_out_exists(&self.out) {
            bail!("output corpus directory already exists: {}", self.out.display());
        }

        let staging_out = temporary_cmin_out(&self.out)?;
        let summary = self.run_to(staging_out.path()).await?;
        let staging_path = staging_out.keep();

        if cmin_out_exists(&self.out) {
            bail!(
                "output corpus directory already exists: {}; minimized corpus remains staged at {}",
                self.out.display(),
                staging_path.display()
            );
        }

        std::fs::rename(&staging_path, &self.out).with_context(|| {
            format!(
                "failed to rename minimized corpus {} to {}",
                staging_path.display(),
                self.out.display()
            )
        })?;

        sh_println!(
            "minimized corpus: kept {}/{} entries in {}",
            summary.kept,
            summary.total,
            self.out.display()
        )?;
        if summary.skipped > 0 {
            sh_status!(
                "skipped {} entries or txs that could not be read or replayed",
                summary.skipped
            )?;
        }
        Ok(())
    }

    async fn run_to(&self, out_dir: &Path) -> Result<CminSummary> {
        let session = self.test.clone().prepare_session(&self.corpus_dir).await?;
        let mut kept = 0usize;
        let mut total = 0usize;
        let mut skipped_entries = 0usize;
        let mut unreadable = 0usize;
        let mut empty = 0usize;
        let mut unmatched_txs = 0usize;
        let mut rejected_txs = 0usize;
        let mut failed_entries = 0usize;
        let mut failed_replays = 0usize;
        let mut replayed = 0usize;
        let mut cumulative = BTreeMap::<String, ReplayObservation>::new();
        let evm_edge_indices = FuzzMinimizeEdgeIndices::default();

        for entry in read_corpus_entries(&self.corpus_dir)? {
            total += 1;
            let sequence = entry
                .read_tx_seq()
                .with_context(|| format!("failed to read corpus entry {}", entry.path.display()));
            let Ok(sequence) = sequence else {
                skipped_entries += 1;
                unreadable += 1;
                continue;
            };
            if sequence.is_empty() {
                skipped_entries += 1;
                empty += 1;
                continue;
            }
            let observations = replay_candidate(&session, evm_edge_indices.clone(), sequence)?;
            let mut entry_improved = false;
            let mut entry_failed = false;
            let mut entry_failed_replays = 0usize;
            let mut entry_replayed = 0usize;
            let mut entry_unmatched_txs = 0usize;
            let mut entry_rejected_txs = 0usize;
            for FuzzMinimizeObservation { target, observation } in observations {
                if observation.failure.is_some() {
                    entry_failed = true;
                    entry_failed_replays += observation.replayed;
                    continue;
                }
                entry_replayed += observation.replayed;
                entry_unmatched_txs = entry_unmatched_txs.max(observation.unmatched);
                entry_rejected_txs = entry_rejected_txs.max(observation.skipped);
                let cumulative = cumulative.entry(target).or_default();
                entry_improved |= merge_new_edges(cumulative, &observation);
            }
            if entry_replayed > 0 {
                replayed += entry_replayed;
            } else if entry_failed {
                skipped_entries += 1;
                failed_entries += 1;
                failed_replays += entry_failed_replays;
            } else {
                unmatched_txs += entry_unmatched_txs;
                rejected_txs += entry_rejected_txs;
            }
            if !entry_improved {
                continue;
            }

            let out = if self.corpus_dir.is_file() {
                out_dir.join(entry.path.file_name().unwrap_or_default())
            } else {
                let relative = entry.path.strip_prefix(&self.corpus_dir).with_context(|| {
                    format!(
                        "corpus entry {} is not under {}",
                        entry.path.display(),
                        self.corpus_dir.display()
                    )
                })?;
                out_dir.join(relative)
            };
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            std::fs::copy(&entry.path, &out).with_context(|| {
                format!("failed to copy {} to {}", entry.path.display(), out.display())
            })?;
            kept += 1;
        }

        if total > 0 && replayed == 0 {
            let corpus = self.corpus_dir.display();
            if unreadable == total {
                bail!(
                    "replayed 0 transactions from {corpus}; all {unreadable} corpus entries could not be read"
                );
            }
            if failed_entries > 0 {
                bail!(
                    "replayed 0 successful transactions from {corpus}; {failed_entries} corpus \
                     entries failed during replay after {failed_replays} replayed transactions"
                );
            }
            if unmatched_txs > 0 {
                bail!(
                    "replayed 0 transactions from {corpus}; {unmatched_txs} transactions did not match \
                     the test; check that --mc/--mt and replay-critical options match the corpus \
                     entries"
                );
            }
            if rejected_txs > 0 {
                bail!(
                    "replayed 0 transactions from {corpus}; {rejected_txs} transactions were rejected \
                     by vm.assume or vm.skip"
                );
            }
            if empty == total.saturating_sub(unreadable) {
                bail!(
                    "replayed 0 transactions from {corpus}; corpus entries were empty{}",
                    if unreadable > 0 {
                        format!(" or unreadable ({unreadable} unreadable)")
                    } else {
                        String::new()
                    }
                );
            }
            bail!(
                "replayed 0 transactions from {corpus}; {unreadable} unreadable entries, {empty} \
                 empty entries"
            );
        }

        Ok(CminSummary { kept, total, skipped: skipped_entries + unmatched_txs + rejected_txs })
    }
}

fn cmin_out_exists(out: &Path) -> bool {
    std::fs::symlink_metadata(out).is_ok()
}

struct CminSummary {
    kept: usize,
    total: usize,
    skipped: usize,
}

/// Minimize one corpus entry while preserving its failure or coverage.
#[derive(Clone, Debug, Parser)]
pub struct FuzzTminArgs {
    #[command(flatten)]
    test: FuzzMinimizeTestArgs,
    /// Input corpus file or directory.
    #[arg(value_name = "INPUT", value_hint = ValueHint::AnyPath)]
    input: PathBuf,
    /// Output corpus file or directory.
    #[arg(long = "corpus-out", value_name = "PATH", value_hint = ValueHint::AnyPath)]
    out: PathBuf,
    /// Maximum candidate replays to attempt per corpus entry.
    #[arg(long, default_value_t = 5000, value_name = "N")]
    max_attempts: usize,
}

impl FuzzTminArgs {
    async fn run(self) -> Result<()> {
        if self.max_attempts == 0 {
            bail!("--max-attempts must be greater than 0");
        }
        if self.input.is_dir() { self.run_dir().await } else { self.run_file().await }
    }

    async fn run_file(self) -> Result<()> {
        validate_tmin_output_path(&self.out)?;

        let mut sequence = read_single_sequence(&self.input)?;
        if sequence.is_empty() {
            bail!("corpus entry {} is empty", self.input.display());
        }

        let before_txs = sequence.len();
        let decoder_args = self.test.clone();
        let session = self.test.prepare_session(input_corpus_root(&self.input)).await?;
        let decoder = decoder_args.decoder();
        let attempts =
            minimize_entry(&session, &decoder, &self.input, &mut sequence, self.max_attempts)?;
        write_sequence_create_new(&self.out, &sequence)?;

        sh_println!(
            "minimized entry: {before_txs} txs -> {} txs in {}",
            sequence.len(),
            self.out.display()
        )?;
        sh_status!("attempted {attempts} candidate replays")?;
        Ok(())
    }

    async fn run_dir(self) -> Result<()> {
        if cmin_out_exists(&self.out) {
            bail!("output corpus directory already exists: {}", self.out.display());
        }

        let entries = read_corpus_entries(&self.input)?;
        let staging_out = temporary_cmin_out(&self.out)?;
        let decoder_args = self.test.clone();
        let session = self.test.prepare_session(&self.input).await?;
        let decoder = decoder_args.decoder();
        let mut total_entries = 0usize;
        let mut before_txs = 0usize;
        let mut after_txs = 0usize;
        let mut attempts = 0usize;
        let mut skipped_entries = 0usize;

        for entry in entries {
            let sequence = entry
                .read_tx_seq()
                .with_context(|| format!("failed to read corpus entry {}", entry.path.display()));
            let Ok(mut sequence) = sequence else {
                skipped_entries += 1;
                continue;
            };
            if sequence.is_empty() {
                skipped_entries += 1;
                continue;
            }
            before_txs += sequence.len();
            attempts +=
                minimize_entry(&session, &decoder, &entry.path, &mut sequence, self.max_attempts)?;
            after_txs += sequence.len();

            let relative = entry.path.strip_prefix(&self.input).with_context(|| {
                format!(
                    "corpus entry {} is not under {}",
                    entry.path.display(),
                    self.input.display()
                )
            })?;
            write_sequence_create_new(&staging_out.path().join(relative), &sequence)?;
            total_entries += 1;
        }
        if total_entries == 0 {
            bail!("no readable non-empty corpus entries found under {}", self.input.display());
        }

        let staging_path = staging_out.keep();
        if cmin_out_exists(&self.out) {
            bail!(
                "output corpus directory already exists: {}; minimized corpus remains staged at {}",
                self.out.display(),
                staging_path.display()
            );
        }
        std::fs::rename(&staging_path, &self.out).with_context(|| {
            format!(
                "failed to rename minimized corpus {} to {}",
                staging_path.display(),
                self.out.display()
            )
        })?;

        sh_println!(
            "minimized corpus: {total_entries} entries, {before_txs} txs -> {after_txs} txs in {}",
            self.out.display()
        )?;
        sh_status!("attempted {attempts} candidate replays")?;
        if skipped_entries > 0 {
            sh_status!("skipped {skipped_entries} entries that could not be read or were empty")?;
        }
        Ok(())
    }
}

fn minimize_entry(
    session: &FuzzMinimizeReplaySession,
    decoder: &CorpusDecoder,
    input: &Path,
    sequence: &mut Vec<BasicTxDetails>,
    max_attempts: usize,
) -> Result<usize> {
    let evm_edge_indices = FuzzMinimizeEdgeIndices::default();
    let baseline = replay_baseline(session, evm_edge_indices.clone(), sequence.clone())
        .with_context(|| format!("failed to replay baseline corpus entry {}", input.display()))?;
    if baseline.requirements.is_empty() {
        bail!(
            "replayed 0 transactions from {}; check that --mc/--mt and replay-critical options match the corpus entry",
            input.display()
        );
    }
    if !baseline.has_failure && !baseline.has_coverage {
        bail!("baseline replay for {} produced no failure or edge coverage", input.display());
    }

    let mut ctx = MinimizeContext::new(session, evm_edge_indices, baseline, max_attempts);
    minimize_sequence(&mut ctx, sequence, decoder)?;
    Ok(ctx.attempts)
}

struct ReplayBaseline {
    requirements: BTreeMap<String, ReplayObservation>,
    has_failure: bool,
    has_coverage: bool,
}

fn replay_baseline(
    session: &FuzzMinimizeReplaySession,
    evm_edge_indices: FuzzMinimizeEdgeIndices,
    sequence: Vec<BasicTxDetails>,
) -> Result<ReplayBaseline> {
    let observations = replay_candidate(session, evm_edge_indices, sequence)?;
    let mut requirements = BTreeMap::new();
    let mut has_failure = false;
    let mut has_coverage = false;
    for FuzzMinimizeObservation { target, observation } in observations {
        let observation_has_failure = observation.failure.is_some();
        let observation_has_coverage = has_edges(&observation);
        if observation.replayed == 0 && !observation_has_failure && !observation_has_coverage {
            continue;
        }
        has_failure |= observation_has_failure;
        has_coverage |= observation_has_coverage;
        requirements.insert(target, observation);
    }
    Ok(ReplayBaseline { requirements, has_failure, has_coverage })
}

struct MinimizeContext<'a> {
    session: &'a FuzzMinimizeReplaySession,
    evm_edge_indices: FuzzMinimizeEdgeIndices,
    baseline: ReplayBaseline,
    max_attempts: usize,
    attempts: usize,
}

impl<'a> MinimizeContext<'a> {
    const fn new(
        session: &'a FuzzMinimizeReplaySession,
        evm_edge_indices: FuzzMinimizeEdgeIndices,
        baseline: ReplayBaseline,
        max_attempts: usize,
    ) -> Self {
        Self { session, evm_edge_indices, baseline, max_attempts, attempts: 0 }
    }

    const fn at_budget(&self) -> bool {
        self.attempts >= self.max_attempts
    }

    const fn remaining_attempts(&self) -> usize {
        self.max_attempts.saturating_sub(self.attempts)
    }

    fn accepts(&mut self, candidate: &[BasicTxDetails]) -> Result<bool> {
        if self.at_budget() {
            return Ok(false);
        }
        self.attempts += 1;
        let observations =
            replay_candidate(self.session, self.evm_edge_indices.clone(), candidate.to_vec())?;
        let observations = observations
            .into_iter()
            .map(|obs| (obs.target, obs.observation))
            .collect::<BTreeMap<_, _>>();

        if has_new_active_targets(&observations, &self.baseline.requirements) {
            return Ok(false);
        }

        for (target, baseline) in &self.baseline.requirements {
            let Some(candidate) = observations.get(target) else {
                return Ok(false);
            };
            if let Some(failure) = &baseline.failure {
                if candidate.failure.as_ref() != Some(failure) {
                    return Ok(false);
                }
            } else if candidate.failure.is_some() || !same_edge_hit_sets(candidate, baseline) {
                return Ok(false);
            }
        }

        Ok(true)
    }
}

fn has_new_active_targets(
    observations: &BTreeMap<String, ReplayObservation>,
    baseline: &BTreeMap<String, ReplayObservation>,
) -> bool {
    observations.iter().any(|(target, observation)| {
        !baseline.contains_key(target)
            && (observation.failure.is_some() || observation.replayed > 0 || has_edges(observation))
    })
}

fn minimize_sequence(
    ctx: &mut MinimizeContext<'_>,
    sequence: &mut Vec<BasicTxDetails>,
    decoder: &CorpusDecoder,
) -> Result<()> {
    let mut idx = 0;
    while idx < sequence.len() && !ctx.at_budget() {
        let removed = sequence.remove(idx);
        if ctx.accepts(sequence)? {
            continue;
        }
        sequence.insert(idx, removed);
        idx += 1;
    }

    let mut idx = 0;
    while idx < sequence.len() && !ctx.at_budget() {
        let restore = sequence[idx].clone();
        cleanup_metadata(&mut sequence[idx]);
        if !ctx.accepts(sequence)? {
            sequence[idx] = restore;
        }
        idx += 1;
    }

    let mut tx_idx = 0;
    while tx_idx < sequence.len() && !ctx.at_budget() {
        loop {
            let candidates = abi_calldata_candidates(
                sequence[tx_idx].call_details.calldata.as_ref(),
                decoder,
                ctx.remaining_attempts(),
            );
            if candidates.is_empty() {
                break;
            };

            let mut accepted = false;
            for calldata in candidates {
                if calldata.len() > sequence[tx_idx].call_details.calldata.len() {
                    continue;
                }
                let restore =
                    std::mem::replace(&mut sequence[tx_idx].call_details.calldata, calldata.into());
                if ctx.accepts(sequence)? {
                    accepted = true;
                    break;
                }
                sequence[tx_idx].call_details.calldata = restore;
                if ctx.at_budget() {
                    break;
                }
            }
            if !accepted || ctx.at_budget() {
                break;
            }
        }
        tx_idx += 1;
    }

    Ok(())
}

fn temporary_cmin_out(out: &Path) -> Result<TempDir> {
    let parent =
        out.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let filename =
        out.file_name().ok_or_else(|| eyre::eyre!("missing output corpus directory name"))?;
    let prefix = format!(".{}.tmp-", filename.to_string_lossy());
    TempDirBuilder::new().prefix(&prefix).tempdir_in(parent).with_context(|| {
        format!("failed to create temporary output directory for {}", out.display())
    })
}

fn input_corpus_root(input: &Path) -> &Path {
    input.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."))
}

fn read_single_sequence(path: &Path) -> Result<Vec<BasicTxDetails>> {
    let entries = read_corpus_tree(path)?;
    let [entry] = entries.as_slice() else {
        bail!("expected one corpus entry at {}, found {}", path.display(), entries.len());
    };
    entry
        .read_tx_seq()
        .with_context(|| format!("failed to read corpus entry {}", entry.path.display()))
}

fn validate_tmin_output_path(path: &Path) -> Result<()> {
    if std::fs::symlink_metadata(path).is_ok() {
        bail!("output corpus file already exists: {}", path.display());
    }
    Ok(())
}

fn write_sequence_create_new(path: &Path, sequence: &[BasicTxDetails]) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("failed to create output corpus file {}", path.display()))?;
    if is_gzip_path(path) {
        let writer = BufWriter::new(file);
        let mut encoder = GzEncoder::new(writer, Compression::default());
        serde_json::to_writer(&mut encoder, &sequence)
            .with_context(|| format!("failed to write output corpus file {}", path.display()))?;
        let mut writer = encoder
            .finish()
            .with_context(|| format!("failed to finish output corpus file {}", path.display()))?;
        writer
            .flush()
            .with_context(|| format!("failed to flush output corpus file {}", path.display()))?;
    } else {
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, &sequence)
            .with_context(|| format!("failed to write output corpus file {}", path.display()))?;
        writer
            .flush()
            .with_context(|| format!("failed to flush output corpus file {}", path.display()))?;
    }
    Ok(())
}

fn is_gzip_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("gz"))
}

fn has_edges(observation: &ReplayObservation) -> bool {
    observation.evm_edges.iter().any(|&edge| edge != 0)
        || observation.sancov_edges.iter().any(|&edge| edge != 0)
}

fn same_edge_hit_sets(candidate: &ReplayObservation, baseline: &ReplayObservation) -> bool {
    same_edge_hit_set(&candidate.evm_edges, &baseline.evm_edges)
        && same_edge_hit_set(&candidate.sancov_edges, &baseline.sancov_edges)
}

fn same_edge_hit_set(candidate: &[u8], baseline: &[u8]) -> bool {
    let len = candidate.len().max(baseline.len());
    (0..len).all(|idx| {
        let candidate_hit = candidate.get(idx).copied().unwrap_or_default() != 0;
        let baseline_hit = baseline.get(idx).copied().unwrap_or_default() != 0;
        candidate_hit == baseline_hit
    })
}

fn cleanup_metadata(tx: &mut BasicTxDetails) {
    if tx.warp == Some(U256::ZERO) {
        tx.warp = None;
    }
    if tx.roll == Some(U256::ZERO) {
        tx.roll = None;
    }
    if tx.call_details.value == Some(U256::ZERO) {
        tx.call_details.value = None;
    }
}

fn abi_calldata_candidates(calldata: &[u8], decoder: &CorpusDecoder, limit: usize) -> Vec<Vec<u8>> {
    if limit == 0 {
        return Vec::new();
    }
    let Some((function, args)) = decoder.unique_decodable_function(calldata) else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    for arg_idx in 0..args.len() {
        for value in value_candidates(&args[arg_idx], limit.saturating_sub(candidates.len())) {
            let mut candidate_args = args.clone();
            candidate_args[arg_idx] = value;
            let Ok(encoded) = function.abi_encode_input(&candidate_args) else {
                continue;
            };
            if encoded.as_slice() != calldata && !candidates.contains(&encoded) {
                candidates.push(encoded);
                if candidates.len() >= limit {
                    return candidates;
                }
            }
        }
    }
    candidates
}

fn value_candidates(value: &DynSolValue, limit: usize) -> Vec<DynSolValue> {
    let mut candidates = Vec::new();
    push_scalar_value_candidates(value, &mut candidates, limit);
    push_compound_value_candidates(value, &mut candidates, limit);
    candidates.into_iter().filter(|candidate| candidate != value).collect()
}

fn push_candidate(candidates: &mut Vec<DynSolValue>, limit: usize, candidate: DynSolValue) -> bool {
    if candidates.len() >= limit {
        return false;
    }
    candidates.push(candidate);
    true
}

fn push_scalar_value_candidates(
    value: &DynSolValue,
    candidates: &mut Vec<DynSolValue>,
    limit: usize,
) {
    match value {
        DynSolValue::Bool(_) => {
            push_candidate(candidates, limit, DynSolValue::Bool(false));
        }
        DynSolValue::Uint(value, bits) => {
            if *value != U256::ZERO {
                push_candidate(candidates, limit, DynSolValue::Uint(U256::ZERO, *bits));
            }
            if *value > U256::from(1) {
                push_candidate(candidates, limit, DynSolValue::Uint(U256::from(1), *bits));
            }
        }
        DynSolValue::Int(value, bits) => {
            if *value != I256::ZERO {
                push_candidate(candidates, limit, DynSolValue::Int(I256::ZERO, *bits));
            }
            if *value != I256::ZERO
                && *value != I256::from_raw(U256::from(1))
                && *value != I256::MINUS_ONE
            {
                push_candidate(
                    candidates,
                    limit,
                    DynSolValue::Int(I256::from_raw(U256::from(1)), *bits),
                );
            }
            if *value != I256::ZERO
                && *value != I256::from_raw(U256::from(1))
                && *value != I256::MINUS_ONE
            {
                push_candidate(candidates, limit, DynSolValue::Int(I256::MINUS_ONE, *bits));
            }
        }
        DynSolValue::Address(_) => {
            push_candidate(candidates, limit, DynSolValue::Address(Address::ZERO));
        }
        DynSolValue::FixedBytes(_, size) => {
            push_candidate(candidates, limit, DynSolValue::FixedBytes(B256::ZERO, *size));
        }
        DynSolValue::Function(_) => {
            push_candidate(candidates, limit, DynSolValue::Function(SolFunction::ZERO));
        }
        DynSolValue::Bytes(bytes) => {
            push_candidate(candidates, limit, DynSolValue::Bytes(Vec::new()));
            if bytes.len() > 1 {
                push_candidate(
                    candidates,
                    limit,
                    DynSolValue::Bytes(bytes[..bytes.len() / 2].to_vec()),
                );
            }
        }
        DynSolValue::String(string) => {
            push_candidate(candidates, limit, DynSolValue::String(String::new()));
            if string.len() > 1 {
                let mut half = string.len() / 2;
                while half > 0 && !string.is_char_boundary(half) {
                    half -= 1;
                }
                push_candidate(candidates, limit, DynSolValue::String(string[..half].to_string()));
            }
        }
        DynSolValue::Array(_)
        | DynSolValue::FixedArray(_)
        | DynSolValue::Tuple(_)
        | DynSolValue::CustomStruct { .. } => {}
    }
}

fn push_compound_value_candidates(
    value: &DynSolValue,
    candidates: &mut Vec<DynSolValue>,
    limit: usize,
) {
    match value {
        DynSolValue::Array(values) => {
            push_candidate(candidates, limit, DynSolValue::Array(Vec::new()));
            if values.len() > 1 {
                push_candidate(
                    candidates,
                    limit,
                    DynSolValue::Array(values[..values.len() / 2].to_vec()),
                );
            }
            push_child_value_candidates(values, candidates, limit, |values| {
                DynSolValue::Array(values.to_vec())
            });
        }
        DynSolValue::FixedArray(values) => {
            push_child_value_candidates(values, candidates, limit, |values| {
                DynSolValue::FixedArray(values.to_vec())
            });
        }
        DynSolValue::Tuple(values) => {
            push_child_value_candidates(values, candidates, limit, |values| {
                DynSolValue::Tuple(values.to_vec())
            });
        }
        DynSolValue::CustomStruct { name, prop_names, tuple } => {
            push_child_value_candidates(tuple, candidates, limit, |values| {
                DynSolValue::CustomStruct {
                    name: name.clone(),
                    prop_names: prop_names.clone(),
                    tuple: values.to_vec(),
                }
            });
        }
        DynSolValue::Bool(_)
        | DynSolValue::Uint(_, _)
        | DynSolValue::Int(_, _)
        | DynSolValue::Address(_)
        | DynSolValue::FixedBytes(_, _)
        | DynSolValue::Function(_)
        | DynSolValue::Bytes(_)
        | DynSolValue::String(_) => {}
    }
}

fn push_child_value_candidates(
    values: &[DynSolValue],
    candidates: &mut Vec<DynSolValue>,
    limit: usize,
    rebuild: impl Fn(&[DynSolValue]) -> DynSolValue,
) {
    for idx in 0..values.len() {
        if candidates.len() >= limit {
            return;
        }
        for child in value_candidates(&values[idx], limit.saturating_sub(candidates.len())) {
            let mut values = values.to_vec();
            values[idx] = child;
            candidates.push(rebuild(&values));
            if candidates.len() >= limit {
                return;
            }
        }
    }
}

#[derive(Serialize)]
pub struct DisplayCorpusEntry {
    path: PathBuf,
    sequence: Vec<DisplayTxDetails>,
}

#[derive(Serialize)]
struct DisplayTxDetails {
    #[serde(flatten)]
    raw: BasicTxDetails,
    #[serde(skip_serializing_if = "Option::is_none")]
    decoded: Option<DecodedCall>,
}

#[derive(Serialize)]
struct DecodedCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    contract: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ambiguous_contracts: Vec<String>,
    signature: String,
    args: Vec<String>,
    call: String,
}

struct IndexedFunction {
    contract: String,
    function: Function,
}

#[derive(Default)]
struct CorpusDecoder {
    functions: BTreeMap<Selector, Vec<IndexedFunction>>,
}

impl CorpusDecoder {
    fn load() -> Self {
        Config::load().ok().map(|config| Self::from_artifacts(&config.out)).unwrap_or_default()
    }

    fn from_artifacts(out: &Path) -> Self {
        let mut this = Self::default();
        if !out.is_dir() {
            return this;
        }

        for path in fs::json_files(out) {
            let Ok(artifact) = fs::read_json_file::<serde_json::Value>(&path) else {
                continue;
            };
            let Some(abi_value) = artifact.get("abi").cloned() else {
                continue;
            };
            let Ok(abi) = serde_json::from_value::<JsonAbi>(abi_value) else {
                continue;
            };
            let contract =
                path.file_stem().and_then(|name| name.to_str()).unwrap_or("<unknown>").to_string();

            for function in abi.functions().cloned() {
                this.functions
                    .entry(function.selector())
                    .or_default()
                    .push(IndexedFunction { contract: contract.clone(), function });
            }
        }

        this
    }

    fn decode(&self, tx: &BasicTxDetails) -> Option<DecodedCall> {
        let calldata = tx.call_details.calldata.as_ref();
        if calldata.len() < 4 {
            return None;
        }

        let selector = Selector::from_slice(&calldata[..4]);
        let functions = self.functions.get(&selector)?;
        let (function, decoded_args) = self.unique_decodable_function(calldata)?;
        let args = format_tokens_raw(&decoded_args).collect::<Vec<_>>();
        let signature = function.signature();

        let mut contracts = functions
            .iter()
            .filter(|indexed| indexed.function.signature() == signature.as_str())
            .map(|indexed| indexed.contract.clone())
            .collect::<Vec<_>>();
        contracts.sort();
        contracts.dedup();

        let function_call = format!("{}({})", function.name, args.join(", "));
        if contracts.len() == 1 {
            let contract = contracts.pop()?;
            Some(DecodedCall {
                call: format!("{contract}.{function_call}"),
                contract: Some(contract),
                ambiguous_contracts: Vec::new(),
                signature,
                args,
            })
        } else {
            Some(DecodedCall {
                call: function_call,
                contract: None,
                ambiguous_contracts: contracts,
                signature,
                args,
            })
        }
    }

    fn unique_decodable_function(&self, calldata: &[u8]) -> Option<(&Function, Vec<DynSolValue>)> {
        if calldata.len() < 4 {
            return None;
        }

        let selector = Selector::from_slice(&calldata[..4]);
        let mut unique = None;
        for (function, decoded_args) in
            self.functions.get(&selector)?.iter().filter_map(|indexed| {
                let decoded_args = indexed.function.abi_decode_input(&calldata[4..]).ok()?;
                Some((&indexed.function, decoded_args))
            })
        {
            let signature = function.signature();
            match &unique {
                Some((existing, _, _)) if existing == &signature => {}
                Some(_) => return None,
                None => unique = Some((signature, function, decoded_args)),
            }
        }
        unique.map(|(_, function, decoded_args)| (function, decoded_args))
    }
}

fn read_entries(
    path: &Path,
    limit: Option<usize>,
    decoder: &CorpusDecoder,
) -> Result<Vec<DisplayCorpusEntry>> {
    let iter = read_corpus_entries(path)?.into_iter().take(limit.unwrap_or(usize::MAX));
    iter.map(|entry| {
        let sequence = entry
            .read_tx_seq()
            .with_context(|| format!("failed to read corpus entry {}", entry.path.display()))?;
        let sequence = sequence
            .into_iter()
            .map(|raw| {
                let decoded = decoder.decode(&raw);
                DisplayTxDetails { raw, decoded }
            })
            .collect();
        Ok(DisplayCorpusEntry { path: entry.path, sequence })
    })
    .collect()
}

fn read_corpus_entries(path: &Path) -> Result<Vec<CorpusDirEntry>> {
    let entries = read_corpus_tree(path)?;
    if entries.is_empty() {
        bail!("no corpus entries found under {}", path.display());
    }
    Ok(entries)
}

#[derive(Clone, Debug, Parser)]
struct FuzzMinimizeTestArgs {
    #[command(flatten)]
    global: GlobalArgs,
    #[command(flatten)]
    filter: FilterArgs,
    #[command(flatten)]
    evm: EvmArgs,
    #[command(flatten)]
    build: BuildOpts,
}

impl FuzzMinimizeTestArgs {
    async fn prepare_session(self, corpus_dir: &Path) -> Result<FuzzMinimizeReplaySession> {
        let mut test = TestArgs::parse_from(["test", "-q"]);
        test.set_fuzz_minimize_replay_options(self.global, self.evm, self.build, self.filter);
        test.enable_fuzz_only();
        prepare_minimize_session(&mut test, corpus_dir).await
    }

    fn decoder(&self) -> CorpusDecoder {
        self.build
            .load_config_no_warnings()
            .ok()
            .map(|config| CorpusDecoder::from_artifacts(&config.out))
            .unwrap_or_default()
    }
}

struct QuietShellGuard {
    previous: OutputMode,
}

impl QuietShellGuard {
    fn new() -> Self {
        let mut shell = Shell::get();
        let previous = shell.output_mode();
        shell.set_output_mode(OutputMode::Quiet);
        Self { previous }
    }
}

impl Drop for QuietShellGuard {
    fn drop(&mut self) {
        Shell::get().set_output_mode(self.previous);
    }
}

async fn prepare_minimize_session(
    test: &mut TestArgs,
    corpus_dir: &Path,
) -> Result<FuzzMinimizeReplaySession> {
    let _quiet = QuietShellGuard::new();
    test.prepare_fuzz_minimize_replay(corpus_dir).await
}

fn replay_candidate(
    session: &FuzzMinimizeReplaySession,
    evm_edge_indices: FuzzMinimizeEdgeIndices,
    sequence: Vec<BasicTxDetails>,
) -> Result<Vec<FuzzMinimizeObservation>> {
    let _quiet = QuietShellGuard::new();
    session.replay(sequence, evm_edge_indices)
}

fn merge_new_edges(cumulative: &mut ReplayObservation, observation: &ReplayObservation) -> bool {
    merge_new_edge_vec(&mut cumulative.evm_edges, &observation.evm_edges)
        | merge_new_edge_vec(&mut cumulative.sancov_edges, &observation.sancov_edges)
}

fn merge_new_edge_vec(cumulative: &mut Vec<u8>, candidate: &[u8]) -> bool {
    if cumulative.len() < candidate.len() {
        cumulative.resize(candidate.len(), 0);
    }
    let mut improved = false;
    for (cumulative, &candidate) in cumulative.iter_mut().zip(candidate) {
        if *cumulative < candidate {
            *cumulative = candidate;
            improved = true;
        }
    }
    improved
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use foundry_evm::executors::ReplayFailure;

    #[test]
    fn fuzz_args_clap_shape_is_valid() {
        FuzzArgs::command().debug_assert();
    }

    #[test]
    fn fuzz_run_rejects_fuzz_worker() {
        assert!(FuzzArgs::try_parse_from(["foundry-cli", "run", "--fuzz-worker", "1"]).is_err());
    }

    #[test]
    fn fuzz_run_rejects_fuzz_run() {
        assert!(FuzzArgs::try_parse_from(["foundry-cli", "run", "--fuzz-run", "1"]).is_err());
    }

    #[test]
    fn fuzz_seed_uses_bounded_defaults() {
        let args = FuzzArgs::parse_from(["foundry-cli", "seed"]);
        let FuzzSubcommands::Seed(args) = args.command else { panic!("expected seed command") };

        assert_eq!(args.run.campaign.runs, None);
        assert_eq!(args.run.campaign.frontier_limit, None);
        assert_eq!(args.solver_timeout, DEFAULT_SEED_SOLVER_TIMEOUT_SECS);
    }

    #[test]
    fn fuzz_seed_accepts_explicit_bounds() {
        let args = FuzzArgs::parse_from([
            "foundry-cli",
            "seed",
            "--runs",
            "64",
            "--frontier-limit",
            "8",
            "--solver-timeout",
            "2",
        ]);
        let FuzzSubcommands::Seed(args) = args.command else { panic!("expected seed command") };

        assert_eq!(args.run.campaign.runs, Some(64));
        assert_eq!(args.run.campaign.frontier_limit, Some(8));
        assert_eq!(args.solver_timeout, 2);
    }

    fn decoder_with_functions(functions: Vec<Function>) -> CorpusDecoder {
        let mut decoder = CorpusDecoder::default();
        for function in functions {
            decoder
                .functions
                .entry(function.selector())
                .or_default()
                .push(IndexedFunction { contract: "Target".to_string(), function });
        }
        decoder
    }

    fn candidate_args(function: &Function, candidates: Vec<Vec<u8>>) -> Vec<Vec<DynSolValue>> {
        candidates
            .into_iter()
            .map(|calldata| function.abi_decode_input(&calldata[4..]).unwrap())
            .collect()
    }

    #[test]
    fn merge_new_edges_keeps_sancov_hit_count_bucket_increases() {
        let mut cumulative = ReplayObservation { sancov_edges: vec![0, 1], ..Default::default() };
        let candidate = ReplayObservation { sancov_edges: vec![0, 8], ..Default::default() };

        assert!(merge_new_edges(&mut cumulative, &candidate));
        assert_eq!(cumulative.sancov_edges, vec![0, 8]);
    }

    #[test]
    fn same_edge_hit_sets_allow_hit_count_bucket_changes() {
        let baseline = ReplayObservation { evm_edges: vec![0, 8], ..Default::default() };
        let candidate = ReplayObservation { evm_edges: vec![0, 1], ..Default::default() };

        assert!(same_edge_hit_sets(&candidate, &baseline));
    }

    #[test]
    fn same_edge_hit_sets_treat_missing_trailing_buckets_as_zero() {
        let baseline = ReplayObservation { sancov_edges: vec![0, 0], ..Default::default() };
        let candidate = ReplayObservation { sancov_edges: vec![0], ..Default::default() };

        assert!(same_edge_hit_sets(&candidate, &baseline));
    }

    #[test]
    fn has_new_active_targets_rejects_candidate_only_activity() {
        let baseline = BTreeMap::from([(
            "A".to_string(),
            ReplayObservation { evm_edges: vec![1], ..Default::default() },
        )]);
        let observations = BTreeMap::from([
            ("A".to_string(), ReplayObservation { evm_edges: vec![1], ..Default::default() }),
            ("B".to_string(), ReplayObservation { evm_edges: vec![1], ..Default::default() }),
        ]);

        assert!(has_new_active_targets(&observations, &baseline));
    }

    #[test]
    fn has_new_active_targets_rejects_candidate_only_failures() {
        let baseline = BTreeMap::from([(
            "A".to_string(),
            ReplayObservation { evm_edges: vec![1], ..Default::default() },
        )]);
        let observations = BTreeMap::from([
            ("A".to_string(), ReplayObservation { evm_edges: vec![1], ..Default::default() }),
            (
                "B".to_string(),
                ReplayObservation {
                    failure: Some(ReplayFailure::AfterInvariant),
                    ..Default::default()
                },
            ),
        ]);

        assert!(has_new_active_targets(&observations, &baseline));
    }

    #[test]
    fn has_new_active_targets_rejects_candidate_only_replayed_transactions() {
        let baseline = BTreeMap::from([(
            "A".to_string(),
            ReplayObservation { evm_edges: vec![1], ..Default::default() },
        )]);
        let observations = BTreeMap::from([
            ("A".to_string(), ReplayObservation { evm_edges: vec![1], ..Default::default() }),
            ("B".to_string(), ReplayObservation { replayed: 1, ..Default::default() }),
        ]);

        assert!(has_new_active_targets(&observations, &baseline));
    }

    #[test]
    fn has_new_active_targets_allows_inactive_candidate_only_targets() {
        let baseline = BTreeMap::from([(
            "A".to_string(),
            ReplayObservation { evm_edges: vec![1], ..Default::default() },
        )]);
        let observations = BTreeMap::from([
            ("A".to_string(), ReplayObservation { evm_edges: vec![1], ..Default::default() }),
            ("B".to_string(), ReplayObservation::default()),
        ]);

        assert!(!has_new_active_targets(&observations, &baseline));
    }

    #[test]
    fn abi_calldata_candidates_simplify_scalar_values() {
        let function = Function::parse("target(uint256,int256,bool,address)").unwrap();
        let decoder = decoder_with_functions(vec![function.clone()]);
        let calldata = function
            .abi_encode_input(&[
                DynSolValue::Uint(U256::from(42), 256),
                DynSolValue::Int(I256::from_raw(U256::from(42)), 256),
                DynSolValue::Bool(true),
                DynSolValue::Address(Address::from([0x11; 20])),
            ])
            .unwrap();

        let candidates =
            candidate_args(&function, abi_calldata_candidates(&calldata, &decoder, usize::MAX));

        assert!(candidates.iter().any(|args| args[0] == DynSolValue::Uint(U256::ZERO, 256)));
        assert!(candidates.iter().any(|args| args[0] == DynSolValue::Uint(U256::from(1), 256)));
        assert!(candidates.iter().any(|args| args[1] == DynSolValue::Int(I256::ZERO, 256)));
        assert!(
            candidates
                .iter()
                .any(|args| { args[1] == DynSolValue::Int(I256::from_raw(U256::from(1)), 256) })
        );
        assert!(candidates.iter().any(|args| args[1] == DynSolValue::Int(I256::MINUS_ONE, 256)));
        assert!(candidates.iter().any(|args| args[2] == DynSolValue::Bool(false)));
        assert!(candidates.iter().any(|args| args[3] == DynSolValue::Address(Address::ZERO)));
    }

    #[test]
    fn abi_calldata_candidates_do_not_oscillate_signed_one_values() {
        let function = Function::parse("target(int256)").unwrap();
        let decoder = decoder_with_functions(vec![function.clone()]);
        let one = DynSolValue::Int(I256::from_raw(U256::from(1)), 256);
        let minus_one = DynSolValue::Int(I256::MINUS_ONE, 256);

        let one_calldata = function.abi_encode_input(&[one]).unwrap();
        let one_candidates =
            candidate_args(&function, abi_calldata_candidates(&one_calldata, &decoder, usize::MAX));
        assert_eq!(one_candidates, vec![vec![DynSolValue::Int(I256::ZERO, 256)]]);

        let minus_one_calldata = function.abi_encode_input(&[minus_one]).unwrap();
        let minus_one_candidates = candidate_args(
            &function,
            abi_calldata_candidates(&minus_one_calldata, &decoder, usize::MAX),
        );
        assert_eq!(minus_one_candidates, vec![vec![DynSolValue::Int(I256::ZERO, 256)]]);
    }

    #[test]
    fn abi_calldata_candidates_shrink_dynamic_values() {
        let function = Function::parse("target(bytes,string,uint256[])").unwrap();
        let decoder = decoder_with_functions(vec![function.clone()]);
        let calldata = function
            .abi_encode_input(&[
                DynSolValue::Bytes(vec![1, 2, 3, 4]),
                DynSolValue::String("abcdef".to_string()),
                DynSolValue::Array(vec![
                    DynSolValue::Uint(U256::from(10), 256),
                    DynSolValue::Uint(U256::from(11), 256),
                    DynSolValue::Uint(U256::from(12), 256),
                    DynSolValue::Uint(U256::from(13), 256),
                ]),
            ])
            .unwrap();

        let candidates =
            candidate_args(&function, abi_calldata_candidates(&calldata, &decoder, usize::MAX));

        assert!(candidates.iter().any(|args| args[0] == DynSolValue::Bytes(Vec::new())));
        assert!(candidates.iter().any(|args| args[0] == DynSolValue::Bytes(vec![1, 2])));
        assert!(candidates.iter().any(|args| args[1] == DynSolValue::String(String::new())));
        assert!(candidates.iter().any(|args| args[1] == DynSolValue::String("abc".to_string())));
        assert!(candidates.iter().any(|args| args[2] == DynSolValue::Array(Vec::new())));
        assert!(candidates.iter().any(|args| {
            args[2]
                == DynSolValue::Array(vec![
                    DynSolValue::Uint(U256::from(10), 256),
                    DynSolValue::Uint(U256::from(11), 256),
                ])
        }));
    }

    #[test]
    fn abi_calldata_candidates_simplify_tuple_children() {
        let function = Function::parse("target((uint256,bool,address))").unwrap();
        let decoder = decoder_with_functions(vec![function.clone()]);
        let calldata = function
            .abi_encode_input(&[DynSolValue::Tuple(vec![
                DynSolValue::Uint(U256::from(42), 256),
                DynSolValue::Bool(true),
                DynSolValue::Address(Address::from([0x11; 20])),
            ])])
            .unwrap();

        let candidates =
            candidate_args(&function, abi_calldata_candidates(&calldata, &decoder, usize::MAX));

        assert!(candidates.iter().any(|args| {
            args[0]
                == DynSolValue::Tuple(vec![
                    DynSolValue::Uint(U256::ZERO, 256),
                    DynSolValue::Bool(true),
                    DynSolValue::Address(Address::from([0x11; 20])),
                ])
        }));
        assert!(candidates.iter().any(|args| {
            args[0]
                == DynSolValue::Tuple(vec![
                    DynSolValue::Uint(U256::from(42), 256),
                    DynSolValue::Bool(false),
                    DynSolValue::Address(Address::from([0x11; 20])),
                ])
        }));
        assert!(candidates.iter().any(|args| {
            args[0]
                == DynSolValue::Tuple(vec![
                    DynSolValue::Uint(U256::from(42), 256),
                    DynSolValue::Bool(true),
                    DynSolValue::Address(Address::ZERO),
                ])
        }));
    }

    #[test]
    fn abi_calldata_candidates_skip_ambiguous_or_undecodable_calldata() {
        let function = Function::parse("target(uint256)").unwrap();
        let other = Function::parse("other(uint256)").unwrap();
        let calldata =
            function.abi_encode_input(&[DynSolValue::Uint(U256::from(42), 256)]).unwrap();

        let mut ambiguous = CorpusDecoder::default();
        ambiguous.functions.entry(function.selector()).or_default().extend([
            IndexedFunction { contract: "Target".to_string(), function: function.clone() },
            IndexedFunction { contract: "Other".to_string(), function: other },
        ]);
        assert!(abi_calldata_candidates(&calldata, &ambiguous, usize::MAX).is_empty());

        let decoder = decoder_with_functions(vec![function]);
        assert!(
            abi_calldata_candidates(&calldata[..calldata.len() - 1], &decoder, usize::MAX)
                .is_empty()
        );
    }

    #[test]
    fn abi_calldata_candidates_accept_same_signature_with_metadata_differences() {
        let function =
            Function::parse("function target(uint256 value) external returns (uint256)").unwrap();
        let same_signature = Function::parse("function target(uint256 value) view").unwrap();
        let calldata =
            function.abi_encode_input(&[DynSolValue::Uint(U256::from(42), 256)]).unwrap();

        let mut decoder = CorpusDecoder::default();
        decoder.functions.entry(function.selector()).or_default().extend([
            IndexedFunction { contract: "WithReturn".to_string(), function: function.clone() },
            IndexedFunction { contract: "NoReturn".to_string(), function: same_signature },
        ]);

        let candidates =
            candidate_args(&function, abi_calldata_candidates(&calldata, &decoder, usize::MAX));

        assert!(candidates.iter().any(|args| args[0] == DynSolValue::Uint(U256::ZERO, 256)));
    }
}
