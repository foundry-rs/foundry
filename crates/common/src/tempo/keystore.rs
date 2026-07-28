//! Tempo Accounts store discovery and local key metadata.

use alloy_primitives::{Address, hex};
use alloy_rlp::Decodable;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tempo_alloy::accounts::{TempoAccountsStore, default_accounts_store_path};
use tempo_primitives::{
    SignatureType,
    transaction::{SignedKeyAuthorization, TokenLimit},
};

/// Environment variable to override the Tempo home directory.
pub const TEMPO_HOME_ENV: &str = "TEMPO_HOME";

/// Cryptographic key type used by Tempo Accounts access keys.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyType {
    #[default]
    Secp256k1,
    P256,
    WebAuthn,
}

impl From<SignatureType> for KeyType {
    fn from(value: SignatureType) -> Self {
        match value {
            SignatureType::Secp256k1 => Self::Secp256k1,
            SignatureType::P256 => Self::P256,
            SignatureType::WebAuthn => Self::WebAuthn,
        }
    }
}

/// Per-token spending limit exposed by the local keychain commands.
#[derive(Debug, Clone)]
pub struct StoredTokenLimit {
    pub currency: Address,
    pub limit: String,
    pub period: u64,
}

impl From<&TokenLimit> for StoredTokenLimit {
    fn from(value: &TokenLimit) -> Self {
        Self { currency: value.token, limit: value.limit.to_string(), period: value.period }
    }
}

/// Non-secret view of one access key in the Tempo Accounts store.
#[derive(Clone, Default)]
pub struct KeyEntry {
    pub wallet_address: Address,
    pub chain_id: u64,
    pub key_type: KeyType,
    pub key_address: Address,
    pub key_authorization: Option<SignedKeyAuthorization>,
    pub expiry: Option<u64>,
    pub limits: Vec<StoredTokenLimit>,
    locally_signable: bool,
}

impl std::fmt::Debug for KeyEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyEntry")
            .field("wallet_address", &self.wallet_address)
            .field("chain_id", &self.chain_id)
            .field("key_type", &self.key_type)
            .field("key_address", &self.key_address)
            .field("has_key", &self.locally_signable)
            .field("has_key_authorization", &self.key_authorization.is_some())
            .field("expiry", &self.expiry)
            .field("limits", &self.limits)
            .finish()
    }
}

impl KeyEntry {
    /// Construct non-secret metadata for one Tempo Accounts access key.
    pub const fn new(
        wallet_address: Address,
        chain_id: u64,
        key_type: KeyType,
        key_address: Address,
    ) -> Self {
        Self {
            wallet_address,
            chain_id,
            key_type,
            key_address,
            key_authorization: None,
            expiry: None,
            limits: Vec::new(),
            locally_signable: false,
        }
    }

    /// Attach a pending authorization to this metadata view.
    #[doc(hidden)]
    pub fn with_key_authorization(mut self, authorization: SignedKeyAuthorization) -> Self {
        self.key_authorization = Some(authorization);
        self
    }

    /// Whether the Accounts store contains usable local signing material.
    pub const fn has_inline_key(&self) -> bool {
        self.locally_signable
    }

    /// Override local signing availability when constructing diagnostic fixtures.
    #[doc(hidden)]
    pub const fn with_locally_signable(mut self, locally_signable: bool) -> Self {
        self.locally_signable = locally_signable;
        self
    }
}

/// Snapshot used by the existing keychain list/show/doctor commands.
#[derive(Debug, Default)]
pub struct AccountsStoreView {
    pub keys: Vec<KeyEntry>,
}

/// Return the canonical Tempo Accounts `store.json` path.
pub fn tempo_accounts_store_path() -> Option<PathBuf> {
    default_accounts_store_path().ok()
}

/// Read non-secret metadata from the canonical Tempo Accounts store.
pub fn read_tempo_accounts_store() -> Option<AccountsStoreView> {
    let store = match TempoAccountsStore::try_open_default() {
        Ok(Some(store)) => store,
        Ok(None) => return None,
        Err(error) => {
            tracing::warn!(%error, "failed to open Tempo Accounts store");
            return None;
        }
    };
    let keys = match store.access_keys() {
        Ok(keys) => keys,
        Err(error) => {
            tracing::warn!(%error, path = %store.path().display(), "failed to read Tempo Accounts access keys");
            return None;
        }
    };
    Some(AccountsStoreView {
        keys: keys
            .into_iter()
            .map(|key| KeyEntry {
                wallet_address: key.account(),
                chain_id: key.chain_id(),
                key_type: key.key_type().into(),
                key_address: key.address(),
                key_authorization: key.key_authorization().cloned(),
                expiry: key.expiry(),
                limits: key.limits().iter().map(Into::into).collect(),
                locally_signable: key.is_locally_signable(),
            })
            .collect(),
    })
}

/// Decode an RLP key authorization stored by Tempo Accounts.
pub fn decode_key_authorization<T: Decodable>(encoded: &str) -> eyre::Result<T> {
    let bytes = hex::decode(encoded)?;
    let mut bytes = bytes.as_slice();
    let authorization = T::decode(&mut bytes)?;
    if !bytes.is_empty() {
        eyre::bail!("key authorization has trailing bytes");
    }
    Ok(authorization)
}
