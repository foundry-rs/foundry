use super::{ChainValueParser, Wallet, WalletSigner};
use clap::Parser;
use eyre::Result;
use foundry_config::{
    figment::{
        self,
        value::{Dict, Map, Value},
        Metadata, Profile,
    },
    impl_figment_convert_cast, Chain, Config,
};
use serde::Serialize;
use std::borrow::Cow;

const FLASHBOTS_URL: &str = "https://rpc.flashbots.net";

#[derive(Clone, Debug, Default, Parser)]
pub struct RpcOpts {
    /// The RPC endpoint.
    #[clap(short = 'r', long = "rpc-url", env = "ETH_RPC_URL")]
    pub url: Option<String>,

    /// Use the Flashbots RPC URL (https://rpc.flashbots.net).
    #[clap(long)]
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
    #[clap(long, env = "ETH_RPC_JWT_SECRET")]
    pub jwt_secret: Option<String>,
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
        dict
    }
}

#[derive(Clone, Debug, Default, Parser, Serialize)]
pub struct EtherscanOpts {
    /// The Etherscan (or equivalent) API key
    #[clap(short = 'e', long = "etherscan-api-key", alias = "api-key", env = "ETHERSCAN_API_KEY")]
    #[serde(rename = "etherscan_api_key", skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,

    /// The chain name or EIP-155 chain ID
    #[clap(
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
    pub fn key<'a>(&'a self, config: Option<&'a Config>) -> Option<Cow<'a, str>> {
        match (self.key.as_deref(), config) {
            (Some(key), _) => Some(Cow::Borrowed(key)),
            (None, Some(config)) => config.get_etherscan_api_key(self.chain).map(Cow::Owned),
            (None, None) => None,
        }
    }

    pub fn dict(&self) -> Dict {
        Value::serialize(self).unwrap().into_dict().unwrap()
    }
}

#[derive(Clone, Debug, Default, Parser)]
#[clap(next_help_heading = "Ethereum options")]
pub struct EthereumOpts {
    #[clap(flatten)]
    pub rpc: RpcOpts,

    #[clap(flatten)]
    pub etherscan: EtherscanOpts,

    #[clap(flatten)]
    pub wallet: Wallet,
}

impl_figment_convert_cast!(EthereumOpts);

impl EthereumOpts {
    pub async fn signer(&self) -> Result<WalletSigner> {
        self.wallet.signer(self.etherscan.chain.unwrap_or_default().id()).await
    }
}

// Make this args a `Figment` so that it can be merged into the `Config`
impl figment::Provider for EthereumOpts {
    fn metadata(&self) -> Metadata {
        Metadata::named("Ethereum Opts Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut dict = self.etherscan.dict();
        dict.extend(self.rpc.dict());

        if let Some(from) = self.wallet.from {
            dict.insert("sender".to_string(), format!("{from:?}").into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}
