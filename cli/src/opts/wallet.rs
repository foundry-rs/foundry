use std::{path::Path, str::FromStr, sync::Arc};

use clap::Parser;
use ethers::{
    middleware::SignerMiddleware,
    prelude::Signer,
    signers::{coins_bip39::English, Ledger, LocalWallet, MnemonicBuilder, Trezor},
    types::Address,
};
use eyre::{eyre, Result};
use foundry_common::{fs, RetryProvider};
use serde::Serialize;

type SignerClient<T> = SignerMiddleware<Arc<RetryProvider>, T>;

#[derive(Debug)]
pub enum WalletType {
    Local(SignerClient<LocalWallet>),
    Ledger(SignerClient<Ledger>),
    Trezor(SignerClient<Trezor>),
}

impl From<SignerClient<Ledger>> for WalletType {
    fn from(hw: SignerClient<Ledger>) -> WalletType {
        WalletType::Ledger(hw)
    }
}

impl From<SignerClient<Trezor>> for WalletType {
    fn from(hw: SignerClient<Trezor>) -> WalletType {
        WalletType::Trezor(hw)
    }
}

impl From<SignerClient<LocalWallet>> for WalletType {
    fn from(wallet: SignerClient<LocalWallet>) -> WalletType {
        WalletType::Local(wallet)
    }
}

impl WalletType {
    pub fn chain_id(&self) -> u64 {
        match self {
            WalletType::Local(inner) => inner.signer().chain_id(),
            WalletType::Ledger(inner) => inner.signer().chain_id(),
            WalletType::Trezor(inner) => inner.signer().chain_id(),
        }
    }
}

#[derive(Parser, Debug, Default, Clone, Serialize)]
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
    #[clap(
        long,
        short,
        help_heading = "WALLET OPTIONS - RAW",
        help = "Open an interactive prompt to enter your private key."
    )]
    pub interactive: bool,

    #[clap(
        long = "private-key",
        help_heading = "WALLET OPTIONS - RAW",
        help = "Use the provided private key.",
        value_name = "RAW_PRIVATE_KEY",
        value_parser = foundry_common::clap_helpers::strip_0x_prefix
    )]
    pub private_key: Option<String>,

    #[clap(
        long = "mnemonic",
        alias = "mnemonic-path",
        help_heading = "WALLET OPTIONS - RAW",
        help = "Use the mnemonic phrase of mnemonic file at the specified path.",
        value_name = "PATH"
    )]
    pub mnemonic: Option<String>,

    #[clap(
        long = "mnemonic-passphrase",
        help_heading = "WALLET OPTIONS - RAW",
        help = "Use a BIP39 passphrase for the mnemonic.",
        value_name = "PASSPHRASE"
    )]
    pub mnemonic_passphrase: Option<String>,

    #[clap(
        long = "mnemonic-derivation-path",
        alias = "hd-path",
        help_heading = "WALLET OPTIONS - RAW",
        help = "The wallet derivation path. Works with both --mnemonic-path and hardware wallets.",
        value_name = "PATH"
    )]
    pub hd_path: Option<String>,

    #[clap(
        long = "mnemonic-index",
        conflicts_with = "hd-path",
        help_heading = "WALLET OPTIONS - RAW",
        help = "Use the private key from the given mnemonic index. Used with --mnemonic-path.",
        default_value = "0",
        value_name = "INDEX"
    )]
    pub mnemonic_index: u32,

    #[clap(
        env = "ETH_KEYSTORE",
        long = "keystore",
        help_heading = "WALLET OPTIONS - KEYSTORE",
        help = "Use the keystore in the given folder or file.",
        value_name = "PATH"
    )]
    pub keystore_path: Option<String>,

    #[clap(
        long = "password",
        help_heading = "WALLET OPTIONS - KEYSTORE",
        help = "The keystore password. Used with --keystore.",
        requires = "keystore-path",
        value_name = "PASSWORD"
    )]
    pub keystore_password: Option<String>,

    #[clap(
        short,
        long = "ledger",
        help_heading = "WALLET OPTIONS - HARDWARE WALLET",
        help = "Use a Ledger hardware wallet."
    )]
    pub ledger: bool,

    #[clap(
        short,
        long = "trezor",
        help_heading = "WALLET OPTIONS - HARDWARE WALLET",
        help = "Use a Trezor hardware wallet."
    )]
    pub trezor: bool,

    #[clap(
        env = "ETH_FROM",
        short,
        long = "from",
        help_heading = "WALLET OPTIONS - REMOTE",
        help = "The sender account.",
        value_name = "ADDRESS"
    )]
    pub from: Option<Address>,
}

impl Wallet {
    pub fn interactive(&self) -> Result<Option<LocalWallet>> {
        Ok(if self.interactive { Some(self.get_from_interactive()?) } else { None })
    }

    pub fn private_key(&self) -> Result<Option<LocalWallet>> {
        Ok(if let Some(ref private_key) = self.private_key {
            Some(self.get_from_private_key(private_key)?)
        } else {
            None
        })
    }

    pub fn keystore(&self) -> Result<Option<LocalWallet>> {
        self.get_from_keystore(self.keystore_path.as_ref(), self.keystore_password.as_ref())
    }

    pub fn mnemonic(&self) -> Result<Option<LocalWallet>> {
        Ok(if let Some(ref mnemonic) = self.mnemonic {
            Some(self.get_from_mnemonic(
                mnemonic,
                self.mnemonic_passphrase.as_ref(),
                self.hd_path.as_ref(),
                self.mnemonic_index,
            )?)
        } else {
            None
        })
    }
}

impl WalletTrait for Wallet {}

pub trait WalletTrait {
    fn get_from_interactive(&self) -> Result<LocalWallet> {
        println!("Insert private key:");
        let private_key = rpassword::read_password()?;
        let private_key = private_key.strip_prefix("0x").unwrap_or(&private_key);
        Ok(LocalWallet::from_str(private_key)?)
    }

    fn get_from_private_key(&self, private_key: &str) -> Result<LocalWallet> {
        let privk = private_key.trim().strip_prefix("0x").unwrap_or(private_key);
        LocalWallet::from_str(privk)
            .map_err(|x| eyre!("Failed to create wallet from private key: {x}"))
    }

    fn get_from_mnemonic(
        &self,
        mnemonic: &String,
        passphrase: Option<&String>,
        derivation_path: Option<&String>,
        index: u32,
    ) -> Result<LocalWallet> {
        let mnemonic = if Path::new(mnemonic).is_file() {
            fs::read_to_string(mnemonic)?.replace('\n', "")
        } else {
            mnemonic.to_owned()
        };
        let builder = MnemonicBuilder::<English>::default().phrase(mnemonic.as_str());
        let builder = if let Some(passphrase) = passphrase {
            builder.password(passphrase.as_str())
        } else {
            builder
        };
        let builder = if let Some(hd_path) = derivation_path {
            builder.derivation_path(hd_path.as_str())?
        } else {
            builder.index(index)?
        };
        Ok(builder.build()?)
    }

    fn get_from_keystore(
        &self,
        keystore_path: Option<&String>,
        keystore_password: Option<&String>,
    ) -> Result<Option<LocalWallet>> {
        Ok(match (keystore_path, keystore_password) {
            (Some(path), Some(password)) => Some(LocalWallet::decrypt_keystore(path, password)?),
            (Some(path), None) => {
                println!("Insert keystore password:");
                let password = rpassword::read_password().unwrap();
                Some(LocalWallet::decrypt_keystore(path, password)?)
            }
            (None, _) => None,
        })
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
            mnemonic: None,
            mnemonic_passphrase: None,
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
