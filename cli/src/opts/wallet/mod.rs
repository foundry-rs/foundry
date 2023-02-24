use clap::Parser;
use ethers::{
    middleware::SignerMiddleware,
    prelude::Signer,
    signers::{coins_bip39::English, AwsSigner, Ledger, LocalWallet, MnemonicBuilder, Trezor},
    types::Address,
};
use eyre::{bail, eyre, Result, WrapErr};
use foundry_common::{fs, RetryProvider};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

pub mod multi_wallet;
use crate::opts::error::PrivateKeyError;
pub use multi_wallet::*;

pub mod error;

type SignerClient<T> = SignerMiddleware<Arc<RetryProvider>, T>;

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
7. AWS KMS
"#
)]
#[clap(next_help_heading = "Wallet options")]
pub struct Wallet {
    #[clap(
        long,
        short,
        help_heading = "Wallet options - raw",
        help = "Open an interactive prompt to enter your private key."
    )]
    pub interactive: bool,

    #[clap(
        long = "private-key",
        help_heading = "Wallet options - raw",
        help = "Use the provided private key.",
        value_name = "RAW_PRIVATE_KEY",
        value_parser = foundry_common::clap_helpers::strip_0x_prefix
    )]
    pub private_key: Option<String>,

    #[clap(
        long = "mnemonic",
        alias = "mnemonic-path",
        help_heading = "Wallet options - raw",
        help = "Use the mnemonic phrase of mnemonic file at the specified path.",
        value_name = "PATH"
    )]
    pub mnemonic: Option<String>,

    #[clap(
        long = "mnemonic-passphrase",
        help_heading = "Wallet options - raw",
        help = "Use a BIP39 passphrase for the mnemonic.",
        value_name = "PASSPHRASE"
    )]
    pub mnemonic_passphrase: Option<String>,

    #[clap(
        long = "mnemonic-derivation-path",
        alias = "hd-path",
        help_heading = "Wallet options - raw",
        help = "The wallet derivation path. Works with both --mnemonic-path and hardware wallets.",
        value_name = "PATH"
    )]
    pub hd_path: Option<String>,

    #[clap(
        long = "mnemonic-index",
        conflicts_with = "hd_path",
        help_heading = "Wallet options - raw",
        help = "Use the private key from the given mnemonic index. Used with --mnemonic-path.",
        default_value = "0",
        value_name = "INDEX"
    )]
    pub mnemonic_index: u32,

    #[clap(
        env = "ETH_KEYSTORE",
        long = "keystore",
        help_heading = "Wallet options - keystore",
        help = "Use the keystore in the given folder or file.",
        value_name = "PATH"
    )]
    pub keystore_path: Option<String>,

    #[clap(
        long = "password",
        help_heading = "Wallet options - keystore",
        help = "The keystore password. Used with --keystore.",
        requires = "keystore_path",
        value_name = "PASSWORD"
    )]
    pub keystore_password: Option<String>,

    #[clap(
        env = "ETH_PASSWORD",
        long = "password-file",
        help_heading = "Wallet options - keystore",
        help = "The keystore password file path. Used with --keystore.",
        requires = "keystore_path",
        value_name = "PASSWORD_FILE"
    )]
    pub keystore_password_file: Option<String>,

    #[clap(
        short,
        long = "ledger",
        help_heading = "Wallet options - hardware wallet",
        help = "Use a Ledger hardware wallet."
    )]
    pub ledger: bool,

    #[clap(
        short,
        long = "trezor",
        help_heading = "Wallet options - hardware wallet",
        help = "Use a Trezor hardware wallet."
    )]
    pub trezor: bool,

    #[clap(
        long = "aws",
        help_heading = "WALLET OPTIONS - KEYSTORE",
        help = "Use AWS Key Management Service"
    )]
    pub aws: bool,

    #[clap(
        env = "ETH_FROM",
        short,
        long = "from",
        help_heading = "Wallet options - remote",
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
        self.get_from_keystore(
            self.keystore_path.as_ref(),
            self.keystore_password.as_ref(),
            self.keystore_password_file.as_ref(),
        )
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

pub trait WalletTrait {
    /// Returns the configured sender.
    fn sender(&self) -> Option<Address>;

    fn get_from_interactive(&self) -> Result<LocalWallet> {
        let private_key = rpassword::prompt_password("Enter private key: ")?;
        let private_key = private_key.strip_prefix("0x").unwrap_or(&private_key);
        Ok(LocalWallet::from_str(private_key)?)
    }

    fn get_from_private_key(&self, private_key: &str) -> Result<LocalWallet> {
        use ethers::signers::WalletError;
        let privk = private_key.trim().strip_prefix("0x").unwrap_or(private_key);
        LocalWallet::from_str(privk).map_err(|err| {
            // helper macro to check if pk was meant to be an env var, this usually happens if `$`
            // is missing
            macro_rules! bail_env_var {
                ($private_key:ident) => {
                    // check if pk was meant to be an env var
                    if !$private_key.starts_with("0x") && std::env::var($private_key).is_ok() {
                        // SAFETY: at this point we know the user actually wanted to use an env var
                        // and most likely forgot the `$` anchor, so the
                        // `private_key` here is an unresolved env var
                        return PrivateKeyError::ExistsAsEnvVar($private_key.to_string()).into()
                    }
                };
            }
            match err {
                WalletError::HexError(err) => {
                    bail_env_var!(private_key);
                    return PrivateKeyError::InvalidHex(err).into()
                }
                WalletError::EcdsaError(_) => {
                    bail_env_var!(private_key);
                }
                _ => {}
            };
            eyre!("Failed to create wallet from private key: {err}")
        })
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

    /// Attempts to find the actual path of the keystore file.
    ///
    /// If the path is a directory then we try to find the first keystore file with the correct
    /// sender address
    fn find_keystore_file(&self, path: impl AsRef<Path>) -> Result<PathBuf> {
        let path = path.as_ref();
        if !path.exists() {
            bail!("Keystore file `{path:?}` does not exist")
        }

        if path.is_dir() {
            let sender =
                self.sender().ok_or_else(|| eyre!("No sender account configured: $ETH_FROM"))?;

            let (_, file) = walkdir::WalkDir::new(path)
                .max_depth(2)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_file())
                .filter_map(|e| {
                    fs::read_json_file::<KeystoreFile>(e.path())
                        .map(|keystore| (keystore, e.path().to_path_buf()))
                        .ok()
                })
                .find(|(keystore, _)| keystore.address == sender)
                .ok_or_else(|| {
                    eyre!("No matching keystore file found for {sender:?} in {path:?}")
                })?;
            return Ok(file)
        }

        Ok(path.to_path_buf())
    }

    fn get_from_keystore(
        &self,
        keystore_path: Option<&String>,
        keystore_password: Option<&String>,
        keystore_password_file: Option<&String>,
    ) -> Result<Option<LocalWallet>> {
        Ok(match (keystore_path, keystore_password, keystore_password_file) {
            (Some(path), Some(password), _) => {
                let path = self.find_keystore_file(path)?;
                Some(
                    LocalWallet::decrypt_keystore(&path, password)
                        .wrap_err_with(|| format!("Failed to decrypt keystore {path:?}"))?,
                )
            }
            (Some(path), _, Some(password_file)) => {
                let path = self.find_keystore_file(path)?;
                Some(
                    LocalWallet::decrypt_keystore(&path, self.password_from_file(password_file)?)
                        .wrap_err_with(|| format!("Failed to decrypt keystore {path:?} with password file {password_file:?}"))?,
                )
            }
            (Some(path), None, None) => {
                let path = self.find_keystore_file(path)?;
                let password = rpassword::prompt_password("Enter keystore password:")?;
                Some(LocalWallet::decrypt_keystore(path, password)?)
            }
            (None, _, _) => None,
        })
    }

    /// Attempts to read the keystore password from the password file.
    fn password_from_file(&self, password_file: impl AsRef<Path>) -> Result<String> {
        let password_file = password_file.as_ref();
        if !password_file.is_file() {
            bail!("Keystore password file `{password_file:?}` does not exist")
        }

        Ok(fs::read_to_string(password_file)?.trim_end().to_string())
    }
}

impl WalletTrait for Wallet {
    fn sender(&self) -> Option<Address> {
        self.from
    }
}

#[derive(Debug)]
pub enum WalletType {
    Local(SignerClient<LocalWallet>),
    Ledger(SignerClient<Ledger>),
    Trezor(SignerClient<Trezor>),
    Aws(SignerClient<AwsSigner>),
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

impl From<SignerClient<AwsSigner>> for WalletType {
    fn from(wallet: SignerClient<AwsSigner>) -> WalletType {
        WalletType::Aws(wallet)
    }
}

impl WalletType {
    pub fn chain_id(&self) -> u64 {
        match self {
            WalletType::Local(inner) => inner.signer().chain_id(),
            WalletType::Ledger(inner) => inner.signer().chain_id(),
            WalletType::Trezor(inner) => inner.signer().chain_id(),
            WalletType::Aws(inner) => inner.signer().chain_id(),
        }
    }
}

/// Excerpt of a keystore file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeystoreFile {
    pub address: Address,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_keystore() {
        let keystore = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/keystore");
        let keystore_file = keystore
            .join("UTC--2022-10-30T06-51-20.130356000Z--560d246fcddc9ea98a8b032c9a2f474efb493c28");
        let wallet: Wallet = Wallet::parse_from([
            "foundry-cli",
            "--from",
            "560d246fcddc9ea98a8b032c9a2f474efb493c28",
        ]);
        let file = wallet.find_keystore_file(&keystore).unwrap();
        assert_eq!(file, keystore_file);

        let file = wallet.find_keystore_file(&keystore_file).unwrap();
        assert_eq!(file, keystore_file);
    }

    #[test]
    fn illformed_private_key_generates_user_friendly_error() {
        let wallet = Wallet {
            from: None,
            interactive: false,
            private_key: Some("123".to_string()),
            keystore_path: None,
            keystore_password: None,
            keystore_password_file: None,
            mnemonic: None,
            mnemonic_passphrase: None,
            ledger: false,
            trezor: false,
            aws: false,
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

    #[test]
    fn gets_password_from_file() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/keystore/password")
            .into_os_string();
        let wallet: Wallet = Wallet::parse_from(["foundry-cli"]);
        let password = wallet.password_from_file(path).unwrap();
        assert_eq!(password, "this is keystore password")
    }
}
