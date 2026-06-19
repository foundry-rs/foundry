use crate::{cmd::test::TestArgs, multi_runner::ShowmapConfig};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::Selector;
use clap::{Parser, Subcommand, ValueEnum, ValueHint};
use eyre::{Context, Result, bail};
use foundry_cli::json::{JsonEnvelope, print_json};
use foundry_common::{fmt::format_tokens_raw, fs, sh_println};
use foundry_config::Config;
use foundry_evm::{
    executors::{CorpusDirEntry, ShowmapDomain, read_corpus_tree},
    fuzz::BasicTxDetails,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
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
                args.run()?;
                Ok(crate::result::TestOutcome::empty(None, true))
            }
            FuzzSubcommands::Tmin(args) => {
                args.run()?;
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
            FuzzSubcommands::Show(args) => {
                print_json(&JsonEnvelope::success(args.run_machine()?))?;
                Ok(None)
            }
            FuzzSubcommands::Cmin(args) => {
                print_json(&JsonEnvelope::success(args.run_machine()?))?;
                Ok(None)
            }
            FuzzSubcommands::Tmin(args) => {
                print_json(&JsonEnvelope::success(args.run_machine()?))?;
                Ok(None)
            }
        }
    }
}

#[derive(Clone, Debug, Subcommand)]
pub enum FuzzSubcommands {
    /// Run only fuzz and invariant tests.
    Run(TestArgs),
    /// Replay persisted fuzz and invariant corpus entries without running a campaign.
    Replay(FuzzReplayArgs),
    /// Print persisted corpus entries.
    Show(FuzzShowArgs),
    /// Minimize a corpus by removing duplicate transaction sequences.
    Cmin(FuzzCminArgs),
    /// Minimize one transaction sequence by trimming no-op metadata and empty calls.
    Tmin(FuzzTminArgs),
}

/// Replay persisted fuzz/invariant corpus entries.
#[derive(Clone, Debug, Parser)]
pub struct FuzzReplayArgs {
    #[command(flatten)]
    test: TestArgs,
    /// Override the corpus directory to replay.
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
    fn run_machine(&self) -> Result<FuzzShowData> {
        let decoder = CorpusDecoder::load();
        Ok(FuzzShowData { entries: read_entries(&self.corpus, self.limit, &decoder)? })
    }

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

#[derive(Serialize)]
pub struct FuzzShowData {
    entries: Vec<DisplayCorpusEntry>,
}

/// Minimize a corpus by removing duplicate transaction sequences.
#[derive(Clone, Debug, Parser)]
pub struct FuzzCminArgs {
    /// Input corpus directory.
    #[arg(value_name = "CORPUS_DIR", value_hint = ValueHint::DirPath)]
    corpus_dir: PathBuf,
    /// Output corpus directory.
    #[arg(long, short, value_name = "DIR", value_hint = ValueHint::DirPath)]
    out: PathBuf,
}

impl FuzzCminArgs {
    fn run_machine(&self) -> Result<FuzzCminData> {
        if self.out.exists() {
            bail!("output corpus directory already exists: {}", self.out.display());
        }
        fs::create_dir_all(&self.out)?;

        let mut seen_sequences = HashSet::new();
        let mut kept = 0usize;
        let mut total = 0usize;
        for entry in read_corpus_entries(&self.corpus_dir)? {
            total += 1;
            let sequence = read_sequence(&entry.path)
                .with_context(|| format!("failed to read corpus entry {}", entry.path.display()))?;
            let key = serde_json::to_vec(&sequence)?;
            if !seen_sequences.insert(key) {
                continue;
            }
            let out = self.out.join(entry.path.file_name().expect("corpus entry has file name"));
            std::fs::copy(&entry.path, &out).with_context(|| {
                format!("failed to copy {} to {}", entry.path.display(), out.display())
            })?;
            kept += 1;
        }

        Ok(FuzzCminData {
            input: self.corpus_dir.clone(),
            output: self.out.clone(),
            total,
            kept,
            removed: total - kept,
        })
    }

    fn run(&self) -> Result<()> {
        let data = self.run_machine()?;
        sh_println!(
            "minimized corpus: kept {}/{} entries in {}",
            data.kept,
            data.total,
            data.output.display()
        )?;
        Ok(())
    }
}

#[derive(Serialize)]
pub struct FuzzCminData {
    input: PathBuf,
    output: PathBuf,
    total: usize,
    kept: usize,
    removed: usize,
}

/// Minimize one corpus entry.
#[derive(Clone, Debug, Parser)]
pub struct FuzzTminArgs {
    /// Input corpus file.
    #[arg(value_name = "INPUT", value_hint = ValueHint::FilePath)]
    input: PathBuf,
    /// Output corpus file.
    #[arg(long, short, value_name = "FILE", value_hint = ValueHint::FilePath)]
    out: PathBuf,
}

impl FuzzTminArgs {
    fn run_machine(&self) -> Result<FuzzTminData> {
        if self.out.exists() {
            bail!("output corpus file already exists: {}", self.out.display());
        }

        let mut sequence = read_sequence(&self.input)?;
        let before_txs = sequence.len();
        sequence.retain(|tx| !tx.call_details.calldata.is_empty());
        for tx in &mut sequence {
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

        if self.out.extension() == Some("gz".as_ref()) {
            fs::write_json_gzip_file(&self.out, &sequence)?;
        } else {
            fs::write_json_file(&self.out, &sequence)?;
        }
        Ok(FuzzTminData {
            input: self.input.clone(),
            output: self.out.clone(),
            before_txs,
            after_txs: sequence.len(),
            removed_txs: before_txs - sequence.len(),
        })
    }

    fn run(&self) -> Result<()> {
        let data = self.run_machine()?;
        sh_println!("minimized entry: {} txs -> {}", data.after_txs, data.output.display())?;
        Ok(())
    }
}

#[derive(Serialize)]
pub struct FuzzTminData {
    input: PathBuf,
    output: PathBuf,
    before_txs: usize,
    after_txs: usize,
    removed_txs: usize,
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
