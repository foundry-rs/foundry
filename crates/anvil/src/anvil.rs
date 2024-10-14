//! The `anvil` cli

use anvil::cmd::NodeArgs;
use clap::{CommandFactory, Parser, Subcommand};
use foundry_cli::utils;

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
}

fn main() -> eyre::Result<()> {
    utils::load_dotenv();

    let mut args = Anvil::parse();
    args.node.evm_opts.resolve_rpc_alias();

    if let Some(cmd) = &args.cmd {
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
        }
        return Ok(())
    }

    let _ = fdlimit::raise_fd_limit();
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(args.node.run())
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
