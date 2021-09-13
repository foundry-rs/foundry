mod opts;
use opts::EthereumOpts;

use seth::{Seth, SimpleSeth};

use ethers::{
    middleware::SignerMiddleware,
    providers::{Middleware, Provider},
    signers::Signer,
    types::{Address, BlockId, BlockNumber, H256, U64},
};
use std::{convert::TryFrom, str::FromStr};
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(about = "Perform Ethereum RPC calls from the comfort of your command line.")]
pub enum Subcommands {
    #[structopt(name = "--from-ascii")]
    #[structopt(about = "convert text data into hexdata")]
    FromAscii { text: String },
    #[structopt(name = "--to-checksum-address")]
    #[structopt(about = "convert an address to a checksummed format (EIP-55)")]
    ToCheckSumAddress { address: Address },
    #[structopt(name = "--to-bytes32")]
    #[structopt(about = "left-pads a hex bytes string to 32 bytes)")]
    ToBytes32 { bytes: String },
    #[structopt(name = "block")]
    #[structopt(
        about = "Prints information about <block>. If <field> is given, print only the value of that field"
    )]
    Block {
        #[structopt(help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id))]
        block: BlockId,
        #[structopt(long, env = "SETH_FULL_BLOCK")]
        full: bool,
        field: Option<String>,
        #[structopt(long = "--json", short = "-j")]
        to_json: bool,
        #[structopt(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[structopt(name = "call")]
    #[structopt(about = "Perform a local call to <to> without publishing a transaction.")]
    Call {
        #[structopt(help = "the address you want to query")]
        address: Address,
        sig: String,
        args: Vec<String>,
        #[structopt(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[structopt(name = "send")]
    #[structopt(about = "Publish a transaction signed by <from> to call <to> with <data>")]
    SendTx {
        #[structopt(help = "the address you want to transact with")]
        to: Address,
        #[structopt(help = "the function signature you want to call")]
        sig: String,
        #[structopt(help = "the list of arguments you want to call the function with")]
        args: Vec<String>,
        #[structopt(flatten)]
        eth: EthereumOpts,
    },
}

fn parse_block_id(s: &str) -> eyre::Result<BlockId> {
    Ok(match s {
        "earliest" => BlockId::Number(BlockNumber::Earliest),
        "latest" => BlockId::Number(BlockNumber::Latest),
        s if s.starts_with("0x") => BlockId::Hash(H256::from_str(s)?),
        s => BlockId::Number(BlockNumber::Number(U64::from_str(s)?)),
    })
}

#[derive(Debug, StructOpt)]
pub struct Opts {
    #[structopt(subcommand)]
    pub sub: Subcommands,
}

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
                let from = eth.from.unwrap_or_else(|| signer.address());
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
