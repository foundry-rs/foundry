pub mod cast;
pub mod evm;
pub mod forge;

use std::{convert::TryFrom, str::FromStr};

use clap::Parser;
use ethers::{
    middleware::SignerMiddleware,
    providers::{Http, Provider},
    signers::{
        coins_bip39::English, HDPath as LedgerHDPath, Ledger, LocalWallet, MnemonicBuilder, Signer,
        Trezor, TrezorHDPath,
    },
    types::{Address, Chain, U256},
};
use eyre::Result;
use foundry_config::{
    figment::{
        self,
        value::{Dict, Map, Value},
        Metadata, Profile,
    },
    Config,
};

const FLASHBOTS_URL: &str = "https://rpc.flashbots.net";

// Helper for exposing enum values for `Chain`
#[derive(Debug, Clone, Parser)]
pub struct ClapChain {
    #[clap(
        short = 'c',
        long = "chain",
        env = "CHAIN",
        default_value = "mainnet",
        // if Chain implemented ArgEnum, we'd get this for free
        possible_values = [
            "mainnet",
            "ropsten",
            "rinkeby",
            "goerli",
            "kovan",
            "xdai",
            "polygon",
            "polygon_mumbai",
            "avalanche",
            "avalanche_fuji",
            "sepolia",
            "moonbeam",
            "moonbeam_dev",
            "moonriver",
            "optimism",
            "optimism-kovan"
        ])]
    pub inner: Chain,
}

#[derive(Parser, Debug, Clone)]
pub struct EthereumOpts {
    #[clap(env = "ETH_RPC_URL", long = "rpc-url", help = "The tracing / archival node's URL")]
    pub rpc_url: Option<String>,

    #[clap(env = "ETH_FROM", short, long = "from", help = "The sender account")]
    pub from: Option<Address>,

    #[clap(flatten)]
    pub wallet: Wallet,

    #[clap(long, help = "Use the flashbots RPC URL (https://rpc.flashbots.net)")]
    pub flashbots: bool,

    #[clap(long, env = "ETHERSCAN_API_KEY")]
    pub etherscan_api_key: Option<String>,

    #[clap(long, env = "CHAIN", default_value = "mainnet")]
    pub chain: Chain,
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
            self.from.unwrap_or_else(Address::zero)
        }
    }

    #[allow(unused)]
    pub async fn signer(&self, chain_id: U256) -> eyre::Result<Option<WalletType>> {
        self.signer_with(chain_id, Provider::try_from(self.rpc_url()?)?).await
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
            self.rpc_url.as_deref().ok_or_else(|| eyre::Error::msg("no Ethereum RPC provided, maybe you forgot to set the --rpc-url or the ETH_RPC_URL parameter? Alternatively, consider using the --flashbots flag to get frontrunning protection"))
        }
    }
}

// Make this args a `Figment` so that it can be merged into the `Config`
impl figment::Provider for EthereumOpts {
    fn metadata(&self) -> Metadata {
        Metadata::named("Ethereum Opts Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut dict = Dict::new();
        let rpc = self.rpc_url().map_err(|err| err.to_string())?;
        dict.insert("eth_rpc_url".to_string(), Value::from(rpc.to_string()));
        dict.insert("chain".to_string(), Value::from(self.chain.to_string()));
        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

#[derive(Debug)]
pub enum WalletType {
    Local(SignerMiddleware<Provider<Http>, LocalWallet>),
    Ledger(SignerMiddleware<Provider<Http>, Ledger>),
    Trezor(SignerMiddleware<Provider<Http>, Trezor>),
}

#[derive(Parser, Debug, Clone)]
#[cfg_attr(not(doc), allow(missing_docs))]
#[cfg_attr(
    doc,
    doc = r#"
The wallet options can either be:
1. Ledger
2. Trezor
3. Mnemonic (via file path)
4. Keystore (via file path)
5. Private Key (cleartext in CLI)
6. Private Key (interactively via secure prompt)
"#
)]
pub struct Wallet {
    #[clap(long, short, help = "Interactive prompt to insert your private key")]
    pub interactive: bool,

    #[clap(long = "private-key", help = "Your private key string")]
    pub private_key: Option<String>,

    #[clap(env = "ETH_KEYSTORE", long = "keystore", help = "Path to your keystore folder / file")]
    pub keystore_path: Option<String>,

    #[clap(long = "password", help = "Your keystore password", requires = "keystore-path")]
    pub keystore_password: Option<String>,

    #[clap(long = "mnemonic-path", help = "Path to your mnemonic file")]
    pub mnemonic_path: Option<String>,

    #[clap(short, long = "ledger", help = "Use your Ledger hardware wallet")]
    pub ledger: bool,

    #[clap(short, long = "trezor", help = "Use your Trezor hardware wallet")]
    pub trezor: bool,

    #[clap(long = "hd-path", help = "Derivation path for your hardware wallet (trezor or ledger)")]
    pub hd_path: Option<String>,

    #[clap(
        long = "mnemonic_index",
        help = "your index in the standard hd path",
        default_value = "0"
    )]
    pub mnemonic_index: u32,
}

impl Wallet {
    fn interactive(&self) -> Result<Option<LocalWallet>> {
        Ok(if self.interactive {
            println!("Insert private key:");
            let private_key = rpassword::read_password()?;
            let private_key = private_key.strip_prefix("0x").unwrap_or(&private_key);
            Some(LocalWallet::from_str(private_key)?)
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
