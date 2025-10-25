//! Common RPC options shared between different CLI commands.

use clap::Parser;
use foundry_config::{
    Config,
    figment::{
        self, Metadata, Profile,
        value::{Dict, Map},
    },
};
use serde::Serialize;

/// Common RPC-related options that can be shared across different CLI commands.
#[derive(Clone, Debug, Default, Serialize, Parser)]
pub struct RpcCommonOpts {
    /// The RPC endpoint URL.
    #[arg(long, short, visible_alias = "rpc-url", value_name = "URL")]
    #[serde(rename = "eth_rpc_url", skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// Allow insecure RPC connections (accept invalid HTTPS certificates).
    #[arg(short = 'k', long = "insecure", default_value = "false")]
    pub accept_invalid_certs: bool,

    /// Timeout for the RPC request in seconds.
    #[arg(long, env = "ETH_RPC_TIMEOUT")]
    pub rpc_timeout: Option<u64>,
}

impl figment::Provider for RpcCommonOpts {
    fn metadata(&self) -> Metadata {
        Metadata::named("RpcCommonOpts")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        Ok(Map::from([(Config::selected_profile(), self.dict())]))
    }
}

impl RpcCommonOpts {
    /// Returns the RPC endpoint.
    pub fn url<'a>(
        &'a self,
        config: Option<&'a Config>,
    ) -> Result<Option<std::borrow::Cow<'a, str>>, eyre::Error> {
        let url = match (self.url.as_deref(), config) {
            (Some(url), _) => Some(std::borrow::Cow::Borrowed(url)),
            (None, Some(config)) => config.get_rpc_url().transpose()?,
            (None, None) => None,
        };
        Ok(url)
    }

    pub fn dict(&self) -> Dict {
        let mut dict = Dict::new();
        if let Ok(Some(url)) = self.url(None) {
            dict.insert("eth_rpc_url".into(), url.into_owned().into());
        }
        if let Some(rpc_timeout) = self.rpc_timeout {
            dict.insert("eth_rpc_timeout".into(), rpc_timeout.into());
        }
        if self.accept_invalid_certs {
            dict.insert("eth_rpc_accept_invalid_certs".into(), true.into());
        }
        dict
    }
}
