//! The `anvil` cli
use anvil::cmd::NodeArgs;
use clap::Parser;

/// `anvil 0.1.0 (f01b232bc 2022-04-13T23:28:39.493201+00:00)`
pub(crate) const VERSION_MESSAGE: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("VERGEN_GIT_SHA_SHORT"),
    " ",
    env!("VERGEN_BUILD_TIMESTAMP"),
    ")"
);

#[derive(Debug, Parser)]
#[clap(name = "anvil", version = VERSION_MESSAGE)]
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
