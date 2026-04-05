//! Tempo wallet keystore types and discovery helpers.
//!
//! Shared types for reading keys from the Tempo CLI wallet keystore
//! (`$TEMPO_HOME/wallet/keys.toml`, defaulting to `~/.tempo/wallet/keys.toml`).

use alloy_primitives::{Address, hex};
use alloy_rlp::Decodable;
use serde::Deserialize;
use std::path::PathBuf;

/// Environment variable for an ephemeral Tempo private key.
pub const TEMPO_PRIVATE_KEY_ENV: &str = "TEMPO_PRIVATE_KEY";

/// Environment variable to override the Tempo home directory.
pub const TEMPO_HOME_ENV: &str = "TEMPO_HOME";

/// Default Tempo home directory relative to the user's home.
pub const DEFAULT_TEMPO_HOME: &str = ".tempo";

/// Relative path from Tempo home to the wallet keys file.
pub const WALLET_KEYS_PATH: &str = "wallet/keys.toml";

/// Wallet type matching `tempo-common`'s `WalletType` enum.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WalletType {
    #[default]
    Local,
    Passkey,
}

/// Cryptographic key type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyType {
    #[default]
    Secp256k1,
    P256,
    WebAuthn,
}

/// Per-token spending limit stored in `keys.toml`.
#[derive(Debug, Default, Deserialize)]
pub struct StoredTokenLimit {
    pub currency: Address,
    pub limit: String,
}

/// A single key entry in `keys.toml`.
///
/// Mirrors the fields from `tempo-common::keys::model::KeyEntry`.
/// Unknown fields are ignored by serde.
#[derive(Debug, Default, Deserialize)]
pub struct KeyEntry {
    /// Wallet type: "local" or "passkey".
    #[serde(default)]
    pub wallet_type: WalletType,
    /// Smart wallet address (the on-chain account).
    #[serde(default)]
    pub wallet_address: Address,
    /// Chain ID.
    #[serde(default)]
    pub chain_id: u64,
    /// Cryptographic key type.
    #[serde(default)]
    pub key_type: KeyType,
    /// Key address (the EOA derived from the private key).
    #[serde(default)]
    pub key_address: Option<Address>,
    /// Key private key, stored inline in keys.toml.
    #[serde(default)]
    pub key: Option<String>,
    /// RLP-encoded signed key authorization (hex string).
    /// Used in keychain mode to atomically provision the access key on-chain.
    #[serde(default)]
    pub key_authorization: Option<String>,
    /// Expiry timestamp.
    #[serde(default)]
    pub expiry: Option<u64>,
    /// Per-token spending limits.
    #[serde(default)]
    pub limits: Vec<StoredTokenLimit>,
}

impl KeyEntry {
    /// Whether this entry has a non-empty inline private key.
    pub fn has_inline_key(&self) -> bool {
        self.key.as_ref().is_some_and(|k| !k.trim().is_empty())
    }
}

/// The top-level structure of `keys.toml`.
#[derive(Debug, Default, Deserialize)]
pub struct KeysFile {
    #[serde(default)]
    pub keys: Vec<KeyEntry>,
}

/// Resolve the Tempo home directory.
///
/// Uses `TEMPO_HOME` env var if set, otherwise `~/.tempo`.
pub fn tempo_home() -> Option<PathBuf> {
    if let Ok(home) = std::env::var(TEMPO_HOME_ENV) {
        return Some(PathBuf::from(home));
    }
    dirs::home_dir().map(|h| h.join(DEFAULT_TEMPO_HOME))
}

/// Returns the path to the Tempo wallet keys file.
pub fn tempo_keys_path() -> Option<PathBuf> {
    tempo_home().map(|home| home.join(WALLET_KEYS_PATH))
}

/// Read and parse the Tempo wallet keys file.
///
/// Returns `None` if the file doesn't exist or can't be read/parsed.
/// Errors are logged as warnings.
pub fn read_tempo_keys_file() -> Option<KeysFile> {
    let keys_path = tempo_keys_path()?;
    if !keys_path.exists() {
        tracing::trace!(?keys_path, "tempo keys file not found");
        return None;
    }

    let contents = match std::fs::read_to_string(&keys_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(?keys_path, %e, "failed to read tempo keys file");
            return None;
        }
    };

    match toml::from_str(&contents) {
        Ok(f) => Some(f),
        Err(e) => {
            tracing::warn!(?keys_path, %e, "failed to parse tempo keys file");
            None
        }
    }
}

/// Decodes a hex-encoded, RLP-encoded key authorization.
///
/// The input should be a hex string (with or without 0x prefix) containing
/// RLP-encoded `SignedKeyAuthorization` data.
pub fn decode_key_authorization<T: Decodable>(hex_str: &str) -> eyre::Result<T> {
    let bytes = hex::decode(hex_str)?;
    let auth = T::decode(&mut bytes.as_slice())?;
    Ok(auth)
}
