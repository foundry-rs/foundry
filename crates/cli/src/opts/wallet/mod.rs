use crate::opts::error::PrivateKeyError;
use async_trait::async_trait;
use clap::Parser;
use ethers::{
    signers::{
        coins_bip39::English, AwsSigner, AwsSignerError, HDPath as LedgerHDPath, Ledger,
        LedgerError, LocalWallet, MnemonicBuilder, Signer, Trezor, TrezorError, TrezorHDPath,
        WalletError,
    },
    types::{
        transaction::{eip2718::TypedTransaction, eip712::Eip712},
        Address, Signature,
    },
};
use eyre::{bail, Result, WrapErr};
use foundry_common::fs;
use foundry_config::Config;
use rusoto_core::{
    credential::ChainProvider as AwsChainProvider, region::Region as AwsRegion,
    request::HttpClient as AwsHttpClient, Client as AwsClient,
};
use rusoto_kms::KmsClient;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};
use tracing::{instrument, trace};

pub mod multi_wallet;
pub use multi_wallet::*;

pub mod error;

/// A wrapper for the raw data options for `Wallet`, extracted to also be used standalone.
/// The raw wallet options can either be:
/// 1. Private Key (cleartext in CLI)
/// 2. Private Key (interactively via secure prompt)
/// 3. Mnemonic (via file path)
#[derive(Parser, Debug, Default, Clone, Serialize)]
#[clap(next_help_heading = "Wallet options - raw", about = None, long_about = None)]
pub struct RawWallet {
    /// Open an interactive prompt to enter your private key.
    #[clap(long, short)]
    pub interactive: bool,

    /// Use the provided private key.
    #[clap(
        long,
        value_name = "RAW_PRIVATE_KEY",
        value_parser = foundry_common::clap_helpers::strip_0x_prefix
    )]
    pub private_key: Option<String>,

    /// Use the mnemonic phrase of mnemonic file at the specified path.
    #[clap(long, alias = "mnemonic-path")]
    pub mnemonic: Option<String>,

    /// Use a BIP39 passphrase for the mnemonic.
    #[clap(long, value_name = "PASSPHRASE")]
    pub mnemonic_passphrase: Option<String>,

    /// The wallet derivation path.
    ///
    /// Works with both --mnemonic-path and hardware wallets.
    #[clap(long = "mnemonic-derivation-path", alias = "hd-path", value_name = "PATH")]
    pub hd_path: Option<String>,

    /// Use the private key from the given mnemonic index.
    ///
    /// Used with --mnemonic-path.
    #[clap(long, conflicts_with = "hd_path", default_value_t = 0, value_name = "INDEX")]
    pub mnemonic_index: u32,
}

/// The wallet options can either be:
/// 1. Raw (via private key / mnemonic file, see `RawWallet`)
/// 2. Ledger
/// 3. Trezor
/// 4. Keystore (via file path)
/// 5. AWS KMS
#[derive(Parser, Debug, Default, Clone, Serialize)]
#[clap(next_help_heading = "Wallet options", about = None, long_about = None)]
pub struct Wallet {
    /// The sender account.
    #[clap(
        long,
        short,
        value_name = "ADDRESS",
        help_heading = "Wallet options - raw",
        env = "ETH_FROM"
    )]
    pub from: Option<Address>,

    #[clap(flatten)]
    pub raw: RawWallet,

    /// Use the keystore in the given folder or file.
    #[clap(
        long = "keystore",
        help_heading = "Wallet options - keystore",
        value_name = "PATH",
        env = "ETH_KEYSTORE"
    )]
    pub keystore_path: Option<String>,

    /// Use a keystore from the default keystores folder (~/.foundry/keystores) by its filename
    #[clap(
        long = "account",
        help_heading = "Wallet options - keystore",
        value_name = "ACCOUNT_NAME",
        env = "ETH_KEYSTORE_ACCOUNT",
        conflicts_with = "keystore_path"
    )]
    pub keystore_account_name: Option<String>,

    /// The keystore password.
    ///
    /// Used with --keystore.
    #[clap(
        long = "password",
        help_heading = "Wallet options - keystore",
        requires = "keystore_path",
        value_name = "PASSWORD"
    )]
    pub keystore_password: Option<String>,

    /// The keystore password file path.
    ///
    /// Used with --keystore.
    #[clap(
        long = "password-file",
        help_heading = "Wallet options - keystore",
        requires = "keystore_path",
        value_name = "PASSWORD_FILE",
        env = "ETH_PASSWORD"
    )]
    pub keystore_password_file: Option<String>,

    /// Use a Ledger hardware wallet.
    #[clap(long, short, help_heading = "Wallet options - hardware wallet")]
    pub ledger: bool,

    /// Use a Trezor hardware wallet.
    #[clap(long, short, help_heading = "Wallet options - hardware wallet")]
    pub trezor: bool,

    /// Use AWS Key Management Service.
    #[clap(long, help_heading = "Wallet options - AWS KMS")]
    pub aws: bool,
}

impl From<RawWallet> for Wallet {
    fn from(options: RawWallet) -> Self {
        Self { raw: options, ..Default::default() }
    }
}

impl Wallet {
    pub fn interactive(&self) -> Result<Option<LocalWallet>> {
        Ok(if self.raw.interactive { Some(self.get_from_interactive()?) } else { None })
    }

    pub fn private_key(&self) -> Result<Option<LocalWallet>> {
        Ok(if let Some(ref private_key) = self.raw.private_key {
            Some(self.get_from_private_key(private_key)?)
        } else {
            None
        })
    }

    pub fn keystore(&self) -> Result<Option<LocalWallet>> {
        let default_keystore_dir = Config::foundry_keystores_dir()
            .ok_or_else(|| eyre::eyre!("Could not find the default keystore directory."))?;
        // If keystore path is provided, use it, otherwise use default path + keystore account name
        let keystore_path: Option<String> = self.keystore_path.clone().or_else(|| {
            self.keystore_account_name.as_ref().map(|keystore_name| {
                default_keystore_dir.join(keystore_name).to_string_lossy().into_owned()
            })
        });

        self.get_from_keystore(
            keystore_path.as_ref(),
            self.keystore_password.as_ref(),
            self.keystore_password_file.as_ref(),
        )
    }

    pub fn mnemonic(&self) -> Result<Option<LocalWallet>> {
        Ok(if let Some(ref mnemonic) = self.raw.mnemonic {
            Some(self.get_from_mnemonic(
                mnemonic,
                self.raw.mnemonic_passphrase.as_ref(),
                self.raw.hd_path.as_ref(),
                self.raw.mnemonic_index,
            )?)
        } else {
            None
        })
    }

    /// Returns the sender address of the signer or `from`.
    pub async fn sender(&self) -> Address {
        if let Ok(signer) = self.signer(0).await {
            signer.address()
        } else {
            self.from.unwrap_or_else(Address::zero)
        }
    }

    /// Tries to resolve a local wallet from the provided options.
    #[track_caller]
    pub fn try_resolve_local_wallet(&self) -> Result<Option<LocalWallet>> {
        self.private_key()
            .transpose()
            .or_else(|| self.interactive().transpose())
            .or_else(|| self.mnemonic().transpose())
            .or_else(|| self.keystore().transpose())
            .transpose()
    }
    /// Returns a [Signer] corresponding to the provided private key, mnemonic or hardware signer.
    #[instrument(skip(self), level = "trace")]
    pub async fn signer(&self, chain_id: u64) -> Result<WalletSigner> {
        trace!("start finding signer");

        if self.ledger {
            let derivation = match self.raw.hd_path.as_ref() {
                Some(hd_path) => LedgerHDPath::Other(hd_path.clone()),
                None => LedgerHDPath::LedgerLive(self.raw.mnemonic_index as usize),
            };
            let ledger = Ledger::new(derivation, chain_id).await.wrap_err_with(|| {
                "\
Could not connect to Ledger device.
Make sure it's connected and unlocked, with no other desktop wallet apps open."
            })?;

            Ok(WalletSigner::Ledger(ledger))
        } else if self.trezor {
            let derivation = match self.raw.hd_path.as_ref() {
                Some(hd_path) => TrezorHDPath::Other(hd_path.clone()),
                None => TrezorHDPath::TrezorLive(self.raw.mnemonic_index as usize),
            };

            // cached to ~/.ethers-rs/trezor/cache/trezor.session
            let trezor = Trezor::new(derivation, chain_id, None).await.wrap_err_with(|| {
                "\
Could not connect to Trezor device.
Make sure it's connected and unlocked, with no other conflicting desktop wallet apps open."
            })?;

            Ok(WalletSigner::Trezor(trezor))
        } else if self.aws {
            let client =
                AwsClient::new_with(AwsChainProvider::default(), AwsHttpClient::new().unwrap());

            let kms = KmsClient::new_with_client(client, AwsRegion::default());

            let key_id = std::env::var("AWS_KMS_KEY_ID")?;

            let aws_signer = AwsSigner::new(kms, key_id, chain_id).await?;

            Ok(WalletSigner::Aws(aws_signer))
        } else {
            trace!("finding local key");

            let maybe_local = self.try_resolve_local_wallet()?;

            let local = maybe_local.ok_or_else(|| {
                eyre::eyre!(
                    "\
Error accessing local wallet. Did you set a private key, mnemonic or keystore?
Run `cast send --help` or `forge create --help` and use the corresponding CLI
flag to set your key via:
--private-key, --mnemonic-path, --aws, --interactive, --trezor or --ledger.
Alternatively, if you're using a local node with unlocked accounts,
use the --unlocked flag and either set the `ETH_FROM` environment variable to the address
of the unlocked account you want to use, or provide the --from flag with the address directly."
                )
            })?;

            Ok(WalletSigner::Local(local.with_chain_id(chain_id)))
        }
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

    #[track_caller]
    fn get_from_private_key(&self, private_key: &str) -> Result<LocalWallet> {
        let privk = private_key.trim().strip_prefix("0x").unwrap_or(private_key);
        match LocalWallet::from_str(privk) {
            Ok(pk) => Ok(pk),
            Err(err) => {
                // helper closure to check if pk was meant to be an env var, this usually happens if
                // `$` is missing
                let ensure_not_env = |pk: &str| {
                    // check if pk was meant to be an env var
                    if !pk.starts_with("0x") && std::env::var(pk).is_ok() {
                        // SAFETY: at this point we know the user actually wanted to use an env var
                        // and most likely forgot the `$` anchor, so the
                        // `private_key` here is an unresolved env var
                        return Err(PrivateKeyError::ExistsAsEnvVar(pk.to_string()))
                    }
                    Ok(())
                };
                match err {
                    WalletError::HexError(err) => {
                        ensure_not_env(private_key)?;
                        return Err(PrivateKeyError::InvalidHex(err).into())
                    }
                    WalletError::EcdsaError(_) => {
                        ensure_not_env(private_key)?;
                    }
                    _ => {}
                };
                bail!("Failed to create wallet from private key: {err}")
            }
        }
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

    /// Ensures the path to the keystore exists.
    ///
    /// if the path is a directory, it bails and asks the user to specify the keystore file
    /// directly.
    fn find_keystore_file(&self, path: impl AsRef<Path>) -> Result<PathBuf> {
        let path = path.as_ref();
        if !path.exists() {
            bail!("Keystore file `{path:?}` does not exist")
        }

        if path.is_dir() {
            bail!("Keystore path `{path:?}` is a directory. Please specify the keystore file directly.")
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
            // Path and password provided
            (Some(path), Some(password), _) => {
                let path = self.find_keystore_file(path)?;
                Some(
                    LocalWallet::decrypt_keystore(&path, password)
                        .wrap_err_with(|| format!("Failed to decrypt keystore {path:?}"))?,
                )
            }
            // Path and password file provided
            (Some(path), _, Some(password_file)) => {
                let path = self.find_keystore_file(path)?;
                Some(
                    LocalWallet::decrypt_keystore(&path, self.password_from_file(password_file)?)
                        .wrap_err_with(|| format!("Failed to decrypt keystore {path:?} with password file {password_file:?}"))?,
                )
            }
            // Only Path provided -> interactive
            (Some(path), None, None) => {
                let path = self.find_keystore_file(path)?;
                let password = rpassword::prompt_password("Enter keystore password:")?;
                Some(LocalWallet::decrypt_keystore(path, password)?)
            }
            // Nothing provided
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

#[derive(Debug, thiserror::Error)]
pub enum WalletSignerError {
    #[error(transparent)]
    Local(#[from] WalletError),
    #[error(transparent)]
    Ledger(#[from] LedgerError),
    #[error(transparent)]
    Trezor(#[from] TrezorError),
    #[error(transparent)]
    Aws(#[from] AwsSignerError),
}

#[derive(Debug)]
pub enum WalletSigner {
    Local(LocalWallet),
    Ledger(Ledger),
    Trezor(Trezor),
    Aws(AwsSigner),
}

impl From<LocalWallet> for WalletSigner {
    fn from(wallet: LocalWallet) -> Self {
        Self::Local(wallet)
    }
}

impl From<Ledger> for WalletSigner {
    fn from(hw: Ledger) -> Self {
        Self::Ledger(hw)
    }
}

impl From<Trezor> for WalletSigner {
    fn from(hw: Trezor) -> Self {
        Self::Trezor(hw)
    }
}

impl From<AwsSigner> for WalletSigner {
    fn from(wallet: AwsSigner) -> Self {
        Self::Aws(wallet)
    }
}

macro_rules! delegate {
    ($s:ident, $inner:ident => $e:expr) => {
        match $s {
            Self::Local($inner) => $e,
            Self::Ledger($inner) => $e,
            Self::Trezor($inner) => $e,
            Self::Aws($inner) => $e,
        }
    };
}

#[async_trait]
impl Signer for WalletSigner {
    type Error = WalletSignerError;

    async fn sign_message<S: Send + Sync + AsRef<[u8]>>(
        &self,
        message: S,
    ) -> Result<Signature, Self::Error> {
        delegate!(self, inner => inner.sign_message(message).await.map_err(Into::into))
    }

    async fn sign_transaction(&self, message: &TypedTransaction) -> Result<Signature, Self::Error> {
        delegate!(self, inner => inner.sign_transaction(message).await.map_err(Into::into))
    }

    async fn sign_typed_data<T: Eip712 + Send + Sync>(
        &self,
        payload: &T,
    ) -> Result<Signature, Self::Error> {
        delegate!(self, inner => inner.sign_typed_data(payload).await.map_err(Into::into))
    }

    fn address(&self) -> Address {
        delegate!(self, inner => inner.address())
    }

    fn chain_id(&self) -> u64 {
        delegate!(self, inner => inner.chain_id())
    }

    fn with_chain_id<T: Into<u64>>(self, chain_id: T) -> Self {
        match self {
            Self::Local(inner) => Self::Local(inner.with_chain_id(chain_id)),
            Self::Ledger(inner) => Self::Ledger(inner.with_chain_id(chain_id)),
            Self::Trezor(inner) => Self::Trezor(inner.with_chain_id(chain_id)),
            Self::Aws(inner) => Self::Aws(inner.with_chain_id(chain_id)),
        }
    }
}

#[async_trait]
impl Signer for &WalletSigner {
    type Error = WalletSignerError;

    async fn sign_message<S: Send + Sync + AsRef<[u8]>>(
        &self,
        message: S,
    ) -> Result<Signature, Self::Error> {
        (*self).sign_message(message).await
    }

    async fn sign_transaction(&self, message: &TypedTransaction) -> Result<Signature, Self::Error> {
        (*self).sign_transaction(message).await
    }

    async fn sign_typed_data<T: Eip712 + Send + Sync>(
        &self,
        payload: &T,
    ) -> Result<Signature, Self::Error> {
        (*self).sign_typed_data(payload).await
    }

    fn address(&self) -> Address {
        (*self).address()
    }

    fn chain_id(&self) -> u64 {
        (*self).chain_id()
    }

    fn with_chain_id<T: Into<u64>>(self, chain_id: T) -> Self {
        let _ = chain_id;
        self
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
        let keystore =
            Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../cast/tests/fixtures/keystore"));
        let keystore_file = keystore
            .join("UTC--2022-10-30T06-51-20.130356000Z--560d246fcddc9ea98a8b032c9a2f474efb493c28");
        let wallet: Wallet = Wallet::parse_from([
            "foundry-cli",
            "--from",
            "560d246fcddc9ea98a8b032c9a2f474efb493c28",
        ]);
        let file = wallet.find_keystore_file(&keystore_file).unwrap();
        assert_eq!(file, keystore_file);
    }

    #[test]
    fn illformed_private_key_generates_user_friendly_error() {
        let wallet = Wallet {
            raw: RawWallet {
                interactive: false,
                private_key: Some("123".to_string()),
                mnemonic: None,
                mnemonic_passphrase: None,
                hd_path: None,
                mnemonic_index: 0,
            },
            from: None,
            keystore_path: None,
            keystore_account_name: None,
            keystore_password: None,
            keystore_password_file: None,
            ledger: false,
            trezor: false,
            aws: false,
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
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../cast/tests/fixtures/keystore/password");
        let wallet: Wallet = Wallet::parse_from(["foundry-cli"]);
        let password = wallet.password_from_file(path).unwrap();
        assert_eq!(password, "this is keystore password")
    }
}
