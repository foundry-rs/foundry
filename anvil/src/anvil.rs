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

#[derive(Clone, Debug, Subcommand)]
pub enum Commands {
    #[clap(visible_alias = "com", about = "Generate shell completions script.")]
    Completions {
        #[clap(arg_enum)]
        shell: clap_complete::Shell,
    },
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
        }
        return Ok(())
    }

    let _ = fdlimit::raise_fd_limit();
    app.node.run().await?;

    Ok(())
}
