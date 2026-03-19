//! Auto-discovery of MPP signing keys from the Tempo wallet.
//!
//! Key discovery chain (matches `tempoxyz/wallet` `tempo-common` behavior):
//! 1. `TEMPO_PRIVATE_KEY` env var → use directly (ephemeral, never touches disk)
//! 2. `$TEMPO_HOME/wallet/keys.toml` (default: `~/.tempo/wallet/keys.toml`) → read from disk
//!
//! Primary key selection (deterministic, mirrors `Keystore::primary_key()`):
//! - passkey entry > first entry with inline `key` > first entry Only entries with a non-empty
//!   inline `key` field are usable for signing.

use serde::Deserialize;
use std::path::PathBuf;
use tracing::debug;

/// Environment variable for an ephemeral Tempo private key.
const TEMPO_PRIVATE_KEY_ENV: &str = "TEMPO_PRIVATE_KEY";

/// Environment variable to override the Tempo home directory.
const TEMPO_HOME_ENV: &str = "TEMPO_HOME";

/// Default Tempo home directory relative to the user's home.
const DEFAULT_TEMPO_HOME: &str = ".tempo";

/// Relative path from Tempo home to the wallet keys file.
const WALLET_KEYS_PATH: &str = "wallet/keys.toml";

/// Wallet type matching `tempo-common`'s `WalletType` enum.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum WalletType {
    #[default]
    Local,
    Passkey,
}

/// A single key entry in `keys.toml`.
///
/// Mirrors the fields from `tempo-common::keys::model::KeyEntry` that are
/// relevant for key discovery. Unknown fields are ignored via `#[serde(default)]`.
#[derive(Debug, Default, Deserialize)]
struct KeyEntry {
    /// Wallet type: "local" or "passkey".
    #[serde(default)]
    wallet_type: WalletType,
    /// Key private key, stored inline in keys.toml.
    #[serde(default)]
    key: Option<String>,
    /// Smart wallet address (the on-chain account).
    #[serde(default)]
    wallet_address: Option<String>,
    /// Key address (the EOA derived from the private key).
    #[serde(default)]
    key_address: Option<String>,
}

/// Discovered MPP key configuration.
///
/// Contains the private key and optional keychain metadata for signing mode
/// configuration.
#[derive(Debug, Clone)]
pub struct MppKeyConfig {
    /// The hex-encoded private key.
    pub key: String,
    /// Smart wallet address (for keychain signing mode).
    pub wallet_address: Option<String>,
    /// Key address / signer address (for keychain authorized signer).
    pub key_address: Option<String>,
}

impl KeyEntry {
    /// Whether this entry has a non-empty inline private key.
    fn has_inline_key(&self) -> bool {
        self.key.as_ref().is_some_and(|k| !k.trim().is_empty())
    }
}

/// The top-level structure of `keys.toml`.
#[derive(Debug, Default, Deserialize)]
struct KeysFile {
    #[serde(default)]
    keys: Vec<KeyEntry>,
}

/// Attempt to auto-discover an MPP signing key from the Tempo wallet.
///
/// Returns `Some(hex_key)` if a key is found, `None` otherwise.
/// Never fails — discovery errors are silently ignored (logged at debug level).
pub fn discover_mpp_key() -> Option<String> {
    discover_mpp_config().map(|c| c.key)
}

/// Attempt to auto-discover MPP key configuration from the Tempo wallet.
///
/// Returns the private key along with optional wallet/key addresses needed for
/// keychain signing mode. Never fails — discovery errors are silently ignored.
pub fn discover_mpp_config() -> Option<MppKeyConfig> {
    // 1. Check TEMPO_PRIVATE_KEY env var (no keychain metadata available)
    if let Ok(key) = std::env::var(TEMPO_PRIVATE_KEY_ENV) {
        let key = key.trim().to_string();
        if !key.is_empty() {
            debug!("using MPP key from {TEMPO_PRIVATE_KEY_ENV} env var");
            return Some(MppKeyConfig { key, wallet_address: None, key_address: None });
        }
    }

    // 2. Read $TEMPO_HOME/wallet/keys.toml (default: ~/.tempo/wallet/keys.toml)
    let keys_path = tempo_keys_path()?;
    if !keys_path.exists() {
        debug!(?keys_path, "tempo keys file not found");
        return None;
    }

    let contents = match std::fs::read_to_string(&keys_path) {
        Ok(c) => c,
        Err(e) => {
            debug!(?keys_path, %e, "failed to read tempo keys file");
            return None;
        }
    };

    let keys_file: KeysFile = match toml::from_str(&contents) {
        Ok(f) => f,
        Err(e) => {
            debug!(?keys_path, %e, "failed to parse tempo keys file");
            return None;
        }
    };

    // Pick primary key using the same deterministic order as
    // `Keystore::primary_key()` in tempo-common:
    //   passkey > first entry with inline key > first entry
    // Only entries with a usable inline key can provide a signing key.
    let primary = keys_file
        .keys
        .iter()
        .find(|k| k.wallet_type == WalletType::Passkey)
        .or_else(|| keys_file.keys.iter().find(|k| k.has_inline_key()))
        .or(keys_file.keys.first());

    if let Some(entry) = primary
        && let Some(key) = &entry.key
    {
        let key = key.trim().to_string();
        if !key.is_empty() {
            debug!(?keys_path, "using MPP key from tempo wallet keys file");
            return Some(MppKeyConfig {
                key,
                wallet_address: entry.wallet_address.clone(),
                key_address: entry.key_address.clone(),
            });
        }
    }

    debug!(?keys_path, "no usable key found in tempo keys file");
    None
}

/// Resolve the Tempo home directory.
///
/// Uses `TEMPO_HOME` env var if set, otherwise `~/.tempo`.
fn tempo_home() -> Option<PathBuf> {
    if let Ok(home) = std::env::var(TEMPO_HOME_ENV) {
        return Some(PathBuf::from(home));
    }
    dirs::home_dir().map(|h| h.join(DEFAULT_TEMPO_HOME))
}

/// Returns the path to the Tempo wallet keys file.
fn tempo_keys_path() -> Option<PathBuf> {
    tempo_home().map(|home| home.join(WALLET_KEYS_PATH))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write a keys.toml to a temp dir and set TEMPO_HOME to point at it.
    /// Returns the tempdir (must be kept alive for the duration of the test)
    /// and the private key that was written.
    fn setup_keys_toml(toml_content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let wallet_dir = dir.path().join("wallet");
        std::fs::create_dir_all(&wallet_dir).expect("create wallet dir");
        let keys_path = wallet_dir.join("keys.toml");
        let mut f = std::fs::File::create(&keys_path).expect("create keys.toml");
        f.write_all(toml_content.as_bytes()).expect("write keys.toml");
        (dir, keys_path)
    }

    #[test]
    fn discover_from_tempo_home_keys_toml() {
        let key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let toml_content = format!(
            r#"
[[keys]]
wallet_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
key_address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
key = "{key}"
chain_id = 4217
"#
        );
        let (dir, _) = setup_keys_toml(&toml_content);

        // Point TEMPO_HOME at the temp dir so discover_mpp_key reads our file.
        // Clear TEMPO_PRIVATE_KEY so it doesn't short-circuit.
        // SAFETY: test-only env manipulation.
        unsafe {
            std::env::set_var("TEMPO_HOME", dir.path());
            std::env::remove_var("TEMPO_PRIVATE_KEY");
        }

        let discovered = discover_mpp_key();
        assert_eq!(discovered.as_deref(), Some(key));

        // Cleanup
        unsafe { std::env::remove_var("TEMPO_HOME") };
    }

    #[test]
    fn discover_env_var_takes_priority_over_keys_toml() {
        let file_key = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        let env_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let toml_content = format!(
            r#"
[[keys]]
wallet_address = "0xAAAA"
key = "{file_key}"
"#
        );
        let (dir, _) = setup_keys_toml(&toml_content);

        // SAFETY: test-only env manipulation.
        unsafe {
            std::env::set_var("TEMPO_HOME", dir.path());
            std::env::set_var("TEMPO_PRIVATE_KEY", env_key);
        }

        let discovered = discover_mpp_key();
        assert_eq!(discovered.as_deref(), Some(env_key));

        // Cleanup
        unsafe {
            std::env::remove_var("TEMPO_HOME");
            std::env::remove_var("TEMPO_PRIVATE_KEY");
        }
    }

    #[test]
    fn discover_returns_none_when_no_keys() {
        let (dir, _) = setup_keys_toml(""); // empty file, no [[keys]]

        // SAFETY: test-only env manipulation.
        unsafe {
            std::env::set_var("TEMPO_HOME", dir.path());
            std::env::remove_var("TEMPO_PRIVATE_KEY");
        }

        let discovered = discover_mpp_key();
        assert!(discovered.is_none());

        unsafe { std::env::remove_var("TEMPO_HOME") };
    }

    #[test]
    fn discover_skips_entries_without_inline_key() {
        let key = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let toml_content = format!(
            r#"
[[keys]]
wallet_address = "0xNoKey"
chain_id = 4217

[[keys]]
wallet_address = "0xHasKey"
key = "{key}"
chain_id = 4217
"#
        );
        let (dir, _) = setup_keys_toml(&toml_content);

        // SAFETY: test-only env manipulation.
        unsafe {
            std::env::set_var("TEMPO_HOME", dir.path());
            std::env::remove_var("TEMPO_PRIVATE_KEY");
        }

        let discovered = discover_mpp_key();
        assert_eq!(discovered.as_deref(), Some(key));

        unsafe { std::env::remove_var("TEMPO_HOME") };
    }

    #[test]
    fn parse_keys_toml() {
        let toml_str = r#"
[[keys]]
wallet_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
key_address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
chain_id = 4217
"#;
        let keys_file: KeysFile = toml::from_str(toml_str).unwrap();
        assert_eq!(keys_file.keys.len(), 1);
        assert_eq!(
            keys_file.keys[0].key.as_deref(),
            Some("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
        );
    }

    #[test]
    fn parse_keys_toml_no_inline_key() {
        let toml_str = r#"
[[keys]]
wallet_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
key_address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
chain_id = 4217
"#;
        let keys_file: KeysFile = toml::from_str(toml_str).unwrap();
        assert_eq!(keys_file.keys.len(), 1);
        assert!(keys_file.keys[0].key.is_none());
    }

    #[test]
    fn parse_keys_toml_multiple_entries() {
        let toml_str = r#"
[[keys]]
wallet_address = "0xAAAA"
key_address = "0xBBBB"
chain_id = 4217

[[keys]]
wallet_address = "0xCCCC"
key_address = "0xDDDD"
key = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
chain_id = 4217
"#;
        let keys_file: KeysFile = toml::from_str(toml_str).unwrap();
        assert_eq!(keys_file.keys.len(), 2);
        assert!(keys_file.keys[0].key.is_none());
        assert!(keys_file.keys[1].key.is_some());
    }

    #[test]
    fn parse_keys_toml_with_wallet_type() {
        let toml_str = r#"
[[keys]]
wallet_type = "passkey"
wallet_address = "0xAAAA"
key = "0xpasskey_secret"
chain_id = 4217

[[keys]]
wallet_type = "local"
wallet_address = "0xBBBB"
key = "0xlocal_secret"
chain_id = 4217
"#;
        let keys_file: KeysFile = toml::from_str(toml_str).unwrap();
        assert_eq!(keys_file.keys.len(), 2);
        assert_eq!(keys_file.keys[0].wallet_type, WalletType::Passkey);
        assert_eq!(keys_file.keys[1].wallet_type, WalletType::Local);
    }

    #[test]
    fn primary_key_passkey_wins() {
        let toml_str = r#"
[[keys]]
wallet_type = "local"
wallet_address = "0xLocal"
key = "0xlocal_key"

[[keys]]
wallet_type = "passkey"
wallet_address = "0xPasskey"
key = "0xpasskey_key"
"#;
        let keys_file: KeysFile = toml::from_str(toml_str).unwrap();
        // Passkey should be selected as primary
        let primary = keys_file
            .keys
            .iter()
            .find(|k| k.wallet_type == WalletType::Passkey)
            .or_else(|| keys_file.keys.iter().find(|k| k.has_inline_key()))
            .or(keys_file.keys.first());
        assert_eq!(primary.unwrap().key.as_deref(), Some("0xpasskey_key"));
    }

    #[test]
    fn primary_key_inline_key_over_no_key() {
        let toml_str = r#"
[[keys]]
wallet_address = "0xNoKey"

[[keys]]
wallet_address = "0xHasKey"
key = "0xthe_key"
"#;
        let keys_file: KeysFile = toml::from_str(toml_str).unwrap();
        let primary = keys_file
            .keys
            .iter()
            .find(|k| k.wallet_type == WalletType::Passkey)
            .or_else(|| keys_file.keys.iter().find(|k| k.has_inline_key()))
            .or(keys_file.keys.first());
        assert_eq!(primary.unwrap().key.as_deref(), Some("0xthe_key"));
    }

    #[test]
    fn parse_keys_toml_unknown_fields_ignored() {
        let toml_str = r#"
[[keys]]
wallet_address = "0xAAAA"
key = "0xsecret"
chain_id = 4217
key_authorization = "0xauth_data"
expiry = 1750000000
unknown_future_field = "should be ignored"

[[keys.limits]]
currency = "0x20c000000000000000000000b9537d11c60e8b50"
limit = "1000"
"#;
        let keys_file: KeysFile = toml::from_str(toml_str).unwrap();
        assert_eq!(keys_file.keys.len(), 1);
        assert_eq!(keys_file.keys[0].key.as_deref(), Some("0xsecret"));
    }
}
