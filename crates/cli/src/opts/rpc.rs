use crate::opts::{ChainValueParser, RpcCommonOpts};
use alloy_chains::ChainKind;
use clap::Parser;
use eyre::Result;
use foundry_config::{
    Chain, Config, FigmentProviders,
    figment::{
        self, Figment, Metadata, Profile,
        value::{Dict, Map},
    },
    find_project_root, impl_figment_convert_cast,
};
use foundry_wallets::WalletOpts;
use serde::Serialize;
use std::borrow::Cow;

const FLASHBOTS_URL: &str = "https://rpc.flashbots.net/fast";

#[derive(Clone, Debug, Default, Serialize, Parser)]
pub struct RpcOpts {
    #[command(flatten)]
    pub common: RpcCommonOpts,

    /// JWT Secret for the RPC endpoint.
    #[arg(long, env = "ETH_RPC_JWT_SECRET")]
    pub jwt_secret: Option<String>,

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

    /// Use the Flashbots RPC URL with fast mode (<https://rpc.flashbots.net/fast>).
    ///
    /// This shares the transaction privately with all registered builders.
    ///
    /// See: <https://docs.flashbots.net/flashbots-protect/quick-start#faster-transactions>
    #[arg(long)]
    pub flashbots: bool,
}

impl_figment_convert_cast!(RpcOpts);

impl figment::Provider for RpcOpts {
    fn metadata(&self) -> Metadata {
        Metadata::named("RpcOpts")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        Ok(Map::from([(Config::selected_profile(), self.dict())]))
    }
}

impl RpcOpts {
    /// Returns the RPC endpoint.
    pub fn url<'a>(&'a self, config: Option<&'a Config>) -> Result<Option<Cow<'a, str>>> {
        if self.flashbots {
            Ok(Some(Cow::Borrowed(FLASHBOTS_URL)))
        } else {
            self.common.url(config)
        }
    }

    /// Returns the JWT secret.
    pub fn jwt<'a>(&'a self, config: Option<&'a Config>) -> Result<Option<Cow<'a, str>>> {
        let jwt = match (self.jwt_secret.as_deref(), config) {
            (Some(jwt), _) => Some(Cow::Borrowed(jwt)),
            (None, Some(config)) => config.get_rpc_jwt_secret()?,
            (None, None) => None,
        };
        Ok(jwt)
    }

    pub fn dict(&self) -> Dict {
        let mut dict = self.common.dict();

        if let Ok(Some(jwt)) = self.jwt(None) {
            dict.insert("eth_rpc_jwt".into(), jwt.into_owned().into());
        }
        if let Some(headers) = &self.rpc_headers {
            dict.insert("eth_rpc_headers".into(), headers.clone().into());
        }
        if let Some(cups) = self.compute_units_per_second {
            dict.insert("compute_units_per_second".into(), cups.into());
        }
        if self.no_rpc_rate_limit {
            dict.insert("no_rpc_rate_limit".into(), self.no_rpc_rate_limit.into());
        }

        // Flashbots URL is handled in the url() method, not in dict()
        dict
    }

    pub fn into_figment(self, all: bool) -> Figment {
        let root = find_project_root(None).expect("could not determine project root");
        Config::with_root(&root)
            .to_figment(if all { FigmentProviders::All } else { FigmentProviders::Cast })
            .merge(self)
    }
}

#[derive(Clone, Debug, Default, Serialize, Parser)]
pub struct EtherscanOpts {
    /// The Etherscan (or equivalent) API key.
    #[arg(short = 'e', long = "etherscan-api-key", alias = "api-key", env = "ETHERSCAN_API_KEY")]
    #[serde(rename = "etherscan_api_key", skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,

    /// The chain name or EIP-155 chain ID.
    #[arg(
        short,
        long,
        alias = "chain-id",
        env = "CHAIN",
        value_parser = ChainValueParser::default(),
    )]
    #[serde(rename = "chain_id", skip_serializing_if = "Option::is_none")]
    pub chain: Option<Chain>,
}

impl_figment_convert_cast!(EtherscanOpts);

impl figment::Provider for EtherscanOpts {
    fn metadata(&self) -> Metadata {
        Metadata::named("EtherscanOpts")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        Ok(Map::from([(Config::selected_profile(), self.dict())]))
    }
}

impl EtherscanOpts {
    /// Returns true if the Etherscan API key is set.
    pub fn has_key(&self) -> bool {
        self.key.as_ref().filter(|key| !key.trim().is_empty()).is_some()
    }

    /// Returns the Etherscan API key.
    pub fn key(&self) -> Option<String> {
        self.key.as_ref().filter(|key| !key.trim().is_empty()).cloned()
    }

    pub fn dict(&self) -> Dict {
        let mut dict = Dict::new();
        if let Some(key) = self.key() {
            dict.insert("etherscan_api_key".into(), key.into());
        }

        if let Some(chain) = self.chain {
            if let ChainKind::Id(id) = chain.kind() {
                dict.insert("chain_id".into(), (*id).into());
            } else {
                dict.insert("chain_id".into(), chain.to_string().into());
            }
        }
        dict
    }
}

#[derive(Clone, Debug, Default, Parser)]
#[command(next_help_heading = "Ethereum options")]
pub struct EthereumOpts {
    #[command(flatten)]
    pub rpc: RpcOpts,

    #[command(flatten)]
    pub etherscan: EtherscanOpts,

    #[command(flatten)]
    pub wallet: WalletOpts,
}

impl_figment_convert_cast!(EthereumOpts);

// Make this args a `Figment` so that it can be merged into the `Config`
impl figment::Provider for EthereumOpts {
    fn metadata(&self) -> Metadata {
        Metadata::named("Ethereum Opts Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut dict = self.etherscan.dict();
        dict.extend(self.rpc.dict());

        if let Some(from) = self.wallet.from {
            dict.insert("sender".to_string(), from.to_string().into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_etherscan_opts() {
        let args: EtherscanOpts =
            EtherscanOpts::parse_from(["foundry-cli", "--etherscan-api-key", "dummykey"]);
        assert_eq!(args.key(), Some("dummykey".to_string()));

        let args: EtherscanOpts =
            EtherscanOpts::parse_from(["foundry-cli", "--etherscan-api-key", ""]);
        assert!(!args.has_key());
    }
}
