use crate::error::WalletSignerError;
use async_trait::async_trait;
use ethers_core::types::{
    transaction::{eip2718::TypedTransaction, eip712::Eip712},
    Signature,
};
use ethers_signers::{
    coins_bip39::English, AwsSigner, HDPath as LedgerHDPath, Ledger, LocalWallet, MnemonicBuilder,
    Signer, Trezor, TrezorHDPath,
};
use rusoto_core::{
    credential::ChainProvider as AwsChainProvider, region::Region as AwsRegion,
    request::HttpClient as AwsHttpClient, Client as AwsClient,
};
use rusoto_kms::KmsClient;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, WalletSignerError>;

/// Wrapper enum around different signers.
#[derive(Debug)]
pub enum WalletSigner {
    /// Wrapper around local wallet. e.g. private key, mnemonic
    Local(LocalWallet),
    /// Wrapper around Ledger signer.
    Ledger(Ledger),
    /// Wrapper around Trezor signer.
    Trezor(Trezor),
    /// Wrapper around AWS KMS signer.
    Aws(AwsSigner),
}

impl WalletSigner {
    pub async fn from_ledger_path(path: LedgerHDPath) -> Result<Self> {
        let ledger = Ledger::new(path, 1).await?;
        Ok(Self::Ledger(ledger))
    }

    pub async fn from_trezor_path(path: TrezorHDPath) -> Result<Self> {
        // cached to ~/.ethers-rs/trezor/cache/trezor.session
        let trezor = Trezor::new(path, 1, None).await?;
        Ok(Self::Trezor(trezor))
    }

    pub async fn from_aws(key_id: &str) -> Result<Self> {
        let client =
            AwsClient::new_with(AwsChainProvider::default(), AwsHttpClient::new().unwrap());

        let kms = KmsClient::new_with_client(client, AwsRegion::default());

        Ok(Self::Aws(AwsSigner::new(kms, key_id, 1).await?))
    }

    pub fn from_private_key(private_key: impl AsRef<[u8]>) -> Result<Self> {
        let wallet = LocalWallet::from_bytes(private_key.as_ref())?;
        Ok(Self::Local(wallet))
    }

    pub fn from_mnemonic(
        mnemonic: &str,
        passphrase: Option<&str>,
        derivation_path: Option<&str>,
        index: u32,
    ) -> Result<Self> {
        let mut builder = MnemonicBuilder::<English>::default().phrase(mnemonic);

        if let Some(passphrase) = passphrase {
            builder = builder.password(passphrase)
        }

        builder = if let Some(hd_path) = derivation_path {
            builder.derivation_path(hd_path)?
        } else {
            builder.index(index)?
        };

        Ok(Self::Local(builder.build()?))
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

    async fn sign_message<S: Send + Sync + AsRef<[u8]>>(&self, message: S) -> Result<Signature> {
        delegate!(self, inner => inner.sign_message(message).await.map_err(Into::into))
    }

    async fn sign_transaction(&self, message: &TypedTransaction) -> Result<Signature> {
        delegate!(self, inner => inner.sign_transaction(message).await.map_err(Into::into))
    }

    async fn sign_typed_data<T: Eip712 + Send + Sync>(&self, payload: &T) -> Result<Signature> {
        delegate!(self, inner => inner.sign_typed_data(payload).await.map_err(Into::into))
    }

    fn address(&self) -> ethers_core::types::Address {
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

    async fn sign_message<S: Send + Sync + AsRef<[u8]>>(&self, message: S) -> Result<Signature> {
        (*self).sign_message(message).await
    }

    async fn sign_transaction(&self, message: &TypedTransaction) -> Result<Signature> {
        (*self).sign_transaction(message).await
    }

    async fn sign_typed_data<T: Eip712 + Send + Sync>(&self, payload: &T) -> Result<Signature> {
        (*self).sign_typed_data(payload).await
    }

    fn address(&self) -> ethers_core::types::Address {
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

/// Signers that require user action to be obtained.
#[derive(Debug, Clone)]
pub enum PendingSigner {
    Keystore(PathBuf),
    Interactive,
}

impl PendingSigner {
    pub fn unlock(self) -> Result<WalletSigner> {
        match self {
            Self::Keystore(path) => {
                let password = rpassword::prompt_password("Enter keystore password:")?;
                Ok(WalletSigner::Local(LocalWallet::decrypt_keystore(path, password)?))
            }
            Self::Interactive => {
                let private_key = rpassword::prompt_password("Enter private key:")?;
                Ok(WalletSigner::from_private_key(hex::decode(private_key)?)?)
            }
        }
    }
}
