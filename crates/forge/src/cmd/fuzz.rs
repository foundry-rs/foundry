use crate::{
    cmd::test::{FilterArgs, TestArgs},
    multi_runner::{FuzzMinimizeConfig, ShowmapConfig},
};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::Selector;
use clap::{Parser, Subcommand, ValueEnum, ValueHint};
use eyre::{Context, Result, bail};
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
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

/// Run and manage Forge fuzzing corpora.
#[derive(Clone, Debug, Parser)]
pub struct FuzzArgs {
    #[command(subcommand)]
    pub command: FuzzSubcommands,
}

impl FuzzArgs {
    pub async fn run(self) -> Result<crate::result::TestOutcome> {
        match self.command {
            FuzzSubcommands::Run(mut args) => {
                args.enable_fuzz_only();
                args.reject_machine_unsupported_flags()?;
                args.run().await
            }
            FuzzSubcommands::Replay(args) => args.run().await,
            FuzzSubcommands::Show(args) => {
                args.run()?;
                Ok(crate::result::TestOutcome::empty(None, true))
            }
            FuzzSubcommands::Cmin(args) => {
                args.run().await?;
                Ok(crate::result::TestOutcome::empty(None, true))
            }
            FuzzSubcommands::Tmin(args) => {
                args.run().await?;
                Ok(crate::result::TestOutcome::empty(None, true))
            }
        }
    }

    pub async fn run_machine(self) -> Result<Option<crate::result::TestOutcome>> {
        match self.command {
            FuzzSubcommands::Run(mut args) => {
                args.enable_fuzz_only();
                args.reject_machine_unsupported_flags()?;
                Ok(Some(args.run().await?))
            }
            FuzzSubcommands::Replay(args) => Ok(Some(args.run().await?)),
            // TODO: Reintroduce `--machine` for these corpus subcommands once their
            // output schema is stable and does not interleave minimizer replay events.
            FuzzSubcommands::Show(_) => reject_machine("show"),
            FuzzSubcommands::Cmin(_) => reject_machine("cmin"),
            FuzzSubcommands::Tmin(_) => reject_machine("tmin"),
        }
    }
}

fn reject_machine(subcommand: &str) -> ! {
    foundry_cli::machine::bail_machine_usage_with_details(
        format!(
            "`forge fuzz {subcommand}` temporarily does not support `--machine`; run without \
                 `--machine`."
        ),
        serde_json::json!({ "subcommand": format!("fuzz {subcommand}") }),
    )
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
    async fn run(mut self) -> Result<crate::result::TestOutcome> {
        self.test.enable_fuzz_only();
        self.test.reject_machine_unsupported_flags()?;
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
    filter: FilterArgs,
    /// Input corpus directory.
    #[arg(value_name = "CORPUS_DIR", value_hint = ValueHint::DirPath)]
    corpus_dir: PathBuf,
    /// Output corpus directory.
    #[arg(long, short, value_name = "DIR", value_hint = ValueHint::DirPath)]
    out: PathBuf,
}

impl FuzzCminArgs {
    async fn run(self) -> Result<()> {
        if self.out.exists() {
            bail!("output corpus directory already exists: {}", self.out.display());
        }
        fs::create_dir_all(&self.out)?;

        let mut test = minimizer_test_args(self.filter);
        test.enable_fuzz_only();
        test.reject_machine_unsupported_flags()?;
        let evm_edge_indices = Arc::new(Mutex::new(EdgeIndexMap::default()));
        let mut kept = 0usize;
        let mut total = 0usize;
        let mut skipped = 0usize;
        let mut replayed = 0usize;
        let mut cumulative = ReplayObservation::default();
        for entry in read_corpus_entries(&self.corpus_dir)? {
            total += 1;
            let sequence = read_sequence(&entry.path)
                .with_context(|| format!("failed to read corpus entry {}", entry.path.display()));
            let Ok(sequence) = sequence else {
                skipped += 1;
                continue;
            };
            let observation =
                replay_candidate(&mut test, evm_edge_indices.clone(), sequence).await?;
            replayed += observation.replayed;
            skipped += observation.skipped;
            let keep = merge_new_edges(&mut cumulative, &observation);
            if !keep {
                continue;
            }
            let rel = entry.path.strip_prefix(&self.corpus_dir).unwrap_or(&entry.path);
            let out = self.out.join(rel);
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            std::fs::copy(&entry.path, &out).with_context(|| {
                format!("failed to copy {} to {}", entry.path.display(), out.display())
            })?;
            kept += 1;
        }

        if total > 0 && replayed == 0 {
            bail!(
                "replayed 0 transactions from {}; check that --mc/--mt match the corpus entries",
                self.corpus_dir.display()
            );
        }

        sh_println!("minimized corpus: kept {kept}/{total} entries in {}", self.out.display())?;
        if skipped > 0 {
            sh_println!("skipped {skipped} entries or txs that could not be read or replayed")?;
        }
        Ok(())
    }
}

/// Minimize one corpus entry while preserving its failure or coverage.
#[derive(Clone, Debug, Parser)]
pub struct FuzzTminArgs {
    #[command(flatten)]
    filter: FilterArgs,
    /// Input corpus file.
    #[arg(value_name = "INPUT", value_hint = ValueHint::FilePath)]
    input: PathBuf,
    /// Output corpus file.
    #[arg(long, short, value_name = "FILE", value_hint = ValueHint::FilePath)]
    out: PathBuf,
    /// Maximum candidate replays to attempt.
    #[arg(long, default_value_t = 5000, value_name = "N")]
    max_attempts: usize,
}

impl FuzzTminArgs {
    async fn run(self) -> Result<()> {
        if self.out.exists() {
            bail!("output corpus file already exists: {}", self.out.display());
        }

        let mut sequence = read_sequence(&self.input)?;
        let before_txs = sequence.len();
        let mut test = minimizer_test_args(self.filter);
        test.enable_fuzz_only();
        test.reject_machine_unsupported_flags()?;
        let evm_edge_indices = Arc::new(Mutex::new(EdgeIndexMap::default()));
        let original =
            replay_candidate(&mut test, evm_edge_indices.clone(), sequence.clone()).await?;
        if original.replayed == 0 && !original.has_coverage() && original.failure.is_none() {
            bail!(
                "replayed 0 transactions from {}; check that --mc/--mt match the corpus entry",
                self.input.display()
            );
        }
        let mut attempts = 0usize;
        minimize_sequence(
            &mut test,
            evm_edge_indices.clone(),
            &original,
            &mut sequence,
            self.max_attempts,
            &mut attempts,
        )
        .await?;

        if self.out.extension() == Some("gz".as_ref()) {
            fs::write_json_gzip_file(&self.out, &sequence)?;
        } else {
            fs::write_json_file(&self.out, &sequence)?;
        }
        sh_println!("minimized entry: {} txs -> {}", sequence.len(), self.out.display())?;
        sh_println!(
            "removed {} txs after {attempts} candidate replays",
            before_txs - sequence.len()
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
        if calldata.len() < 4 {
            return None;
        }

        let selector = Selector::from_slice(&calldata[..4]);
        self.functions.get(&selector).and_then(|functions| {
            functions.iter().find_map(|indexed| {
                let decoded_args = indexed.function.abi_decode_input(&calldata[4..]).ok()?;
                let args = format_tokens_raw(&decoded_args).collect::<Vec<_>>();
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
        })
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
    if path.extension() == Some("gz".as_ref()) {
        Ok(fs::read_json_gzip_file(path)?)
    } else {
        Ok(fs::read_json_file(path)?)
    }
}

fn minimizer_test_args(filter: FilterArgs) -> TestArgs {
    let mut test = TestArgs::parse_from(["test", "-q"]);
    test.set_filter(filter);
    test
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

async fn replay_candidate(
    test: &mut TestArgs,
    evm_edge_indices: Arc<Mutex<EdgeIndexMap>>,
    sequence: Vec<BasicTxDetails>,
) -> Result<ReplayObservation> {
    let observations = Arc::new(Mutex::new(Vec::new()));
    test.set_fuzz_minimize_override(FuzzMinimizeConfig {
        input: sequence,
        evm_edge_indices,
        observations: observations.clone(),
    });
    let _quiet = QuietShellGuard::new();
    let _ = test.compile_and_run().await?;
    let observations = observations.lock().expect("minimize observations lock").clone();
    Ok(merge_observations(observations))
}

fn merge_observations(observations: Vec<ReplayObservation>) -> ReplayObservation {
    let mut merged = ReplayObservation::default();
    for observation in observations {
        merge_edge_vec(&mut merged.evm_edges, &observation.evm_edges);
        merge_edge_vec(&mut merged.sancov_edges, &observation.sancov_edges);
        merged.replayed += observation.replayed;
        merged.skipped += observation.skipped;
        if merged.failure.is_none() {
            merged.failure = observation.failure;
        }
    }
    merged
}

fn merge_edge_vec(dst: &mut Vec<u8>, src: &[u8]) {
    if dst.len() < src.len() {
        dst.resize(src.len(), 0);
    }
    for (dst, src) in dst.iter_mut().zip(src) {
        *dst = (*dst).max(*src);
    }
}

fn merge_new_edges(cumulative: &mut ReplayObservation, observation: &ReplayObservation) -> bool {
    let before = count_edges(cumulative);
    merge_edge_vec(&mut cumulative.evm_edges, &observation.evm_edges);
    merge_edge_vec(&mut cumulative.sancov_edges, &observation.sancov_edges);
    count_edges(cumulative) > before
}

fn count_edges(observation: &ReplayObservation) -> usize {
    observation.evm_edges.iter().filter(|&&edge| edge != 0).count()
        + observation.sancov_edges.iter().filter(|&&edge| edge != 0).count()
}

async fn minimize_sequence(
    test: &mut TestArgs,
    evm_edge_indices: Arc<Mutex<EdgeIndexMap>>,
    original: &ReplayObservation,
    sequence: &mut Vec<BasicTxDetails>,
    max_attempts: usize,
    attempts: &mut usize,
) -> Result<()> {
    let mut idx = 0;
    while idx < sequence.len() && *attempts < max_attempts {
        let mut candidate = sequence.clone();
        candidate.remove(idx);
        if accepts_candidate(
            test,
            evm_edge_indices.clone(),
            original,
            candidate.clone(),
            max_attempts,
            attempts,
        )
        .await?
        {
            *sequence = candidate;
        } else {
            idx += 1;
        }
    }

    let mut idx = 0;
    while idx < sequence.len() && *attempts < max_attempts {
        let mut candidate = sequence.clone();
        cleanup_metadata(&mut candidate[idx]);
        if accepts_candidate(
            test,
            evm_edge_indices.clone(),
            original,
            candidate.clone(),
            max_attempts,
            attempts,
        )
        .await?
        {
            *sequence = candidate;
        }
        idx += 1;
    }

    let mut tx_idx = 0;
    while tx_idx < sequence.len() && *attempts < max_attempts {
        for calldata in calldata_candidates(sequence[tx_idx].call_details.calldata.as_ref()) {
            if *attempts >= max_attempts {
                break;
            }
            let mut candidate = sequence.clone();
            candidate[tx_idx].call_details.calldata = calldata.into();
            if accepts_candidate(
                test,
                evm_edge_indices.clone(),
                original,
                candidate.clone(),
                max_attempts,
                attempts,
            )
            .await?
            {
                *sequence = candidate;
            }
        }
        tx_idx += 1;
    }
    Ok(())
}

async fn accepts_candidate(
    test: &mut TestArgs,
    evm_edge_indices: Arc<Mutex<EdgeIndexMap>>,
    original: &ReplayObservation,
    candidate: Vec<BasicTxDetails>,
    max_attempts: usize,
    attempts: &mut usize,
) -> Result<bool> {
    if *attempts >= max_attempts {
        return Ok(false);
    }
    *attempts += 1;
    let observed = replay_candidate(test, evm_edge_indices, candidate).await?;
    if let Some(failure) = &original.failure {
        Ok(observed.failure.as_ref() == Some(failure))
    } else {
        Ok(observed.failure.is_none() && covers_edges(&observed, original))
    }
}

fn covers_edges(candidate: &ReplayObservation, original: &ReplayObservation) -> bool {
    covers_edge_vec(&candidate.evm_edges, &original.evm_edges)
        && covers_edge_vec(&candidate.sancov_edges, &original.sancov_edges)
}

fn covers_edge_vec(candidate: &[u8], original: &[u8]) -> bool {
    original
        .iter()
        .enumerate()
        .all(|(idx, &edge)| edge == 0 || candidate.get(idx).copied().unwrap_or_default() != 0)
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

fn calldata_candidates(calldata: &[u8]) -> Vec<Vec<u8>> {
    if calldata.len() <= 4 {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    let mut offset = 4;
    while offset + 32 <= calldata.len() {
        for replacement in calldata_word_replacements() {
            if calldata[offset..offset + 32] == replacement {
                continue;
            }
            let mut candidate = calldata.to_vec();
            candidate[offset..offset + 32].copy_from_slice(&replacement);
            candidates.push(candidate);
        }
        offset += 32;
    }
    candidates
}

const fn calldata_word_replacements() -> [[u8; 32]; 3] {
    let zero = [0u8; 32];
    let mut one = [0u8; 32];
    one[31] = 1;
    let minus_one = [0xffu8; 32];
    [zero, one, minus_one]
}
