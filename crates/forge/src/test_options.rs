//! Engine-independent test execution options.

use foundry_evm_coverage::ShowmapDomain;
use foundry_evm_fuzz::BaseCounterExample;
use foundry_evm_networks::NetworkVariant;
use std::{path::PathBuf, sync::Arc};

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
