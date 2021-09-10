use structopt::StructOpt;

use dapptools::opts::{Opts, Subcommands};
use dapptools::seth::Seth;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let opts = Opts::from_args();
    match opts.sub {
        Subcommands::FromAscii { text } => {
            println!("{}", Seth::from_ascii(&text));
        }
        Subcommands::ToCheckSumAddress { address } => {
            println!("{}", Seth::to_checksum_address(&address)?);
        }
    };

    Ok(())
}
