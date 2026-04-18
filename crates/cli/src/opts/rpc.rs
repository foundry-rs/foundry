use crate::opts::{ChainValueParser, RpcCommonOpts};
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

#[derive(Clone, Debug, Default, Parser)]
#[command(next_help_heading = "Rpc options")]
pub struct RpcOpts {
    /// Common RPC options (URL, timeout, rate limiting, etc.).
    #[command(flatten)]
    pub common: RpcCommonOpts,

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

    /// Specify custom headers for RPC requests.
    #[arg(long, alias = "headers", env = "ETH_RPC_HEADERS", value_delimiter(','))]
    pub rpc_headers: Option<Vec<String>>,

    /// Print the equivalent curl command instead of making the RPC request.
    #[arg(long)]
    pub curl: bool,
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
        if self.flashbots {
            dict.insert("eth_rpc_url".into(), FLASHBOTS_URL.into());
        }
        if let Ok(Some(jwt)) = self.jwt(None) {
            dict.insert("eth_rpc_jwt".into(), jwt.into_owned().into());
        }
        if let Some(headers) = &self.rpc_headers {
            dict.insert("eth_rpc_headers".into(), headers.clone().into());
        }
        if self.curl {
            dict.insert("eth_rpc_curl".into(), true.into());
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
            dict.insert("chain_id".into(), chain.id().into());
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

    // <https://github.com/foundry-rs/foundry/issues/14314>
    #[test]
    fn named_chain_dict_inserts_numeric_id() {
        // Chain 9745 is recognized as NamedChain::Plasma by alloy-chains.
        // Previously, dict() would insert chain_id as the string "plasma",
        // causing deserialization failure when EvmOpts expects u64.
        let args = EtherscanOpts::parse_from(["foundry-cli", "--chain", "9745"]);
        let dict = args.dict();
        let chain_id = dict.get("chain_id").expect("chain_id should be present");
        let id: u64 = chain_id.deserialize().expect("chain_id should deserialize as u64");
        assert_eq!(id, 9745);
    }
}
