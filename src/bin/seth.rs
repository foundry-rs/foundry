use structopt::StructOpt;

use dapptools::opts::{Opts, Subcommands};
use dapptools::Seth;

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
        Subcommands::ToBytes32 { bytes } => {
            println!("{}", Seth::to_bytes32(&bytes)?);
        }
        Subcommands::Block {
            rpc_url,
            block,
            full,
            field,
            to_json,
        } => {
            println!(
                "{}",
                Seth::new(&rpc_url)
                    .await?
                    .block(block, full, field, to_json)
                    .await?
            );
        }
        Subcommands::Call {
            rpc_url,
            address,
            sig,
            args,
        } => {
            println!(
                "{}",
                Seth::new(&rpc_url).await?.call(address, &sig, args).await?
            );
        }
    };

    Ok(())
}
