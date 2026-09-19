//! Compare Cast BAL execution and replay using the same binary.

use clap::{Args, Parser, Subcommand};
use eyre::Result;
use std::path::PathBuf;

mod bal;

#[derive(Parser)]
#[command(name = "foundry-bal-bench", about = "Measure Cast BAL latency and RPC overhead")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Freeze a metadata-selected finalized block panel before probing BAL availability.
    Capture(CaptureArgs),
    /// Compare ordinary Cast with --no-bal replay; optionally inject an unsupported BAL method.
    Run(RunArgs),
    /// Recompute summaries from saved samples without accessing the endpoint.
    Report {
        #[arg(long)]
        output_dir: PathBuf,
    },
    /// Inspect the effective configuration in the same environment as a Cast child.
    #[command(hide = true)]
    ConfigCheck,
}

#[derive(Args, Clone)]
struct EndpointArgs {
    /// Existing environment variable holding the HTTP endpoint. Defaults to configured RPC,
    /// then the repository's archive test endpoint.
    #[arg(long)]
    rpc_env: Option<String>,
}

#[derive(Args)]
struct CaptureArgs {
    #[command(flatten)]
    endpoint: EndpointArgs,
    #[arg(long)]
    output_dir: PathBuf,
    /// Non-sensitive provider identifier saved in artifacts.
    #[arg(long, default_value = "repository-archive")]
    endpoint_label: String,
    /// Last block in the candidate interval. Defaults to the finalized head.
    #[arg(long)]
    end_block: Option<u64>,
    #[arg(long, default_value_t = 12)]
    blocks: usize,
    /// Contiguous metadata candidate interval size; defaults to ten times --blocks.
    #[arg(long)]
    candidate_blocks: Option<usize>,
    #[arg(long, default_value_t = 7928)]
    seed: u64,
}

#[derive(Args)]
struct RunArgs {
    #[command(flatten)]
    endpoint: EndpointArgs,
    #[arg(long)]
    manifest: PathBuf,
    /// Optional provenance from pr-bal-bench.sh; verifies the binary hash and build settings.
    #[arg(long)]
    build_manifest: Option<PathBuf>,
    /// Cast binary supporting --no-bal, used for both comparison arms.
    #[arg(long)]
    cast: PathBuf,
    /// Add a same-binary control with a locally injected method-not-found BAL response.
    #[arg(long)]
    include_miss: bool,
    #[arg(long)]
    output_dir: PathBuf,
    #[arg(long, default_value_t = 10)]
    rounds: usize,
    #[arg(long, default_value_t = 2)]
    warmup_rounds: usize,
    #[arg(long, default_value_t = 0)]
    round_offset: usize,
    #[arg(long)]
    warmup_only: bool,
    #[arg(long, default_value_t = 120)]
    timeout_seconds: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Capture(args) => bal::capture::capture(args).await,
        Command::Run(args) => bal::run(args).await,
        Command::Report { output_dir } => bal::results::report(&output_dir),
        Command::ConfigCheck => bal::config_check(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command};
    use clap::Parser;

    #[test]
    fn run_accepts_one_binary_without_a_build_manifest() {
        let cli = Cli::try_parse_from([
            "foundry-bal-bench",
            "run",
            "--cast",
            "/tmp/cast",
            "--manifest",
            "/tmp/panel.json",
            "--output-dir",
            "/tmp/results",
        ])
        .expect("an existing Cast binary is sufficient for a same-binary comparison");
        assert!(matches!(cli.command, Command::Run(_)));
    }
}
