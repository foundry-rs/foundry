// cast estimate subcommands
use crate::{
    opts::{EthereumOpts, TransactionOpts},
    utils::{self},
};
use cast::{Cast, TxBuilder};
use clap::Parser;
use ethers::types::{BlockId, NameOrAddress};
use eyre::WrapErr;
use foundry_config::Config;
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

        let mut builder = TxBuilder::new(&provider, sender, to, chain, tx.legacy).await?;
        builder
            .gas(tx.gas_limit)
            .etherscan_api_key(config.get_etherscan_api_key(Some(chain)))
            .gas_price(tx.gas_price)
            .priority_gas_price(tx.priority_gas_price)
            .nonce(tx.nonce);
    
            builder.value(tx.value);

        if let Some(sig) = sig {
            builder.set_args(sig.as_str(), args).await?;
        }
        if let Some(data) = data {
            // Note: `sig+args` and `data` are mutually exclusive
            builder.set_data(
                hex::decode(data).wrap_err("Expected hex encoded function data")?,
            );
        }        

        let builder_output = builder.peek();
        println!("{}", Cast::new(&provider).access_list(builder_output, block, to_json).await?);
        Ok(())
    }
}