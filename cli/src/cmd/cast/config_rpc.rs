use clap::Parser;
use eyre::Result;
use foundry_config::{
    figment::{
        self,
        error::Kind::InvalidType,
        value::{Dict, Map, Value},
        Figment, Metadata, Profile, Provider,
    },
    find_project_root_path, Config,
};

use serde::Serialize;

#[derive(Debug, Clone, Parser, Serialize)]
pub struct ConfigRPCArgs {
    rpc_url: Option<String>,
}

impl From<ConfigRPCArgs> for Config {
    fn from(args: ConfigRPCArgs) -> Self {
        let config = Config::figment_with_root(find_project_root_path().unwrap())
            .merge(args)
            .extract::<Config>()
            .unwrap();

        config
    }
}

impl figment::Provider for ConfigRPCArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("RPC url provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let value = Value::serialize(self)?;
        // let error = InvalidType(self.to_actual(), "map".into());
        let mut dict = value.into_dict().unwrap();

        // let rpc = self.eth.rpc_url().map_err(|err| err.to_string())?;

        if let Some(rpc_url) = &self.rpc_url {
            dict.insert("eth_rpc_url".to_string(), Value::from(rpc_url.to_string()));
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}
