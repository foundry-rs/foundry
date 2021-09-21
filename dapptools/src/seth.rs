mod seth_opts;
use seth_opts::{Opts, Subcommands};

use seth::{Seth, SimpleSeth};

use ethers::{
    middleware::SignerMiddleware,
    providers::{Middleware, Provider},
    signers::Signer,
    types::NameOrAddress,
};
use std::{convert::TryFrom, str::FromStr};
use structopt::StructOpt;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let opts = Opts::from_args();
    match opts.sub {
        Subcommands::FromUtf8 { text } => {
            println!("{}", SimpleSeth::from_utf8(&text));
        }
        Subcommands::ToHex { decimal } => {
            println!("{}", SimpleSeth::hex(unwrap_or_stdin(decimal)?));
        }
        Subcommands::ToCheckSumAddress { address } => {
            println!("{}", SimpleSeth::checksum_address(&address)?);
        }
        Subcommands::ToBytes32 { bytes } => {
            println!("{}", SimpleSeth::bytes32(&bytes)?);
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
            println!("{}", Seth::new(provider).call(address, &sig, args).await?);
        }
        Subcommands::SendTx { eth, to, sig, args } => {
            let provider = Provider::try_from(eth.rpc_url.as_str())?;
            if let Some(signer) = eth.signer()? {
                let from = eth.from.unwrap_or_else(|| signer.address());
                let provider = SignerMiddleware::new(provider, signer);
                seth_send(provider, from, to, sig, args, eth.seth_async).await?;
            } else {
                let from = eth.from.expect("No ETH_FROM or signer specified");
                seth_send(provider, from, to, sig, args, eth.seth_async).await?;
            }
        }
        Subcommands::Balance {
            block,
            who,
            rpc_url,
        } => {
            let provider = Provider::try_from(rpc_url)?;
            println!("{}", Seth::new(provider).balance(who, block).await?);
        }
        Subcommands::ResolveName {
            who,
            rpc_url,
            verify,
        } => {
            let provider = Provider::try_from(rpc_url)?;
            let who = unwrap_or_stdin(who)?;
            let address = provider.resolve_name(&who).await?;
            if verify {
                let name = provider.lookup_address(address).await?;
                assert_eq!(
                    name, who,
                    "forward lookup verification failed. got {}, expected {}",
                    name, who
                );
            }
            println!("{:?}", address);
        }
        Subcommands::LookupAddress {
            who,
            rpc_url,
            verify,
        } => {
            let provider = Provider::try_from(rpc_url)?;
            let who = unwrap_or_stdin(who)?;
            let name = provider.lookup_address(who).await?;
            if verify {
                let address = provider.resolve_name(&name).await?;
                assert_eq!(
                    address, who,
                    "forward lookup verification failed. got {}, expected {}",
                    name, who
                );
            }
            println!("{}", name);
        }
    };

    Ok(())
}

fn unwrap_or_stdin<T>(what: Option<T>) -> eyre::Result<T>
where
    T: FromStr + Send + Sync,
    T::Err: Send + Sync + std::error::Error + 'static,
{
    Ok(match what {
        Some(what) => what,
        None => {
            use std::io::Read;
            let mut input = std::io::stdin();
            let mut what = String::new();
            input.read_to_string(&mut what)?;
            T::from_str(&what.replace("\n", ""))?
        }
    })
}

async fn seth_send<M: Middleware, F: Into<NameOrAddress>, T: Into<NameOrAddress>>(
    provider: M,
    from: F,
    to: T,
    sig: String,
    args: Vec<String>,
    seth_async: bool,
) -> eyre::Result<()>
where
    M::Error: 'static,
{
    let seth = Seth::new(provider);
    let pending_tx = seth
        .send(
            from,
            to,
            if !sig.is_empty() {
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
            .ok_or_else(|| eyre::eyre!("tx {} not found", tx_hash))?;
        println!("Receipt: {:?}", receipt);
    }

    Ok(())
}
