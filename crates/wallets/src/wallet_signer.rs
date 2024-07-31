use crate::error::WalletSignerError;
use alloy_consensus::SignableTransaction;
use alloy_dyn_abi::TypedData;
use alloy_network::TxSigner;
use alloy_primitives::{hex, Address, ChainId, B256};
use alloy_signer::{Signature, Signer};
use alloy_signer_ledger::{HDPath as LedgerHDPath, LedgerSigner};
use alloy_signer_local::{coins_bip39::English, MnemonicBuilder, PrivateKeySigner};
use alloy_signer_trezor::{HDPath as TrezorHDPath, TrezorSigner};
use alloy_sol_types::{Eip712Domain, SolStruct};
use async_trait::async_trait;
use std::path::PathBuf;

#[cfg(feature = "aws-kms")]
use {alloy_signer_aws::AwsSigner, aws_config::BehaviorVersion, aws_sdk_kms::Client as AwsClient};

#[cfg(feature = "gcp-kms")]
use {
    alloy_signer_gcp::{GcpKeyRingRef, GcpSigner, GcpSignerError, KeySpecifier},
    gcloud_sdk::{
        google::cloud::kms::v1::key_management_service_client::KeyManagementServiceClient,
        GoogleApi,
    },
};

pub type Result<T> = std::result::Result<T, WalletSignerError>;

/// Wrapper enum around different signers.
#[derive(Debug)]
pub enum WalletSigner {
    /// Wrapper around local wallet. e.g. private key, mnemonic
    Local(PrivateKeySigner),
    /// Wrapper around Ledger signer.
    Ledger(LedgerSigner),
    /// Wrapper around Trezor signer.
    Trezor(TrezorSigner),
    /// Wrapper around AWS KMS signer.
    #[cfg(feature = "aws-kms")]
    Aws(AwsSigner),
    /// Wrapper around Google Cloud KMS signer.
    #[cfg(feature = "gcp-kms")]
    Gcp(GcpSigner),
}

impl WalletSigner {
    pub async fn from_ledger_path(path: LedgerHDPath) -> Result<Self> {
        let ledger = LedgerSigner::new(path, None).await?;
        Ok(Self::Ledger(ledger))
    }

    pub async fn from_trezor_path(path: TrezorHDPath) -> Result<Self> {
        let trezor = TrezorSigner::new(path, None).await?;
        Ok(Self::Trezor(trezor))
    }

    pub async fn from_aws(key_id: String) -> Result<Self> {
        #[cfg(feature = "aws-kms")]
        {
            let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
            let client = AwsClient::new(&config);

            Ok(Self::Aws(AwsSigner::new(client, key_id, None).await?))
        }

        #[cfg(not(feature = "aws-kms"))]
        {
            let _ = key_id;
            Err(WalletSignerError::aws_unsupported())
        }
    }

    pub async fn from_gcp(
        project_id: String,
        location: String,
        keyring: String,
        key_name: String,
        key_version: u64,
    ) -> Result<Self> {
        #[cfg(feature = "gcp-kms")]
        {
            let keyring = GcpKeyRingRef::new(&project_id, &location, &keyring);
            let client = match GoogleApi::from_function(
                KeyManagementServiceClient::new,
                "https://cloudkms.googleapis.com",
                None,
            )
            .await
            {
                Ok(c) => c,
                Err(e) => return Err(WalletSignerError::from(GcpSignerError::GoogleKmsError(e))),
            };

            let specifier = KeySpecifier::new(keyring, &key_name, key_version);

            Ok(Self::Gcp(GcpSigner::new(client, specifier, None).await?))
        }

        #[cfg(not(feature = "gcp-kms"))]
        {
            let _ = project_id;
            let _ = location;
            let _ = keyring;
            let _ = key_name;
            let _ = key_version;
            Err(WalletSignerError::gcp_unsupported())
        }
    }

    pub fn from_private_key(private_key: &B256) -> Result<Self> {
        Ok(Self::Local(PrivateKeySigner::from_bytes(private_key)?))
    }

    /// Returns a list of addresses available to use with current signer
    ///
    /// - for Ledger and Trezor signers the number of addresses to retrieve is specified as argument
    /// - the result for Ledger signers includes addresses available for both LedgerLive and Legacy
    ///   derivation paths
    /// - for Local and AWS signers the result contains a single address
    pub async fn available_senders(&self, max: usize) -> Result<Vec<Address>> {
        let mut senders = Vec::new();
        match self {
            Self::Local(local) => {
                senders.push(local.address());
            }
            Self::Ledger(ledger) => {
                for i in 0..max {
                    if let Ok(address) =
                        ledger.get_address_with_path(&LedgerHDPath::LedgerLive(i)).await
                    {
                        senders.push(address);
                    }
                }
                for i in 0..max {
                    if let Ok(address) =
                        ledger.get_address_with_path(&LedgerHDPath::Legacy(i)).await
                    {
                        senders.push(address);
                    }
                }
            }
            Self::Trezor(trezor) => {
                for i in 0..max {
                    if let Ok(address) =
                        trezor.get_address_with_path(&TrezorHDPath::TrezorLive(i)).await
                    {
                        senders.push(address);
                    }
                }
            }
            #[cfg(feature = "aws-kms")]
            Self::Aws(aws) => {
                senders.push(alloy_signer::Signer::address(aws));
            }
            #[cfg(feature = "gcp-kms")]
            Self::Gcp(gcp) => {
                senders.push(alloy_signer::Signer::address(gcp));
            }
        }
        Ok(senders)
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
            #[cfg(feature = "aws-kms")]
            Self::Aws($inner) => $e,
            #[cfg(feature = "gcp-kms")]
            Self::Gcp($inner) => $e,
        }
    };
}

#[async_trait]
impl Signer for WalletSigner {
    /// Signs the given hash.
    async fn sign_hash(&self, hash: &B256) -> alloy_signer::Result<Signature> {
        delegate!(self, inner => inner.sign_hash(hash)).await
    }

    async fn sign_message(&self, message: &[u8]) -> alloy_signer::Result<Signature> {
        delegate!(self, inner => inner.sign_message(message)).await
    }

    fn address(&self) -> Address {
        delegate!(self, inner => alloy_signer::Signer::address(inner))
    }

    fn chain_id(&self) -> Option<ChainId> {
        delegate!(self, inner => inner.chain_id())
    }

    fn set_chain_id(&mut self, chain_id: Option<ChainId>) {
        delegate!(self, inner => inner.set_chain_id(chain_id))
    }

    async fn sign_typed_data<T: SolStruct + Send + Sync>(
        &self,
        payload: &T,
        domain: &Eip712Domain,
    ) -> alloy_signer::Result<Signature>
    where
        Self: Sized,
    {
        delegate!(self, inner => inner.sign_typed_data(payload, domain)).await
    }

    async fn sign_dynamic_typed_data(
        &self,
        payload: &TypedData,
    ) -> alloy_signer::Result<Signature> {
        delegate!(self, inner => inner.sign_dynamic_typed_data(payload)).await
    }
}

#[async_trait]
impl TxSigner<Signature> for WalletSigner {
    fn address(&self) -> Address {
        delegate!(self, inner => alloy_signer::Signer::address(inner))
    }

    async fn sign_transaction(
        &self,
        tx: &mut dyn SignableTransaction<Signature>,
    ) -> alloy_signer::Result<Signature> {
        delegate!(self, inner => inner.sign_transaction(tx)).await
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
                match PrivateKeySigner::decrypt_keystore(path, password) {
                    Ok(signer) => Ok(WalletSigner::Local(signer)),
                    Err(e) => match e {
                        // Catch the `MacMismatch` error, which indicates an incorrect password and
                        // return a more user-friendly `IncorrectKeystorePassword`.
                        alloy_signer_local::LocalSignerError::EthKeystoreError(
                            eth_keystore::KeystoreError::MacMismatch,
                        ) => Err(WalletSignerError::IncorrectKeystorePassword),
                        _ => Err(WalletSignerError::Local(e)),
                    },
                }
            }
            Self::Interactive => {
                let private_key = rpassword::prompt_password("Enter private key:")?;
                Ok(WalletSigner::from_private_key(&hex::FromHex::from_hex(private_key)?)?)
            }
        }
    }
}
