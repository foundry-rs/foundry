//! The `anvil` cli
use anvil::cmd::NodeArgs;
use clap::Parser;

#[derive(Debug, Parser)]
#[clap(name = "anvil", version = anvil::VERSION_MESSAGE)]
pub struct App {
    #[clap(flatten)]
    pub node: NodeArgs,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = App::parse();
    let _ = fdlimit::raise_fd_limit();
    app.node.run().await?;

    Ok(())
}
