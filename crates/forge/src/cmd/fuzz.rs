use crate::{
    cmd::test::{FilterArgs, FuzzMinimizeReplaySession, TestArgs},
    multi_runner::{FuzzMinimizeEdgeIndices, FuzzMinimizeObservation, ShowmapConfig},
    result::TestOutcome,
};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::Selector;
use clap::{Parser, Subcommand, ValueEnum, ValueHint};
use eyre::{Context, Result, bail};
use foundry_cli::opts::{BuildOpts, EvmArgs, GlobalArgs};
use foundry_common::{
    fmt::format_tokens_raw,
    fs, sh_println, sh_status,
    shell::{OutputMode, Shell},
};
use foundry_config::Config;
use foundry_evm::{
    executors::{CorpusDirEntry, ReplayObservation, ShowmapDomain, read_corpus_tree},
    fuzz::BasicTxDetails,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
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
        }
    }

    pub const fn is_junit(&self) -> bool {
        match &self.command {
            FuzzSubcommands::Run(args) => args.junit,
            FuzzSubcommands::Replay(args) => args.is_junit(),
            FuzzSubcommands::Show(_) | FuzzSubcommands::Cmin(_) => false,
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
    // TODO(@mablr): add corpus test case minimization subcommand `tmin`.
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
        let mut successful_replays = 0usize;
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
                successful_replays += entry_replayed;
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
                "replayed 0 transactions from {corpus}; {successful_replays} successful replayed \
                 transactions, {unreadable} unreadable entries, {empty} empty entries, \
                 {rejected_txs} rejected transactions"
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

    #[test]
    fn merge_new_edges_keeps_sancov_hit_count_bucket_increases() {
        let mut cumulative = ReplayObservation { sancov_edges: vec![0, 1], ..Default::default() };
        let candidate = ReplayObservation { sancov_edges: vec![0, 8], ..Default::default() };

        assert!(merge_new_edges(&mut cumulative, &candidate));
        assert_eq!(cumulative.sancov_edges, vec![0, 8]);
    }
}
