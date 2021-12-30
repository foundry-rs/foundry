pub mod cast;
pub mod forge;

use std::{convert::TryFrom, str::FromStr};

use ethers::{
    middleware::SignerMiddleware,
    providers::{Http, Provider},
    signers::{
        coins_bip39::English, HDPath as LedgerHDPath, Ledger, LocalWallet, MnemonicBuilder, Signer,
        Trezor, TrezorHDPath,
    },
    types::{Address, U256},
};
use eyre::Result;
use structopt::StructOpt;

#[derive(StructOpt, Debug, Clone)]
pub struct EthereumOpts {
    #[structopt(env = "ETH_RPC_URL", long = "rpc-url", help = "The tracing / archival node's URL")]
    pub rpc_url: String,

    #[structopt(env = "ETH_FROM", short, long = "from", help = "The sender account")]
    pub from: Option<Address>,

    #[structopt(flatten)]
    pub wallet: Wallet,
}

impl EthereumOpts {
    #[allow(unused)]
    pub async fn signer(&self, chain_id: U256) -> eyre::Result<Option<WalletType>> {
        self.signer_with(chain_id, Provider::try_from(self.rpc_url.as_str())?).await
    }

    /// Returns a [`SignerMiddleware`] corresponding to the provided private key, mnemonic or hw
    /// signer
    pub async fn signer_with(
        &self,
        chain_id: U256,
        provider: Provider<Http>,
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
                .or_else(|| self.wallet.mnemonic().transpose())
                .or_else(|| self.wallet.keystore().transpose())
                .transpose()?
                .ok_or_else(|| eyre::eyre!("error accessing local wallet"))?;

            let local = local.with_chain_id(chain_id.as_u64());

            Ok(Some(WalletType::Local(SignerMiddleware::new(provider, local))))
        }
    }
}

#[derive(Debug)]
pub enum WalletType {
    Local(SignerMiddleware<Provider<Http>, LocalWallet>),
    Ledger(SignerMiddleware<Provider<Http>, Ledger>),
    Trezor(SignerMiddleware<Provider<Http>, Trezor>),
}

#[derive(StructOpt, Debug, Clone)]
pub struct Wallet {
    #[structopt(long = "private-key", help = "Your private key string")]
    pub private_key: Option<String>,

    #[structopt(long = "keystore", help = "Path to your keystore folder / file")]
    pub keystore_path: Option<String>,

    #[structopt(long = "password", help = "Your keystore password", requires = "keystore-path")]
    pub keystore_password: Option<String>,

    #[structopt(long = "mnemonic-path", help = "Path to your mnemonic file")]
    pub mnemonic_path: Option<String>,

    #[structopt(short, long = "ledger", help = "Use your Ledger hardware wallet")]
    pub ledger: bool,

    #[structopt(short, long = "trezor", help = "Use your Trezor hardware wallet")]
    pub trezor: bool,

    #[structopt(
        long = "hd-path",
        help = "Derivation path for your hardware wallet (trezor or ledger)"
    )]
    pub hd_path: Option<String>,

    #[structopt(
        long = "mnemonic_index",
        help = "your index in the standard hd path",
        default_value = "0"
    )]
    pub mnemonic_index: u32,
}

impl Wallet {
    fn private_key(&self) -> Result<Option<LocalWallet>> {
        Ok(if let Some(ref private_key) = self.private_key {
            Some(LocalWallet::from_str(private_key)?)
        } else {
            None
        })
    }

    fn keystore(&self) -> Result<Option<LocalWallet>> {
        Ok(match (&self.keystore_path, &self.keystore_password) {
            (Some(path), Some(password)) => Some(LocalWallet::decrypt_keystore(path, password)?),
            (Some(path), None) => {
                println!("Insert keystore password:");
                let password = rpassword::read_password().unwrap();
                Some(LocalWallet::decrypt_keystore(path, password)?)
            }
            (None, _) => None,
        })
    }

    fn mnemonic(&self) -> Result<Option<LocalWallet>> {
        Ok(if let Some(ref path) = self.mnemonic_path {
            let mnemonic = std::fs::read_to_string(path)?.replace('\n', "");
            Some(
                MnemonicBuilder::<English>::default()
                    .phrase(mnemonic.as_str())
                    .index(self.mnemonic_index)?
                    .build()?,
            )
        } else {
            None
        })
    }
}
