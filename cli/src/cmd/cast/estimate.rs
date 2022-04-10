//! cast estimate subcommand
use crate::opts::{cast::parse_name_or_address, EthereumOpts};
use clap::Parser;
use ethers::types::{NameOrAddress, U256};
use eyre::Result;
use foundry_config::{figment, impl_eth_data_provider, impl_figment_convert_cast};
use serde::Serialize;

impl_figment_convert_cast!(EstimateArgs);
impl_eth_data_provider!(EstimateArgs);

#[derive(Debug, Clone, Parser, Serialize)]
pub struct EstimateArgs {
    #[clap(help = "the address you want to transact with", parse(try_from_str = parse_name_or_address))]
    #[serde(skip)]
    pub to: NameOrAddress,
    #[clap(help = "the function signature or name you want to call")]
    #[serde(skip)]
    pub sig: String,
    #[clap(help = "the list of arguments you want to call the function with")]
    #[serde(skip)]
    pub args: Vec<String>,
    #[clap(long, help = "value for tx estimate (in wei)")]
    #[serde(skip)]
    pub value: Option<U256>,
    #[clap(flatten)]
    #[serde(flatten)]
    pub eth: EthereumOpts,
}
