use std::convert::TryFrom;

use ethers::{
    prelude::SignerMiddleware,
    providers::{Middleware, Provider},
    signers::Signer,
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
        Subcommands::SendTx { eth, to, sig, args } => {
            let provider = Provider::try_from(eth.rpc_url.as_str())?;
            if let Some(signer) = eth.signer()? {
                let from = eth.from.unwrap_or(signer.address());
                let provider = SignerMiddleware::new(provider, signer);
                seth_send(provider, from, to, sig, args, eth.seth_async).await?;
            } else {
                let from = eth.from.expect("No ETH_FROM or signer specified");
                seth_send(provider, from, to, sig, args, eth.seth_async).await?;
            }
        }
    };

    Ok(())
}

async fn seth_send<M: Middleware>(
    provider: M,
    from: Address,
    to: Address,
    sig: String,
    args: Vec<String>,
    seth_async: bool,
) -> eyre::Result<()>
where
    M::Error: 'static,
{
    let seth = Seth::new(provider).await?;
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
