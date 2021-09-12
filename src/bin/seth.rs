use std::convert::TryFrom;

use ethers::{
    providers::{Middleware, Provider},
    types::Address,
};
use structopt::StructOpt;

use dapptools::opts::{Opts, Subcommands};
use dapptools::{Seth, SimpleSeth};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let opts = Opts::from_args();
    match opts.sub {
        Subcommands::FromAscii { text } => {
            println!("{}", SimpleSeth::from_ascii(&text));
        }
        Subcommands::ToCheckSumAddress { address } => {
            println!("{}", SimpleSeth::to_checksum_address(&address)?);
        }
        Subcommands::ToBytes32 { bytes } => {
            println!("{}", SimpleSeth::to_bytes32(&bytes)?);
        }
        Subcommands::Block {
            rpc_url,
            block,
            full,
            field,
            to_json,
        } => {
            let provider = Provider::try_from(rpc_url)?;
            println!(
                "{}",
                Seth::new(provider)
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
            let provider = Provider::try_from(rpc_url)?;
            println!(
                "{}",
                Seth::new(provider).await?.call(address, &sig, args).await?
            );
        }
        Subcommands::SendTx {
            seth_async,
            rpc_url,
            address,
            sig,
            args,
            from,
        } => {
            let provider = Provider::try_from(rpc_url)?;
            let seth = Seth::new(provider).await?;
            seth_send(seth, from, address, sig, args, seth_async).await?;
        }
    };

    Ok(())
}

async fn seth_send<M: Middleware>(
    seth: Seth<M>,
    from: Address,
    to: Address,
    sig: String,
    args: Vec<String>,
    seth_async: bool,
) -> eyre::Result<()>
where
    M::Error: 'static,
{
    let pending_tx = seth
        .send(
            from,
            to,
            if sig.len() > 0 {
                Some((&sig, args))
            } else {
                None
            },
        )
        .await?;
    let tx_hash = *pending_tx;

    if seth_async {
        println!("{}", tx_hash);
    } else {
        let receipt = pending_tx
            .await?
            .ok_or(eyre::eyre!("tx {} not found", tx_hash))?;
        println!("Receipt: {:?}", receipt);
    }

    Ok(())
}
