use crate::opts::ChainValueParser;
use alloy_chains::ChainKind;
use clap::Parser;
use eyre::Result;
use foundry_block_explorers::EtherscanApiVersion;
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

#[derive(Clone, Debug, Default, Parser)]
pub struct RpcOpts {
    /// The RPC endpoint, default value is http://localhost:8545.
    #[arg(short = 'r', long = "rpc-url", env = "ETH_RPC_URL")]
    pub url: Option<String>,

    /// Allow insecure RPC connections (accept invalid HTTPS certificates).
    ///
    /// When the provider's inner runtime transport variant is HTTP, this configures the reqwest
    /// client to accept invalid certificates.
    #[arg(short = 'k', long = "insecure", default_value = "false")]
    pub accept_invalid_certs: bool,

    /// Use the Flashbots RPC URL with fast mode (<https://rpc.flashbots.net/fast>).
    ///
    /// This shares the transaction privately with all registered builders.
    ///
    /// See: <https://docs.flashbots.net/flashbots-protect/quick-start#faster-transactions>
    #[arg(long)]
    pub flashbots: bool,

    /// JWT Secret for the RPC endpoint.
    ///
    /// The JWT secret will be used to create a JWT for a RPC. For example, the following can be
    /// used to simulate a CL `engine_forkchoiceUpdated` call:
    ///
    /// cast rpc --jwt-secret <JWT_SECRET> engine_forkchoiceUpdatedV2
    /// '["0x6bb38c26db65749ab6e472080a3d20a2f35776494e72016d1e339593f21c59bc",
    /// "0x6bb38c26db65749ab6e472080a3d20a2f35776494e72016d1e339593f21c59bc",
    /// "0x6bb38c26db65749ab6e472080a3d20a2f35776494e72016d1e339593f21c59bc"]'
    #[arg(long, env = "ETH_RPC_JWT_SECRET")]
    pub jwt_secret: Option<String>,

    /// Timeout for the RPC request in seconds.
    ///
    /// The specified timeout will be used to override the default timeout for RPC requests.
    ///
    /// Default value: 45
    #[arg(long, env = "ETH_RPC_TIMEOUT")]
    pub rpc_timeout: Option<u64>,

    /// Specify custom headers for RPC requests.
    #[arg(long, alias = "headers", env = "ETH_RPC_HEADERS", value_delimiter(','))]
    pub rpc_headers: Option<Vec<String>>,
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
        let url = match (self.flashbots, self.url.as_deref(), config) {
            (true, ..) => Some(Cow::Borrowed(FLASHBOTS_URL)),
            (false, Some(url), _) => Some(Cow::Borrowed(url)),
            (false, None, Some(config)) => config.get_rpc_url().transpose()?,
            (false, None, None) => None,
        };
        Ok(url)
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

    /// The Etherscan API version.
    #[arg(
        short,
        long = "etherscan-api-version",
        alias = "api-version",
        env = "ETHERSCAN_API_VERSION"
    )]
    #[serde(rename = "etherscan_api_version", skip_serializing_if = "Option::is_none")]
    pub api_version: Option<EtherscanApiVersion>,

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

        if let Some(api_version) = &self.api_version {
            dict.insert("etherscan_api_version".into(), api_version.to_string().into());
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
