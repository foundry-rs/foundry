//! Common RPC options shared between `RpcOpts` and `EvmArgs`.

use clap::Parser;
use eyre::Result;
use foundry_config::{
    Config,
    figment::{
        self, Metadata, Profile,
        value::{Dict, Map},
    },
};
use serde::Serialize;
use std::borrow::Cow;

/// Common RPC-related options shared across CLI commands.
///
/// This struct holds fields that both [`super::RpcOpts`] (cast) and
/// [`super::EvmArgs`] (forge/script) need, eliminating duplication and
/// making the two structs composable.
///
/// Note: `ETH_RPC_URL` is intentionally **not** bound here as a clap env
/// fallback; otherwise it would be inherited by `EvmArgs` and silently
/// fork all `forge test` runs. Cast resolves `ETH_RPC_URL` explicitly
/// at the call site (see [`super::RpcOpts::url`]).
#[derive(Clone, Debug, Default, Serialize, Parser)]
pub struct RpcCommonOpts {
    /// The RPC endpoint.
    #[arg(short, long, visible_alias = "fork-url", value_name = "URL")]
    #[serde(rename = "eth_rpc_url", skip_serializing_if = "Option::is_none")]
    pub rpc_url: Option<String>,

    /// Allow insecure RPC connections (accept invalid HTTPS certificates).
    ///
    /// When the provider's inner runtime transport variant is HTTP, this configures the reqwest
    /// client to accept invalid certificates.
    #[arg(short = 'k', long = "insecure", default_value = "false")]
    #[serde(skip)]
    pub accept_invalid_certs: bool,

    /// Timeout for the RPC request in seconds.
    ///
    /// The specified timeout will be used to override the default timeout for RPC requests.
    ///
    /// Default value: 45
    #[arg(long, env = "ETH_RPC_TIMEOUT")]
    #[serde(rename = "eth_rpc_timeout", skip_serializing_if = "Option::is_none")]
    pub rpc_timeout: Option<u64>,

    /// Disable automatic proxy detection.
    ///
    /// Use this in sandboxed environments (e.g., Cursor IDE sandbox, macOS App Sandbox) where
    /// system proxy detection causes crashes. When enabled, HTTP_PROXY/HTTPS_PROXY environment
    /// variables and system proxy settings will be ignored.
    #[arg(long = "no-proxy", alias = "disable-proxy", default_value = "false")]
    #[serde(skip)]
    pub no_proxy: bool,

    /// Sets the number of assumed available compute units per second for this provider.
    ///
    /// default value: 330
    ///
    /// See also <https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second>
    #[arg(long, alias = "cups", value_name = "CUPS")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compute_units_per_second: Option<u64>,

    /// Disables rate limiting for this node's provider.
    ///
    /// See also <https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second>
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
    /// Returns the RPC endpoint URL, resolving from CLI args or config.
    pub fn url<'a>(&'a self, config: Option<&'a Config>) -> Result<Option<Cow<'a, str>>> {
        let url = match (self.rpc_url.as_deref(), config) {
            (Some(url), _) => Some(Cow::Borrowed(url)),
            (None, Some(config)) => config.get_rpc_url().transpose()?,
            (None, None) => None,
        };
        Ok(url)
    }

    /// Builds a figment-compatible dictionary from these options.
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
        if self.no_proxy {
            dict.insert("eth_rpc_no_proxy".into(), true.into());
        }
        if let Some(cups) = self.compute_units_per_second {
            dict.insert("compute_units_per_second".into(), cups.into());
        }
        if self.no_rpc_rate_limit {
            dict.insert("no_rpc_rate_limit".into(), true.into());
        }
        dict
    }
}
