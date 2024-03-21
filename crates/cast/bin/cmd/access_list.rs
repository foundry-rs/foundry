use alloy_network::Ethereum;
use alloy_primitives::Address;
use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
use alloy_transport::Transport;
use cast::{Cast, TxBuilder};
use clap::Parser;
use eyre::{Result, WrapErr};
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils,
};
use foundry_common::ens::NameOrAddress;
use foundry_config::{Chain, Config};
use std::str::FromStr;

/// CLI arguments for `cast access-list`.
#[derive(Debug, Parser)]
pub struct AccessListArgs {
    /// The destination of the transaction.
    #[arg(
        value_name = "TO",
        value_parser = NameOrAddress::from_str
    )]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    #[arg(value_name = "SIG")]
    sig: Option<String>,

    /// The arguments of the function to call.
    #[arg(value_name = "ARGS")]
    args: Vec<String>,

    /// The data for the transaction.
    #[arg(
        long,
        value_name = "DATA",
        conflicts_with_all = &["sig", "args"]
    )]
    data: Option<String>,

    /// The block height to query at.
    ///
    /// Can also be the tags earliest, finalized, safe, latest, or pending.
    #[arg(long, short = 'B')]
    block: Option<BlockId>,

    /// Print the access list as JSON.
    #[arg(long, short, help_heading = "Display options")]
    json: bool,

    #[command(flatten)]
    tx: TransactionOpts,

    #[command(flatten)]
    eth: EthereumOpts,
}

impl AccessListArgs {
    pub async fn run(self) -> Result<()> {
        let AccessListArgs { to, sig, args, data, tx, eth, block, json: to_json } = self;

        let config = Config::from(&eth);
        let provider = utils::get_alloy_provider(&config)?;
        let chain = utils::get_chain(config.chain, &provider).await?;
        let sender = eth.wallet.sender().await;

        let to = match to {
            Some(NameOrAddress::Name(name)) => {
                Some(NameOrAddress::Name(name).resolve(&provider).await?)
            }
            Some(NameOrAddress::Address(addr)) => Some(addr),
            None => None,
        };

        access_list(&provider, sender, to, sig, args, data, tx, chain, block, to_json).await?;
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn access_list<P: Provider<Ethereum, T>, T: Transport + Clone>(
    provider: P,
    from: Address,
    to: Option<Address>,
    sig: Option<String>,
    args: Vec<String>,
    data: Option<String>,
    tx: TransactionOpts,
    chain: Chain,
    block: Option<BlockId>,
    to_json: bool,
) -> Result<()> {
    let mut builder = TxBuilder::new(&provider, from, to, chain, tx.legacy).await?;
    builder
        .gas(tx.gas_limit)
        .gas_price(tx.gas_price)
        .priority_gas_price(tx.priority_gas_price)
        .nonce(tx.nonce.map(|n| n.to()));

    builder.value(tx.value);

    if let Some(sig) = sig {
        builder.set_args(sig.as_str(), args).await?;
    }
    if let Some(data) = data {
        // Note: `sig+args` and `data` are mutually exclusive
        builder.set_data(hex::decode(data).wrap_err("Expected hex encoded function data")?.into());
    }

    let builder_output = builder.peek();

    let cast = Cast::new(&provider);

    let access_list: String = cast.access_list(builder_output, block, to_json).await?;

    println!("{}", access_list);

    Ok(())
}
