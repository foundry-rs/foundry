// cast estimate subcommands
use crate::{
    opts::{EthereumOpts, TransactionOpts},
    utils::{self},
};
use cast::{Cast, TxBuilder};
use clap::Parser;
use ethers::{
    providers::Middleware,
    types::{BlockId, NameOrAddress},
};
use eyre::WrapErr;
use foundry_config::{Chain, Config};
use std::str::FromStr;

#[derive(Debug, Parser)]
pub struct AccessListArgs {
    #[clap(
        help = "The destination of the transaction.", 
        value_name = "TO",
        value_parser = NameOrAddress::from_str
    )]
    to: Option<NameOrAddress>,

    #[clap(help = "The signature of the function to call.", value_name = "SIG")]
    sig: Option<String>,

    #[clap(help = "The arguments of the function to call.", value_name = "ARGS")]
    args: Vec<String>,

    #[clap(
        long,
        help = "Data for the transaction.",
        value_name = "DATA",
        value_parser = foundry_common::clap_helpers::strip_0x_prefix,
        conflicts_with_all = &["sig", "args"]
    )]
    data: Option<String>,

    #[clap(flatten)]
    tx: TransactionOpts,

    #[clap(flatten)]
    eth: EthereumOpts,

    #[clap(
        long,
        short,
        help = "The block height you want to query at.",
        long_help = "The block height you want to query at. Can also be the tags earliest, finalized, safe, latest, or pending.",
        value_name = "BLOCK"
    )]
    block: Option<BlockId>,

    #[clap(long = "json", short = 'j', help_heading = "Display options")]
    to_json: bool,
}

impl AccessListArgs {
    pub async fn run(self) -> eyre::Result<()> {
        let AccessListArgs { to, sig, args, data, tx, eth, block, to_json } = self;

        let config = Config::from(&eth);
        let provider = utils::get_provider(&config)?;
        let chain = utils::get_chain(config.chain_id, &provider).await?;
        let sender = eth.wallet.sender().await;

        let provider = utils::get_provider(&config)?;
        access_list(&provider, sender, to, sig, args, data, tx, chain, block, to_json).await?;
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn access_list<M: Middleware, F: Into<NameOrAddress>, T: Into<NameOrAddress>>(
    provider: M,
    from: F,
    to: Option<T>,
    sig: Option<String>,
    args: Vec<String>,
    data: Option<String>,
    tx: TransactionOpts,
    chain: Chain,
    block: Option<BlockId>,
    to_json: bool,
) -> eyre::Result<()>
where
    M::Error: 'static,
{
    let mut builder = TxBuilder::new(&provider, from, to, chain, tx.legacy).await?;
    builder
        .gas(tx.gas_limit)
        .gas_price(tx.gas_price)
        .priority_gas_price(tx.priority_gas_price)
        .nonce(tx.nonce);

    builder.value(tx.value);

    if let Some(sig) = sig {
        builder.set_args(sig.as_str(), args).await?;
    }
    if let Some(data) = data {
        // Note: `sig+args` and `data` are mutually exclusive
        builder.set_data(hex::decode(data).wrap_err("Expected hex encoded function data")?);
    }

    let builder_output = builder.peek();

    let cast = Cast::new(&provider);

    let access_list: String = cast.access_list(builder_output, block, to_json).await?;

    println!("{}", access_list);

    Ok(())
}
