use structopt::StructOpt;

use dapptools::opts::{Opts, Subcommands};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let opts = Opts::from_args();
    match opts.sub {
        Subcommands::Buy(inner) => {
        }
        Subcommands::Deploy(inner) => {
        }
        Subcommands::Prices(inner) => {
        }
    };

    Ok(())
}
