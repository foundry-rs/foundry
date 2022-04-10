//! cast call subcommand
use crate::opts::{
    cast::{parse_block_id, parse_name_or_address},
    EthereumOpts,
};
use clap::Parser;
use ethers::types::{BlockId, NameOrAddress};
use eyre::Result;
use foundry_config::{figment, impl_eth_data_provider, impl_figment_convert_cast};
use serde::Serialize;

impl_figment_convert_cast!(CallArgs);
impl_eth_data_provider!(CallArgs);

#[derive(Debug, Clone, Parser, Serialize)]
pub struct CallArgs {
    #[clap(help = "the address you want to query", parse(try_from_str = parse_name_or_address))]
    #[serde(skip)]
    pub address: NameOrAddress,
    #[serde(skip)]
    pub sig: String,
    #[serde(skip)]
    pub args: Vec<String>,
    #[clap(long, short, help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id))]
    #[serde(skip)]
    pub block: Option<BlockId>,
    #[clap(flatten)]
    #[serde(flatten)]
    pub eth: EthereumOpts,
}
