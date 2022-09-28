//! The `anvil` cli
use anvil::cmd::NodeArgs;
use clap::{CommandFactory, Parser, Subcommand};

/// A fast local Ethereum development node.
#[derive(Debug, Parser)]
#[clap(name = "anvil", version = anvil::VERSION_MESSAGE)]
pub struct App {
    #[clap(flatten)]
    pub node: NodeArgs,

    #[clap(subcommand)]
    pub cmd: Option<Commands>,
}

#[derive(Clone, Debug, Subcommand, Eq, PartialEq)]
pub enum Commands {
    #[clap(visible_alias = "com", about = "Generate shell completions script.")]
    Completions {
        #[clap(value_enum)]
        shell: clap_complete::Shell,
    },
    #[clap(visible_alias = "fig", about = "Generate Fig autocompletion spec.")]
    GenerateFigSpec,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = App::parse();

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
