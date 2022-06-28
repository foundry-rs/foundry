pub mod cast;
pub mod forge;

mod multi_wallet;
mod wallet;

use std::sync::Arc;

pub use multi_wallet::*;
pub use wallet::*;

use clap::Parser;
use ethers::{
    middleware::SignerMiddleware,
    prelude::RetryClient,
    providers::{Http, Provider},
    signers::{HDPath as LedgerHDPath, Ledger, Signer, Trezor, TrezorHDPath},
    types::{Address, Chain, U256},
};
use eyre::Result;
use foundry_config::{
    figment::{
        self,
        value::{Dict, Map, Value},
        Metadata, Profile,
    },
    impl_figment_convert_cast, Config,
};

use serde::Serialize;
use strum::VariantNames;

const FLASHBOTS_URL: &str = "https://rpc.flashbots.net";

// Helper for exposing enum values for `Chain`
// TODO: Is this a duplicate of config/src/chain.rs?
#[derive(Debug, Clone, Parser)]
pub struct ClapChain {
    #[clap(
        short = 'c',
        long = "chain",
        env = "CHAIN",
        default_value = "mainnet",
        // if Chain implemented ArgEnum, we'd get this for free
        possible_values = Chain::VARIANTS,
        value_name = "CHAIN"
    )]
    pub inner: Chain,
}

impl_figment_convert_cast!(EthereumOpts);
#[derive(Parser, Debug, Clone, Serialize)]
pub struct EthereumOpts {
    #[clap(env = "ETH_RPC_URL", long = "rpc-url", help = "The RPC endpoint.", value_name = "URL")]
    pub rpc_url: Option<String>,

    #[clap(long, help = "Use the flashbots RPC URL (https://rpc.flashbots.net)")]
    pub flashbots: bool,

    #[clap(long, env = "ETHERSCAN_API_KEY", value_name = "KEY")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etherscan_api_key: Option<String>,

    #[clap(long, env = "CHAIN", value_name = "CHAIN_NAME")]
    #[serde(skip)]
    pub chain: Option<Chain>,

    #[clap(flatten, next_help_heading = "WALLET OPTIONS")]
    #[serde(skip)]
    pub wallet: Wallet,
}

impl EthereumOpts {
    /// Returns the sender address of the signer or `from`
    #[allow(unused)]
    pub async fn sender(&self) -> Address {
        if let Ok(Some(signer)) = self.signer(0.into()).await {
            match signer {
                WalletType::Ledger(signer) => signer.address(),
                WalletType::Local(signer) => signer.address(),
                WalletType::Trezor(signer) => signer.address(),
            }
        } else {
            self.wallet.from.unwrap_or_else(Address::zero)
        }
    }

    #[allow(unused)]
    pub async fn signer(&self, chain_id: U256) -> eyre::Result<Option<WalletType>> {
        self.signer_with(
            chain_id,
            Arc::new(Provider::<RetryClient<Http>>::new_client(self.rpc_url()?, 10, 1000)?),
        )
        .await
    }

    /// Returns a [`SignerMiddleware`] corresponding to the provided private key, mnemonic or hw
    /// signer
    pub async fn signer_with(
        &self,
        chain_id: U256,
        provider: Arc<Provider<RetryClient<Http>>>,
    ) -> eyre::Result<Option<WalletType>> {
        if self.wallet.ledger {
            let derivation = match &self.wallet.hd_path {
                Some(hd_path) => LedgerHDPath::Other(hd_path.clone()),
                None => LedgerHDPath::LedgerLive(self.wallet.mnemonic_index as usize),
            };
            let ledger = Ledger::new(derivation, chain_id.as_u64()).await?;

            Ok(Some(WalletType::Ledger(SignerMiddleware::new(provider, ledger))))
        } else if self.wallet.trezor {
            let derivation = match &self.wallet.hd_path {
                Some(hd_path) => TrezorHDPath::Other(hd_path.clone()),
                None => TrezorHDPath::TrezorLive(self.wallet.mnemonic_index as usize),
            };

            // cached to ~/.ethers-rs/trezor/cache/trezor.session
            let trezor = Trezor::new(derivation, chain_id.as_u64(), None).await?;

            Ok(Some(WalletType::Trezor(SignerMiddleware::new(provider, trezor))))
        } else {
            let local = self
                .wallet
                .private_key()
                .transpose()
                .or_else(|| self.wallet.interactive().transpose())
                .or_else(|| self.wallet.mnemonic().transpose())
                .or_else(|| self.wallet.keystore().transpose())
                .transpose()?
                .ok_or_else(|| eyre::eyre!("error accessing local wallet, did you set a private key, mnemonic or keystore? Run `cast send --help` or `forge create --help` and use the corresponding CLI flag to set your key via --private-key, --mnemonic-path, --interactive, --trezor or --ledger. Alternatively, if you're using a local node with unlocked accounts, set the `ETH_FROM` environment variable to the address of the account you want to use"))?;

            let local = local.with_chain_id(chain_id.as_u64());

            Ok(Some(WalletType::Local(SignerMiddleware::new(provider, local))))
        }
    }

    pub fn rpc_url(&self) -> Result<&str> {
        if self.flashbots {
            Ok(FLASHBOTS_URL)
        } else {
            Ok(self.rpc_url.as_deref().unwrap_or("http://localhost:8545"))
        }
    }
}

// Make this args a `Figment` so that it can be merged into the `Config`
impl figment::Provider for EthereumOpts {
    fn metadata(&self) -> Metadata {
        Metadata::named("Ethereum Opts Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let value = Value::serialize(self)?;
        let mut dict = value.into_dict().unwrap();

        let rpc_url = self.rpc_url().map_err(|err| err.to_string())?;
        if rpc_url != "http://localhost:8545" {
            dict.insert("eth_rpc_url".to_string(), rpc_url.to_string().into());
        }

        if let Some(from) = self.wallet.from {
            dict.insert("sender".to_string(), format!("{:?}", from).into());
        }

        if let Some(etherscan_api_key) = &self.etherscan_api_key {
            dict.insert("etherscan_api_key".to_string(), etherscan_api_key.to_string().into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn illformed_private_key_generates_user_friendly_error() {
        let wallet = Wallet {
            from: None,
            interactive: false,
            private_key: Some("123".to_string()),
            keystore_path: None,
            keystore_password: None,
            mnemonic_path: None,
            ledger: false,
            trezor: false,
            hd_path: None,
            mnemonic_index: 0,
        };
        match wallet.private_key() {
            Ok(_) => {
                panic!("illformed private key shouldn't decode")
            }
            Err(x) => {
                assert!(
                    x.to_string().contains("Failed to create wallet"),
                    "Error message is not user-friendly"
                );
            }
        }
    }
}
