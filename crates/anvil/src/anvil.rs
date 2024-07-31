//! The `anvil` cli

use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use anvil::cmd::NodeArgs;
use clap::{CommandFactory, Parser, Subcommand};
use foundry_cli::utils;
use foundry_common::{fs::read_json_file, provider::get_http_provider};

#[cfg(all(feature = "jemalloc", unix))]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// A fast local Ethereum development node.
#[derive(Parser)]
#[command(name = "anvil", version = anvil::VERSION_MESSAGE, next_display_order = None)]
pub struct Anvil {
    #[command(flatten)]
    pub node: NodeArgs,

    #[command(subcommand)]
    pub cmd: Option<AnvilSubcommand>,
}

#[derive(Subcommand)]
pub enum AnvilSubcommand {
    /// Generate shell completions script.
    #[command(visible_alias = "com")]
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Generate Fig autocompletion spec.
    #[command(visible_alias = "fig")]
    GenerateFigSpec,

    /// Reorg chain of live anvil instance.
    Reorg {
        // The depth of the reorg. This must not exceed current chain height
        depth: u64,
        // The lebngth of the newly reorged chain
        new_len: u64,
        /// Path to JSON file containing transaction requests and block number pairs
        #[arg(long, short)]
        transactions_path: Option<String>,
        /// The provider URL of the local anvil node. Defaults to localhost:8545
        #[arg(long, short)]
        rpc_url: Option<String>,
    },
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    utils::load_dotenv();

    let mut app = Anvil::parse();
    app.node.evm_opts.resolve_rpc_alias();

    if let Some(ref cmd) = app.cmd {
        match cmd {
            AnvilSubcommand::Completions { shell } => {
                clap_complete::generate(
                    *shell,
                    &mut Anvil::command(),
                    "anvil",
                    &mut std::io::stdout(),
                );
            }
            AnvilSubcommand::GenerateFigSpec => clap_complete::generate(
                clap_complete_fig::Fig,
                &mut Anvil::command(),
                "anvil",
                &mut std::io::stdout(),
            ),
            AnvilSubcommand::Reorg { depth, new_len, transactions_path, rpc_url } => {
                let url = rpc_url.clone().unwrap_or("127.0.0.1:8545".to_string());
                let provider = get_http_provider(url);

                let tx_block_pairs = if let Some(path) = transactions_path {
                    read_json_file::<Vec<(TransactionRequest, u64)>>(path.as_ref())?
                } else {
                    Vec::new()
                };

                provider
                    .raw_request(
                        "anvil_reorg".into(),
                        serde_json::json!([depth, new_len, tx_block_pairs,]),
                    )
                    .await?;
            }
        }
        return Ok(())
    }

    let _ = fdlimit::raise_fd_limit();
    app.node.run().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_cli() {
        Anvil::command().debug_assert();
    }

    #[test]
    fn can_parse_help() {
        let _: Anvil = Anvil::parse_from(["anvil", "--help"]);
    }

    #[test]
    fn can_parse_completions() {
        let args: Anvil = Anvil::parse_from(["anvil", "completions", "bash"]);
        assert!(matches!(
            args.cmd,
            Some(AnvilSubcommand::Completions { shell: clap_complete::Shell::Bash })
        ));
    }
}
