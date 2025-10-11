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

#[derive(Clone, Debug, Default, Parser)]
pub struct RpcOpts {
    #[command(flatten)]
    pub common: RpcCommonOpts,

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
        let url = match (self.flashbots, self.common.url.as_deref(), config) {
            (true, ..) => Some(Cow::Borrowed(FLASHBOTS_URL)),
            (false, Some(url), _) => Some(Cow::Borrowed(url)),
            (false, None, Some(config)) => config.get_rpc_url().transpose()?,
            (false, None, None) => None,
        };
        Ok(url)
    }

    /// Returns the JWT secret.
    pub fn jwt<'a>(&'a self, config: Option<&'a Config>) -> Result<Option<Cow<'a, str>>> {
        self.common.jwt(config)
    }

    pub fn dict(&self) -> Dict {
        let dict = self.common.dict();
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
