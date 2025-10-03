//! Common RPC options shared between different CLI commands.

use clap::Parser;
use foundry_config::{
    figment::{
        self, Metadata, Profile,
        value::{Dict, Map},
    },
    Config,
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

    /// JWT Secret for the RPC endpoint.
    #[arg(long, env = "ETH_RPC_JWT_SECRET")]
    pub jwt_secret: Option<String>,

    /// Timeout for the RPC request in seconds.
    #[arg(long, env = "ETH_RPC_TIMEOUT")]
    pub rpc_timeout: Option<u64>,

    /// Specify custom headers for RPC requests.
    #[arg(long, alias = "headers", env = "ETH_RPC_HEADERS", value_delimiter(','))]
    pub rpc_headers: Option<Vec<String>>,

    /// Sets the number of assumed available compute units per second for this provider.
    #[arg(long, alias = "cups", value_name = "CUPS")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compute_units_per_second: Option<u64>,

    /// Disables rate limiting for this node's provider.
    #[arg(long, value_name = "NO_RATE_LIMITS", visible_alias = "no-rate-limit")]
    #[serde(skip)]
    pub no_rpc_rate_limit: bool,
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
    pub fn url<'a>(&'a self, config: Option<&'a Config>) -> Result<Option<std::borrow::Cow<'a, str>>, eyre::Error> {
        let url = match (self.url.as_deref(), config) {
            (Some(url), _) => Some(std::borrow::Cow::Borrowed(url)),
            (None, Some(config)) => config.get_rpc_url().transpose()?,
            (None, None) => None,
        };
        Ok(url)
    }

    /// Returns the JWT secret.
    pub fn jwt<'a>(&'a self, config: Option<&'a Config>) -> Result<Option<std::borrow::Cow<'a, str>>, eyre::Error> {
        let jwt = match (self.jwt_secret.as_deref(), config) {
            (Some(jwt), _) => Some(std::borrow::Cow::Borrowed(jwt)),
            (None, Some(config)) => config.get_rpc_jwt_secret()?,
            (None, None) => None,
        };
        Ok(jwt)
    }

    pub fn dict(&self) -> Dict {
        let mut dict = Dict::new();
        if let Ok(Some(url)) = self.url(None) {
            dict.insert("eth_rpc_url".into(), url.into_owned().into());
        }
        if let Ok(Some(jwt)) = self.jwt(None) {
            dict.insert("eth_rpc_jwt".into(), jwt.into_owned().into());
        }
        if let Some(rpc_timeout) = self.rpc_timeout {
            dict.insert("eth_rpc_timeout".into(), rpc_timeout.into());
        }
        if let Some(headers) = &self.rpc_headers {
            dict.insert("eth_rpc_headers".into(), headers.clone().into());
        }
        if self.accept_invalid_certs {
            dict.insert("eth_rpc_accept_invalid_certs".into(), true.into());
        }
        if let Some(cups) = self.compute_units_per_second {
            dict.insert("compute_units_per_second".into(), cups.into());
        }
        if self.no_rpc_rate_limit {
            dict.insert("no_rpc_rate_limit".into(), self.no_rpc_rate_limit.into());
        }
        dict
    }
}
