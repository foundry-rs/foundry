//! Arguments for running fuzz and invariant tests.

use super::test::{CampaignArgs, FilterArgs, ShowmapDomainArg};
use clap::{Parser, ValueHint};
use foundry_cli::opts::{BuildOpts, EvmArgs, GlobalArgs};
use foundry_config::filter::GlobMatcher;
use std::path::PathBuf;

/// Run only fuzz and invariant tests.
#[derive(Clone, Debug, Parser)]
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
    #[arg(long, value_name = "PATH", value_hint = ValueHint::FilePath, conflicts_with = "list")]
    pub(crate) fuzz_input_file: Option<PathBuf>,
}
