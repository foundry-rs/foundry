//! The `anvil` cli
use anvil::cmd::NodeArgs;
use clap::{CommandFactory, Parser, Subcommand};

/// A fast local Ethereum development node.
#[derive(Debug, Parser)]
#[clap(name = "anvil", version = anvil::VERSION_MESSAGE, next_display_order = None)]
pub struct App {
    #[clap(flatten)]
    pub node: NodeArgs,

    #[clap(subcommand)]
    pub cmd: Option<Commands>,
}

#[derive(Clone, Debug, PartialEq, Eq, Subcommand)]
pub enum Commands {
    /// Generate shell completions script.
    #[clap(visible_alias = "com")]
    Completions {
        #[clap(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Generate Fig autocompletion spec.
    #[clap(visible_alias = "fig")]
    GenerateFigSpec,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::parse();
    app.node.evm_opts.resolve_rpc_alias();

    if let Some(ref cmd) = app.cmd {
        match cmd {
            Commands::Completions { shell } => {
                clap_complete::generate(
                    *shell,
                    &mut App::command(),
                    "anvil",
                    &mut std::io::stdout(),
                );
            }
            Commands::GenerateFigSpec => clap_complete::generate(
                clap_complete_fig::Fig,
                &mut App::command(),
                "anvil",
                &mut std::io::stdout(),
            ),
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
    fn can_parse_help() {
        let _: App = App::parse_from(["anvil", "--help"]);
    }

    #[test]
    fn can_parse_completions() {
        let args: App = App::parse_from(["anvil", "completions", "bash"]);
        assert_eq!(args.cmd, Some(Commands::Completions { shell: clap_complete::Shell::Bash }));
    }
}
