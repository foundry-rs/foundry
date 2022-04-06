use crate::{
    cmd::Cmd,
    opts::{
        cast::{parse_block_id, parse_name_or_address},
        EthereumOpts,
    },
};
use cast::Cast;
use clap::Parser;
use ethers::types::{BlockId, NameOrAddress};
use eyre::Result;
use foundry_config::{
    figment::{
        self,
        error::Kind::InvalidType,
        value::{Dict, Map, Value},
        Figment, Metadata, Profile, Provider,
    },
    find_project_root_path, remappings_from_env_var, Config,
};

use serde::Serialize;

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

impl<'a> From<&'a CallArgs> for Config {
    fn from(args: &'a CallArgs) -> Self {
        let config = Config::figment_with_root(find_project_root_path().unwrap())
            .merge(args)
            .extract::<Config>()
            .unwrap();

        config
    }
}

impl figment::Provider for CallArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("RPC url provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let value = Value::serialize(self)?;
        // let error = InvalidType(self.to_actual(), "map".into());
        let mut dict = value.into_dict().unwrap();

        // let rpc = self.eth.rpc_url().map_err(|err| err.to_string())?;
        if let Some(rpc_url) = &self.eth.rpc_url {
            dict.insert("eth_rpc_url".to_string(), Value::from(rpc_url.to_string()));
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}
