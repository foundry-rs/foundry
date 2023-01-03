use super::{Wallet, WalletType};
use cast::{AwsChainProvider, AwsClient, AwsHttpClient, AwsRegion, KmsClient};
use clap::Parser;
use ethers::{
    middleware::SignerMiddleware,
    signers::{AwsSigner, HDPath as LedgerHDPath, Ledger, Signer, Trezor, TrezorHDPath},
    types::{Address, U256},
};
use eyre::Result;
use foundry_common::{ProviderBuilder, RetryProvider};
use foundry_config::{
    figment::{
        self,
        value::{Dict, Map, Value},
        Metadata, Profile,
    },
    impl_figment_convert_cast, Chain, Config,
};
use serde::Serialize;
use std::sync::Arc;

const FLASHBOTS_URL: &str = "https://rpc.flashbots.net";

impl_figment_convert_cast!(EthereumOpts);

#[derive(Debug, Clone, Default, Parser, Serialize)]
#[clap(next_help_heading = "Ethereum options")]
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

    #[clap(flatten)]
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
                WalletType::Aws(signer) => signer.address(),
            }
        } else {
            self.wallet.from.unwrap_or_else(Address::zero)
        }
    }

    #[allow(unused)]
    pub async fn signer(&self, chain_id: U256) -> eyre::Result<Option<WalletType>> {
        self.signer_with(
            chain_id,
            Arc::new(
                ProviderBuilder::new(self.rpc_url()?)
                    .chain(chain_id)
                    .initial_backoff(1000)
                    .connect()
                    .await?,
            ),
        )
        .await
    }

    /// Returns a [`SignerMiddleware`] corresponding to the provided private key, mnemonic or hw
    /// signer
    pub async fn signer_with(
        &self,
        chain_id: U256,
        provider: Arc<RetryProvider>,
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
        } else if self.wallet.aws {
            let client =
                AwsClient::new_with(AwsChainProvider::default(), AwsHttpClient::new().unwrap());

            let kms = KmsClient::new_with_client(client, AwsRegion::default());

            let key_id = std::env::var("AWS_KMS_KEY_ID")?;

            let aws_signer = AwsSigner::new(kms, key_id, chain_id.as_u64()).await?;

            Ok(Some(WalletType::Aws(SignerMiddleware::new(provider, aws_signer))))
        } else {
            let local = self
                .wallet
                .private_key()
                .transpose()
                .or_else(|| self.wallet.interactive().transpose())
                .or_else(|| self.wallet.mnemonic().transpose())
                .or_else(|| self.wallet.keystore().transpose())
                .transpose()?
                .ok_or_else(|| eyre::eyre!("error accessing local wallet, did you set a private key, mnemonic or keystore? Run `cast send --help` or `forge create --help` and use the corresponding CLI flag to set your key via --private-key, --mnemonic-path, --aws, --interactive, --trezor or --ledger. Alternatively, if you're using a local node with unlocked accounts, use the --unlocked flag and set the `ETH_FROM` environment variable to the address of the unlocked account you want to use"))?;

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
            dict.insert("sender".to_string(), format!("{from:?}").into());
        }

        if let Some(etherscan_api_key) = &self.etherscan_api_key {
            dict.insert("etherscan_api_key".to_string(), etherscan_api_key.to_string().into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}
