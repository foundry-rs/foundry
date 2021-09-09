use structopt::StructOpt;

use dapptools::opts::{Opts, Subcommands};
use dapptools::seth::Seth;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let opts = Opts::from_args();
    match opts.sub {
        Subcommands::FromAscii(inner) => {
            println!("{}", Seth::from_ascii(&inner.text));
        }
        Subcommands::ToCheckSumAddress(inner) => {
            println!("{}", Seth::to_checksum_address(&inner.address)?);
        }
    };

    Ok(())
}
