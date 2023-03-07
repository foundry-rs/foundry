use super::{Wallet, WalletSigner};
use clap::{
    builder::{PossibleValuesParser, TypedValueParser},
    Parser,
};
use ethers::types::Chain as NamedChain;
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
use strum::VariantNames;

const FLASHBOTS_URL: &str = "https://rpc.flashbots.net";

#[allow(dead_code)]
static E: &str = "\
No Etherscan API Key is set. Consider using the ETHERSCAN_API_KEY environment variable,\n\
setting the -e CLI argument or etherscan-api-key in foundry.toml\
";

#[derive(Clone, Debug, Default, Parser)]
pub struct RpcOpts {
    /// The RPC endpoint
    #[clap(short = 'r', long = "rpc-url", env = "ETH_RPC_URL")]
    pub url: Option<String>,

    /// Use the Flashbots RPC URL (https://rpc.flashbots.net)
    #[clap(long, conflicts_with = "rpc_url")]
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
        let url = match (self.flashbots, self.url.as_deref(), config) {
            (true, ..) => Some(Cow::Borrowed(FLASHBOTS_URL)),
            (false, Some(url), _) => Some(Cow::Borrowed(url)),
            (false, None, Some(config)) => config.get_rpc_url().transpose()?,
            (false, None, None) => None,
        };
        Ok(url)
    }

    fn dict(&self) -> Dict {
        let mut dict = Dict::new();
        if let Ok(Some(url)) = self.url(None) {
            dict.insert("eth_rpc_url".into(), url.into_owned().into());
        }
        dict
    }
}

#[derive(Clone, Debug, Default, Parser, Serialize)]
pub struct EtherscanOpts {
    /// The Etherscan (or equivalent) API key
    #[clap(short = 'e', long = "etherscan-api-key", env = "ETHERSCAN_API_KEY")]
    #[serde(rename = "etherscan_api_key", skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,

    /// The chain name or EIP-155 chain ID
    #[clap(
        short,
        long,
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

    fn dict(&self) -> Dict {
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

/// The value parser for `Chain`s
#[derive(Clone, Debug)]
pub struct ChainValueParser {
    pub inner: PossibleValuesParser,
}

impl Default for ChainValueParser {
    fn default() -> Self {
        ChainValueParser { inner: NamedChain::VARIANTS.into() }
    }
}

impl TypedValueParser for ChainValueParser {
    type Value = Chain;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        self.inner.parse_ref(cmd, arg, value)?.parse::<Chain>().map_err(|_| {
            clap::Error::raw(
                clap::error::ErrorKind::InvalidValue,
                "chain argument did not match any possible chain variant",
            )
        })
    }
}
