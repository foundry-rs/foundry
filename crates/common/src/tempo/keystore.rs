//! Tempo wallet keystore types and discovery helpers.
//!
//! Shared types for reading keys from the Tempo CLI wallet keystore
//! (`$TEMPO_HOME/wallet/keys.toml`, defaulting to `~/.tempo/wallet/keys.toml`).

use alloy_primitives::{Address, hex};
use alloy_rlp::Decodable;
use serde::{Deserialize, Serialize};
use std::{env, fs, io::Write, path::PathBuf};

/// Environment variable for an ephemeral Tempo private key.
pub const TEMPO_PRIVATE_KEY_ENV: &str = "TEMPO_PRIVATE_KEY";

/// Environment variable to override the Tempo home directory.
pub const TEMPO_HOME_ENV: &str = "TEMPO_HOME";

/// Default Tempo home directory relative to the user's home.
pub const DEFAULT_TEMPO_HOME: &str = ".tempo";

/// Relative path from Tempo home to the wallet keys file.
pub const WALLET_KEYS_PATH: &str = "wallet/keys.toml";

/// Wallet type matching `tempo-common`'s `WalletType` enum.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WalletType {
    #[default]
    Local,
    Passkey,
}

/// Cryptographic key type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyType {
    #[default]
    Secp256k1,
    P256,
    WebAuthn,
}

/// Per-token spending limit stored in `keys.toml`.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct StoredTokenLimit {
    pub currency: Address,
    pub limit: String,
}

/// A single key entry in `keys.toml`.
///
/// Mirrors the fields from `tempo-common::keys::model::KeyEntry`.
/// Unknown fields are ignored by serde.
#[derive(Debug, Default, Deserialize, Serialize)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_address: Option<Address>,
    /// Key private key, stored inline in keys.toml.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// RLP-encoded signed key authorization (hex string).
    /// Used in keychain mode to atomically provision the access key on-chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_authorization: Option<String>,
    /// Expiry timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry: Option<u64>,
    /// Per-token spending limits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<StoredTokenLimit>,
}

impl KeyEntry {
    /// Whether this entry has a non-empty inline private key.
    pub fn has_inline_key(&self) -> bool {
        self.key.as_ref().is_some_and(|k| !k.trim().is_empty())
    }
}

/// The top-level structure of `keys.toml`.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct KeysFile {
    #[serde(default)]
    pub keys: Vec<KeyEntry>,
}

/// Process-wide mutex used by tests that mutate `TEMPO_HOME`.
///
/// Returns a [`tokio::sync::Mutex`] so async tests can hold it across `.await`
/// points without tripping `clippy::await_holding_lock`.
#[cfg(test)]
pub(crate) fn test_env_mutex() -> &'static tokio::sync::Mutex<()> {
    static M: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    M.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Resolve the Tempo home directory.
///
/// Uses `TEMPO_HOME` env var if set, otherwise `~/.tempo`.
pub fn tempo_home() -> Option<PathBuf> {
    if let Ok(home) = env::var(TEMPO_HOME_ENV) {
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

    let contents = match fs::read_to_string(&keys_path) {
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

/// Atomically upsert a [`KeyEntry`] into `keys.toml`.
///
/// Replaces any existing entry for the same `(wallet_address, chain_id)`.
/// Each Tempo wallet has at most one active access key per chain, so a fresh
/// login always supersedes the previous entry regardless of the new key
/// address. Creates the file (and parent directories) if missing. Writes via
/// temp file + rename so a crash mid-write cannot corrupt the file.
pub(crate) fn upsert_key_entry(entry: KeyEntry) -> eyre::Result<()> {
    let path = tempo_keys_path().ok_or_else(|| eyre::eyre!("could not resolve tempo home"))?;
    let dir = path.parent().ok_or_else(|| eyre::eyre!("invalid keys path: {}", path.display()))?;
    fs::create_dir_all(dir)?;

    let mut file = read_tempo_keys_file().unwrap_or_default();
    file.keys
        .retain(|k| !(k.wallet_address == entry.wallet_address && k.chain_id == entry.chain_id));
    file.keys.push(entry);

    let body = toml::to_string_pretty(&file)?;
    let contents = format!(
        "# Tempo wallet keys — managed by Foundry / Tempo CLI.\n# Do not edit manually.\n\n{body}"
    );

    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(contents.as_bytes())?;
    tmp.flush()?;
    tmp.persist(&path).map_err(|e| eyre::eyre!("failed to persist keys.toml: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn with_tempo_home<F: FnOnce()>(f: F) {
        let tmp = tempfile::tempdir().unwrap();
        // SAFETY: process-global env access is serialized via the shared mutex.
        let _g = test_env_mutex().blocking_lock();
        unsafe { std::env::set_var(TEMPO_HOME_ENV, tmp.path()) };
        f();
        unsafe { std::env::remove_var(TEMPO_HOME_ENV) };
    }

    #[test]
    fn upsert_replaces_matching_entry_atomically() {
        with_tempo_home(|| {
            let wallet = Address::from_str("0x0000000000000000000000000000000000000001").unwrap();
            let key = Address::from_str("0x0000000000000000000000000000000000000abc").unwrap();

            let mk = |expiry: u64| KeyEntry {
                wallet_type: WalletType::Passkey,
                wallet_address: wallet,
                chain_id: 4217,
                key_type: KeyType::Secp256k1,
                key_address: Some(key),
                key: Some("0xdead".to_string()),
                key_authorization: Some("0xbeef".to_string()),
                expiry: Some(expiry),
                limits: vec![],
            };

            upsert_key_entry(mk(100)).unwrap();
            upsert_key_entry(mk(200)).unwrap();

            let file = read_tempo_keys_file().unwrap();
            assert_eq!(file.keys.len(), 1);
            assert_eq!(file.keys[0].expiry, Some(200));

            // Different chain_id => separate entry.
            let mut other = mk(300);
            other.chain_id = 42431;
            upsert_key_entry(other).unwrap();
            let file = read_tempo_keys_file().unwrap();
            assert_eq!(file.keys.len(), 2);
        });
    }

    #[test]
    fn upsert_replaces_when_key_address_changes() {
        // Re-login produces a fresh random key address; the new entry must
        // supersede the old one for the same (wallet, chain), not coexist.
        with_tempo_home(|| {
            let wallet = Address::from_str("0x0000000000000000000000000000000000000001").unwrap();
            let old_key = Address::from_str("0x000000000000000000000000000000000000aaaa").unwrap();
            let new_key = Address::from_str("0x000000000000000000000000000000000000bbbb").unwrap();

            let mk = |key_addr: Address| KeyEntry {
                wallet_type: WalletType::Passkey,
                wallet_address: wallet,
                chain_id: 4217,
                key_type: KeyType::Secp256k1,
                key_address: Some(key_addr),
                key: Some("0xdead".to_string()),
                key_authorization: Some("0xbeef".to_string()),
                expiry: Some(100),
                limits: vec![],
            };

            upsert_key_entry(mk(old_key)).unwrap();
            upsert_key_entry(mk(new_key)).unwrap();

            let file = read_tempo_keys_file().unwrap();
            assert_eq!(file.keys.len(), 1, "old entry must be replaced, not duplicated");
            assert_eq!(file.keys[0].key_address, Some(new_key));
        });
    }
}
