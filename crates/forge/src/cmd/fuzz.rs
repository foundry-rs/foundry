use crate::{
    cmd::test::{FilterArgs, FuzzMinimizeReplaySession, TestArgs},
    multi_runner::ShowmapConfig,
    result::TestOutcome,
};
use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{Address, B256, Function as SolFunction, I256, Selector, U256};
use clap::{Parser, Subcommand, ValueEnum, ValueHint};
use eyre::{Context, Result, bail};
use foundry_cli::opts::{BuildOpts, EvmArgs, GlobalArgs};
use foundry_common::{
    fmt::format_tokens_raw,
    fs, sh_println,
    shell::{OutputMode, Shell},
};
use foundry_config::Config;
use foundry_evm::{
    executors::{CorpusDirEntry, ReplayObservation, ShowmapDomain, read_corpus_tree},
    fuzz::BasicTxDetails,
    inspectors::EdgeIndexMap,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::{Builder as TempDirBuilder, TempDir};

/// Run and manage Forge fuzzing corpora.
#[derive(Clone, Debug, Parser)]
pub struct FuzzArgs {
    #[command(subcommand)]
    pub command: FuzzSubcommands,
}

impl FuzzArgs {
    pub async fn run(self) -> Result<TestOutcome> {
        match self.command {
            FuzzSubcommands::Run(mut args) => {
                args.enable_fuzz_only();
                args.run().await
            }
            FuzzSubcommands::Replay(args) => args.run().await,
            FuzzSubcommands::Show(args) => {
                args.run()?;
                Ok(TestOutcome::empty(None, true))
            }
            FuzzSubcommands::Cmin(args) => {
                args.run().await?;
                Ok(TestOutcome::empty(None, true))
            }
            FuzzSubcommands::Tmin(args) => {
                args.run().await?;
                Ok(TestOutcome::empty(None, true))
            }
        }
    }

    pub const fn is_junit(&self) -> bool {
        match &self.command {
            FuzzSubcommands::Run(args) => args.junit,
            FuzzSubcommands::Replay(args) => args.is_junit(),
            FuzzSubcommands::Show(_) | FuzzSubcommands::Cmin(_) | FuzzSubcommands::Tmin(_) => false,
        }
    }

    pub fn reject_unsupported_flags(&self) -> Result<()> {
        match &self.command {
            FuzzSubcommands::Run(args) if args.is_watch() => {
                bail!("`--watch` is not supported for `forge fuzz run`")
            }
            FuzzSubcommands::Replay(args) if args.is_watch() => {
                bail!("`--watch` is not supported for `forge fuzz replay`")
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Subcommand)]
pub enum FuzzSubcommands {
    /// Run only fuzz and invariant tests.
    Run(TestArgs),
    /// Replay persisted fuzz failures, or corpus entries with `--corpus-dir`.
    Replay(FuzzReplayArgs),
    /// Print persisted corpus entries.
    Show(FuzzShowArgs),
    /// Minimize a corpus by keeping entries that contribute new coverage.
    Cmin(FuzzCminArgs),
    /// Minimize one corpus entry while preserving its failure or coverage.
    Tmin(FuzzTminArgs),
}

/// Replay persisted fuzz failures, or corpus entries with `--corpus-dir`.
#[derive(Clone, Debug, Parser)]
pub struct FuzzReplayArgs {
    #[command(flatten)]
    test: TestArgs,
    /// Replay corpus entries from this directory instead of persisted fuzz failures.
    #[arg(long, value_name = "PATH", value_hint = ValueHint::DirPath)]
    corpus_dir: Option<PathBuf>,
}

impl FuzzReplayArgs {
    async fn run(mut self) -> Result<TestOutcome> {
        self.test.enable_fuzz_only();
        if self.corpus_dir.is_none() {
            self.test.enable_fuzz_failure_replay();
            return self.test.run().await;
        }

        let replay_id =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
        self.test.set_showmap_override(ShowmapConfig {
            out_dir: std::env::temp_dir().join(format!("forge-fuzz-replay-{replay_id}")),
            approach: "replay".to_string(),
            trial: "replay".to_string(),
            per_input: false,
            domain: ShowmapDomain::Evm,
            corpus_dir: self.corpus_dir.clone(),
            emit_files: false,
        });
        self.test.run().await
    }

    const fn is_junit(&self) -> bool {
        self.test.junit
    }

    const fn is_watch(&self) -> bool {
        self.test.is_watch()
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
                            sh_println!(
                                "  {idx}: {} sender={} target={} value={}",
                                decoded.call,
                                tx.raw.sender,
                                tx.raw.call_details.target,
                                tx.raw
                                    .call_details
                                    .value
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "0".to_string())
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
        if self.out.exists() {
            bail!("output corpus directory already exists: {}", self.out.display());
        }

        let staging_out = temporary_cmin_out(&self.out)?;
        let staging_path = staging_out.path().to_path_buf();
        let summary = self.run_to(&staging_path).await?;

        // Best-effort re-check to narrow the TOCTOU window from the check above.
        // `rename` is not portably no-clobber (it replaces an empty target dir on
        // Unix), so a concurrent writer racing the same `--corpus-out` is still the
        // user's responsibility.
        if self.out.exists() {
            bail!("output corpus directory already exists: {}", self.out.display());
        }

        if let Err(err) = std::fs::rename(&staging_path, &self.out) {
            return Err(err).with_context(|| {
                format!(
                    "failed to rename minimized corpus {} to {}",
                    staging_path.display(),
                    self.out.display()
                )
            });
        }

        sh_println!(
            "minimized corpus: kept {}/{} entries in {}",
            summary.kept,
            summary.total,
            self.out.display()
        )?;
        if summary.skipped > 0 {
            sh_println!(
                "skipped {} entries or txs that could not be read or replayed",
                summary.skipped
            )?;
        }
        Ok(())
    }

    async fn run_to(&self, out_dir: &Path) -> Result<CminSummary> {
        let session = self.test.clone().prepare_session().await?;
        let mut kept = 0usize;
        let mut total = 0usize;
        let mut skipped = 0usize;
        let mut unreadable = 0usize;
        let mut replayed = 0usize;
        let mut cumulative = ReplayObservation::default();
        let evm_edge_indices = Arc::new(Mutex::new(EdgeIndexMap::default()));
        for entry in read_corpus_entries(&self.corpus_dir)? {
            total += 1;
            let sequence = read_sequence(&entry.path)
                .with_context(|| format!("failed to read corpus entry {}", entry.path.display()));
            let Ok(sequence) = sequence else {
                skipped += 1;
                unreadable += 1;
                continue;
            };
            let observation = replay_candidate(&session, evm_edge_indices.clone(), sequence)?;
            replayed += observation.replayed;
            skipped += observation.skipped;
            let keep = merge_new_edges(&mut cumulative, &observation);
            if !keep {
                continue;
            }
            let out = if self.corpus_dir.is_file() {
                out_dir.join(entry.path.file_name().unwrap_or_default())
            } else {
                out_dir.join(entry.path.strip_prefix(&self.corpus_dir).unwrap_or(&entry.path))
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
            // `skipped` also counts unreadable entries; the remainder are txs that
            // existed but did not target the matched test (filter mismatch / `vm.assume`).
            let mismatched = skipped.saturating_sub(unreadable);
            if unreadable == total {
                bail!(
                    "replayed 0 transactions from {corpus}; all {unreadable} corpus entries could not be read"
                );
            }
            if mismatched > 0 {
                bail!(
                    "replayed 0 transactions from {corpus}; {mismatched} transactions did not match \
                     the test — check that --mc/--mt match the corpus entries"
                );
            }
            bail!(
                "replayed 0 transactions from {corpus}; corpus entries were empty\
                 {}",
                if unreadable > 0 {
                    format!(" or unreadable ({unreadable} unreadable)")
                } else {
                    String::new()
                }
            );
        }

        Ok(CminSummary { kept, total, skipped })
    }
}

struct CminSummary {
    kept: usize,
    total: usize,
    skipped: usize,
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

/// Minimize one corpus entry while preserving its failure or coverage.
#[derive(Clone, Debug, Parser)]
pub struct FuzzTminArgs {
    #[command(flatten)]
    test: FuzzMinimizeTestArgs,
    /// Input corpus file.
    #[arg(value_name = "INPUT", value_hint = ValueHint::FilePath)]
    input: PathBuf,
    /// Output corpus file.
    #[arg(long = "corpus-out", value_name = "FILE", value_hint = ValueHint::FilePath)]
    out: PathBuf,
    /// Maximum candidate replays to attempt.
    #[arg(long, default_value_t = 5000, value_name = "N")]
    max_attempts: usize,
}

impl FuzzTminArgs {
    async fn run(self) -> Result<()> {
        let mut sequence = read_sequence(&self.input)?;
        let before_txs = sequence.len();
        let session = self.test.prepare_session().await?;
        let evm_edge_indices = Arc::new(Mutex::new(EdgeIndexMap::default()));
        let original = replay_candidate(&session, evm_edge_indices.clone(), sequence.clone())?;
        if original.replayed == 0 && !original.has_coverage() && original.failure.is_none() {
            bail!(
                "replayed 0 transactions from {}; check that --mc/--mt match the corpus entry",
                self.input.display()
            );
        }
        let decoder = CorpusDecoder::load();
        let mut ctx =
            MinimizeContext::new(&session, evm_edge_indices, &original, self.max_attempts);
        minimize_sequence(&mut ctx, &mut sequence, &decoder)?;

        let write_result = if is_gzip_path(&self.out) {
            fs::write_json_gzip_file_create_new(&self.out, &sequence)
        } else {
            fs::write_json_file_create_new(&self.out, &sequence)
        };
        if let Err(err) = write_result {
            if matches!(
                err,
                foundry_common::errors::FsPathError::CreateFile { ref source, .. }
                    if source.kind() == ErrorKind::AlreadyExists
            ) {
                bail!("output corpus file already exists: {}", self.out.display());
            }
            return Err(err.into());
        }
        sh_println!("minimized entry: {} txs -> {}", sequence.len(), self.out.display())?;
        sh_println!(
            "removed {} txs after {} candidate replays",
            before_txs - sequence.len(),
            ctx.attempts
        )?;
        if original.failure.is_some() {
            sh_println!("preserved original failure")?;
        }
        Ok(())
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
    contract: String,
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
        let function = self.unique_decodable_function(calldata)?;
        let decoded_args = function.abi_decode_input(&calldata[4..]).ok()?;
        let args = format_tokens_raw(&decoded_args).collect::<Vec<_>>();

        let selector = Selector::from_slice(&calldata[..4]);
        self.functions.get(&selector).and_then(|functions| {
            let indexed = functions.iter().find(|indexed| &indexed.function == function)?;
            let signature = indexed.function.signature();
            Some(DecodedCall {
                call: format!(
                    "{}.{}({})",
                    indexed.contract,
                    indexed.function.name,
                    args.join(", ")
                ),
                contract: indexed.contract.clone(),
                signature,
                args,
            })
        })
    }

    fn unique_decodable_function(&self, calldata: &[u8]) -> Option<&Function> {
        if calldata.len() < 4 {
            return None;
        }

        let selector = Selector::from_slice(&calldata[..4]);
        let mut unique = None;
        for function in self.functions.get(&selector)?.iter().filter_map(|indexed| {
            indexed.function.abi_decode_input(&calldata[4..]).ok()?;
            Some(&indexed.function)
        }) {
            match unique {
                Some(existing) if existing == function => {}
                Some(_) => return None,
                None => unique = Some(function),
            }
        }
        unique
    }
}

fn read_entries(
    path: &Path,
    limit: Option<usize>,
    decoder: &CorpusDecoder,
) -> Result<Vec<DisplayCorpusEntry>> {
    let iter = read_corpus_entries(path)?.into_iter().take(limit.unwrap_or(usize::MAX));
    iter.map(|entry| {
        let sequence = read_sequence(&entry.path)
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

fn read_sequence(path: &Path) -> Result<Vec<BasicTxDetails>> {
    if is_gzip_path(path) {
        Ok(fs::read_json_gzip_file(path)?)
    } else {
        Ok(fs::read_json_file(path)?)
    }
}

fn is_gzip_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("gz"))
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
    async fn prepare_session(self) -> Result<FuzzMinimizeReplaySession> {
        let mut test = TestArgs::parse_from(["test", "-q"]);
        test.set_fuzz_minimize_replay_options(self.global, self.evm, self.build, self.filter);
        test.enable_fuzz_only();
        prepare_minimize_session(&mut test).await
    }
}

/// Forces the global shell into [`OutputMode::Quiet`] while any guard is alive.
///
/// Guards are reference-counted so nested/overlapping scopes (e.g. a replay guard
/// taken while a `prepare` guard is held across an `.await`) restore the original
/// output mode only once the outermost guard is dropped, instead of an inner guard
/// prematurely restoring it.
struct QuietShellGuard;

static QUIET_STATE: Mutex<(usize, Option<OutputMode>)> = Mutex::new((0, None));

impl QuietShellGuard {
    fn new() -> Self {
        let mut state = QUIET_STATE.lock().unwrap_or_else(|err| err.into_inner());
        if state.0 == 0 {
            let mut shell = Shell::get();
            state.1 = Some(shell.output_mode());
            shell.set_output_mode(OutputMode::Quiet);
        }
        state.0 += 1;
        Self
    }
}

impl Drop for QuietShellGuard {
    fn drop(&mut self) {
        let mut state = QUIET_STATE.lock().unwrap_or_else(|err| err.into_inner());
        state.0 -= 1;
        if state.0 == 0
            && let Some(previous) = state.1.take()
        {
            Shell::get().set_output_mode(previous);
        }
    }
}

async fn prepare_minimize_session(test: &mut TestArgs) -> Result<FuzzMinimizeReplaySession> {
    let _quiet = QuietShellGuard::new();
    test.prepare_fuzz_minimize_replay().await
}

fn replay_candidate(
    session: &FuzzMinimizeReplaySession,
    evm_edge_indices: Arc<Mutex<EdgeIndexMap>>,
    sequence: Vec<BasicTxDetails>,
) -> Result<ReplayObservation> {
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
        if candidate > *cumulative {
            *cumulative = candidate;
            improved = true;
        }
    }
    improved
}

/// State shared across a `tmin` minimization pass: the replay session, the
/// baseline observation candidates must match, and an attempt budget.
struct MinimizeContext<'a> {
    session: &'a FuzzMinimizeReplaySession,
    evm_edge_indices: Arc<Mutex<EdgeIndexMap>>,
    original: &'a ReplayObservation,
    max_attempts: usize,
    attempts: usize,
}

impl<'a> MinimizeContext<'a> {
    const fn new(
        session: &'a FuzzMinimizeReplaySession,
        evm_edge_indices: Arc<Mutex<EdgeIndexMap>>,
        original: &'a ReplayObservation,
        max_attempts: usize,
    ) -> Self {
        Self { session, evm_edge_indices, original, max_attempts, attempts: 0 }
    }

    const fn at_budget(&self) -> bool {
        self.attempts >= self.max_attempts
    }

    /// Replays `candidate` and reports whether it still reproduces `original`
    /// (the same failure, or full coverage of the original's edges).
    fn accepts(&mut self, candidate: &[BasicTxDetails]) -> Result<bool> {
        if self.at_budget() {
            return Ok(false);
        }
        self.attempts += 1;
        let observed =
            replay_candidate(self.session, self.evm_edge_indices.clone(), candidate.to_vec())?;
        if let Some(failure) = &self.original.failure {
            Ok(observed.failure.as_ref() == Some(failure))
        } else {
            Ok(observed.failure.is_none() && covers_edges(&observed, self.original))
        }
    }
}

fn minimize_sequence(
    ctx: &mut MinimizeContext<'_>,
    sequence: &mut Vec<BasicTxDetails>,
    decoder: &CorpusDecoder,
) -> Result<()> {
    // Drop individual txs, mutating in place and rolling back rejected removals so
    // each attempt only clones the sequence once (inside `MinimizeContext::accepts`).
    let mut idx = 0;
    while idx < sequence.len() && !ctx.at_budget() {
        let removed = sequence.remove(idx);
        if ctx.accepts(sequence)? {
            // Keep the removal; the next tx now occupies `idx`.
        } else {
            sequence.insert(idx, removed);
            idx += 1;
        }
    }

    // Strip redundant per-tx metadata (default warp/roll/value).
    let mut idx = 0;
    while idx < sequence.len() && !ctx.at_budget() {
        let restore = sequence[idx].clone();
        cleanup_metadata(&mut sequence[idx]);
        if !ctx.accepts(sequence)? {
            sequence[idx] = restore;
        }
        idx += 1;
    }

    // Simplify ABI calldata values.
    let mut tx_idx = 0;
    while tx_idx < sequence.len() && !ctx.at_budget() {
        for calldata in
            abi_calldata_candidates(sequence[tx_idx].call_details.calldata.as_ref(), decoder)
        {
            if ctx.at_budget() {
                break;
            }
            let restore =
                std::mem::replace(&mut sequence[tx_idx].call_details.calldata, calldata.into());
            if !ctx.accepts(sequence)? {
                sequence[tx_idx].call_details.calldata = restore;
            }
        }
        tx_idx += 1;
    }
    Ok(())
}

fn covers_edges(candidate: &ReplayObservation, original: &ReplayObservation) -> bool {
    covers_edge_vec(&candidate.evm_edges, &original.evm_edges)
        && covers_edge_vec(&candidate.sancov_edges, &original.sancov_edges)
}

fn covers_edge_vec(candidate: &[u8], original: &[u8]) -> bool {
    original
        .iter()
        .enumerate()
        .all(|(idx, &edge)| candidate.get(idx).copied().unwrap_or_default() >= edge)
}

fn cleanup_metadata(tx: &mut BasicTxDetails) {
    if tx.warp == Some(Default::default()) {
        tx.warp = None;
    }
    if tx.roll == Some(Default::default()) {
        tx.roll = None;
    }
    if tx.call_details.value == Some(Default::default()) {
        tx.call_details.value = None;
    }
}

fn abi_calldata_candidates(calldata: &[u8], decoder: &CorpusDecoder) -> Vec<Vec<u8>> {
    let Some(function) = decoder.unique_decodable_function(calldata) else {
        return Vec::new();
    };
    let Ok(args) = function.abi_decode_input(&calldata[4..]) else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for arg_idx in 0..args.len() {
        for value in value_candidates(&args[arg_idx]) {
            let mut candidate_args = args.clone();
            candidate_args[arg_idx] = value;
            let Ok(encoded) = function.abi_encode_input(&candidate_args) else {
                continue;
            };
            if encoded.as_slice() != calldata && seen.insert(encoded.clone()) {
                candidates.push(encoded);
            }
        }
    }
    candidates
}

fn value_candidates(value: &DynSolValue) -> Vec<DynSolValue> {
    let mut candidates = Vec::new();
    push_scalar_value_candidates(value, &mut candidates);
    push_compound_value_candidates(value, &mut candidates);
    candidates.into_iter().filter(|candidate| candidate != value).collect()
}

fn push_scalar_value_candidates(value: &DynSolValue, candidates: &mut Vec<DynSolValue>) {
    match value {
        DynSolValue::Bool(_) => candidates.push(DynSolValue::Bool(false)),
        DynSolValue::Uint(_, bits) => {
            candidates.push(DynSolValue::Uint(U256::ZERO, *bits));
            candidates.push(DynSolValue::Uint(U256::from(1), *bits));
        }
        DynSolValue::Int(_, bits) => {
            candidates.push(DynSolValue::Int(I256::ZERO, *bits));
            candidates.push(DynSolValue::Int(I256::from_raw(U256::from(1)), *bits));
            candidates.push(DynSolValue::Int(I256::MINUS_ONE, *bits));
        }
        DynSolValue::Address(_) => candidates.push(DynSolValue::Address(Address::ZERO)),
        DynSolValue::FixedBytes(_, size) => {
            candidates.push(DynSolValue::FixedBytes(B256::ZERO, *size));
        }
        DynSolValue::Function(_) => candidates.push(DynSolValue::Function(SolFunction::ZERO)),
        DynSolValue::Bytes(bytes) => {
            candidates.push(DynSolValue::Bytes(Vec::new()));
            if bytes.len() > 1 {
                candidates.push(DynSolValue::Bytes(bytes[..bytes.len() / 2].to_vec()));
            }
        }
        DynSolValue::String(string) => {
            candidates.push(DynSolValue::String(String::new()));
            if string.len() > 1 {
                let mut half = string.len() / 2;
                while half > 0 && !string.is_char_boundary(half) {
                    half -= 1;
                }
                candidates.push(DynSolValue::String(string[..half].to_string()));
            }
        }
        DynSolValue::Array(_)
        | DynSolValue::FixedArray(_)
        | DynSolValue::Tuple(_)
        | DynSolValue::CustomStruct { .. } => {}
    }
}

fn push_compound_value_candidates(value: &DynSolValue, candidates: &mut Vec<DynSolValue>) {
    match value {
        DynSolValue::Array(values) => {
            candidates.push(DynSolValue::Array(Vec::new()));
            if values.len() > 1 {
                candidates.push(DynSolValue::Array(values[..values.len() / 2].to_vec()));
            }
            push_child_value_candidates(values, candidates, |values| {
                DynSolValue::Array(values.to_vec())
            });
        }
        DynSolValue::FixedArray(values) => {
            push_child_value_candidates(values, candidates, |values| {
                DynSolValue::FixedArray(values.to_vec())
            });
        }
        DynSolValue::Tuple(values) => {
            push_child_value_candidates(values, candidates, |values| {
                DynSolValue::Tuple(values.to_vec())
            });
        }
        DynSolValue::CustomStruct { name, prop_names, tuple } => {
            push_child_value_candidates(tuple, candidates, |values| DynSolValue::CustomStruct {
                name: name.clone(),
                prop_names: prop_names.clone(),
                tuple: values.to_vec(),
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
    rebuild: impl Fn(&[DynSolValue]) -> DynSolValue,
) {
    for idx in 0..values.len() {
        for child in value_candidates(&values[idx]) {
            let mut values = values.to_vec();
            values[idx] = child;
            candidates.push(rebuild(&values));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoder_with_functions(functions: Vec<Function>) -> CorpusDecoder {
        let mut abi = JsonAbi::new();
        for function in functions {
            abi.functions.entry(function.name.clone()).or_default().push(function);
        }

        let mut decoder = CorpusDecoder::default();
        for function in abi.functions().cloned() {
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
    fn minimizers_accept_replay_test_args() {
        FuzzArgs::try_parse_from([
            "forge",
            "cmin",
            "--fork-url",
            "http://localhost:8545",
            "--via-ir",
            "--mc",
            "Target",
            "corpus",
            "--out",
            "artifacts",
            "--corpus-out",
            "min-corpus",
        ])
        .unwrap();

        FuzzArgs::try_parse_from([
            "forge",
            "tmin",
            "--fork-url",
            "http://localhost:8545",
            "--via-ir",
            "--mt",
            "testFuzz",
            "corpus/input.json",
            "--corpus-out",
            "min.json",
        ])
        .unwrap();
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

        let candidates = candidate_args(&function, abi_calldata_candidates(&calldata, &decoder));

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

        let candidates = candidate_args(&function, abi_calldata_candidates(&calldata, &decoder));

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
        let duplicate = decoder_with_functions(vec![function.clone(), function.clone()]);
        let calldata =
            function.abi_encode_input(&[DynSolValue::Uint(U256::from(42), 256)]).unwrap();

        assert!(!abi_calldata_candidates(&calldata, &duplicate).is_empty());

        let other = Function::parse("other(uint256)").unwrap();
        let mut ambiguous = CorpusDecoder::default();
        ambiguous.functions.entry(function.selector()).or_default().extend([
            IndexedFunction { contract: "Target".to_string(), function: function.clone() },
            IndexedFunction { contract: "Other".to_string(), function: other },
        ]);
        assert!(abi_calldata_candidates(&calldata, &ambiguous).is_empty());

        let decoder = decoder_with_functions(vec![function]);
        assert!(abi_calldata_candidates(&calldata[..calldata.len() - 1], &decoder).is_empty());
    }
}
