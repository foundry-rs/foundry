use ethers::{
    providers::{Http, Provider},
    signers::{coins_bip39::English, LocalWallet, MnemonicBuilder},
    types::Address,
};
use eyre::Result;
use std::convert::TryFrom;
use std::str::FromStr;
use structopt::StructOpt;

#[derive(StructOpt, Debug, Clone)]
pub struct EthereumOpts {
    #[structopt(
        env = "ETH_RPC_URL",
        short,
        long = "rpc-url",
        help = "The tracing / archival node's URL"
    )]
    pub rpc_url: String,

    #[structopt(env = "ETH_FROM", short, long = "from", help = "The sender account")]
    pub from: Option<Address>,

    #[structopt(long, env = "SETH_ASYNC")]
    pub seth_async: bool,

    #[structopt(flatten)]
    pub wallet: Wallet,
}

// TODO: Improve these so that we return a middleware trait object
use std::sync::Arc;
impl EthereumOpts {
    #[allow(unused)]
    pub fn provider(&self) -> eyre::Result<Arc<Provider<Http>>> {
        Ok(Arc::new(Provider::try_from(self.rpc_url.as_str())?))
    }

    /// Returns a [`LocalWallet`] corresponding to the provided private key or mnemonic
    pub fn signer(&self) -> eyre::Result<Option<LocalWallet>> {
        self.wallet.signer()
    }
}

#[derive(StructOpt, Debug, Clone)]
pub struct Wallet {
    #[structopt(long = "private_key", help = "Your private key string")]
    pub private_key: Option<String>,

    #[structopt(long = "keystore", help = "Path to your keystore folder / file")]
    pub keystore_path: Option<String>,

    #[structopt(
        long = "password",
        help = "Your keystore password",
        requires = "keystore_path"
    )]
    pub keystore_password: Option<String>,

    #[structopt(long = "mnemonic_path", help = "Path to your mnemonic file")]
    pub mnemonic_path: Option<String>,

    #[structopt(
        long = "mnemonic_index",
        help = "your index in the standard hd path",
        default_value = "0",
        requires = "mnemonic_path"
    )]
    pub mnemonic_index: u32,
}

impl Wallet {
    #[allow(clippy::manual_map)]
    pub fn signer(&self) -> Result<Option<LocalWallet>> {
        Ok(if let Some(wallet) = self.private_key()? {
            Some(wallet)
        } else if let Some(wallet) = self.mnemonic()? {
            Some(wallet)
        } else if let Some(wallet) = self.keystore()? {
            Some(wallet)
        } else {
            None
        })
    }

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
            let mnemonic = std::fs::read_to_string(path)?.replace("\n", "");
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
