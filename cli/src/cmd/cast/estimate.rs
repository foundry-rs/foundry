//! cast call subcommand
use crate::opts::{cast::parse_name_or_address, EthereumOpts};
use ethers::types::U256;

use clap::Parser;
use ethers::types::NameOrAddress;
use eyre::Result;
use foundry_config::{
    figment::{
        self,
        value::{Dict, Map, Value},
        Metadata, Profile,
    },
    impl_figment_convert_cast, Config,
};

use serde::Serialize;

impl_figment_convert_cast!(EstimateArgs);

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

impl figment::Provider for EstimateArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Call args provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let value = Value::serialize(self)?;
        let mut dict = value.into_dict().unwrap();

        let rpc_url = self.eth.rpc_url().map_err(|err| err.to_string())?;
        if rpc_url != "http://localhost:8545" {
            dict.insert("eth_rpc_url".to_string(), Value::from(rpc_url.to_string()));
        }

        if let Some(etherscan_api_key) = &self.eth.etherscan_key {
            dict.insert(
                "etherscan_api_key".to_string(),
                Value::from(etherscan_api_key.to_string()),
            );
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}
