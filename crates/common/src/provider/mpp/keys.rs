//! Auto-discovery of MPP signing keys from the Tempo wallet.
//!
//! Uses the shared Tempo keystore types from [`crate::tempo`] and adds
//! MPP-specific primary key selection logic (passkey > first entry with
//! inline key > first entry, mirroring `Keystore::primary_key()` in
//! `tempo-common`).

use crate::tempo::{TEMPO_PRIVATE_KEY_ENV, WalletType, read_tempo_keys_file};
use alloy_primitives::Address;
use tracing::debug;

/// Discovered MPP key configuration.
///
/// Contains the private key and optional keychain metadata for signing mode
/// configuration.
#[derive(Debug, Clone)]
pub struct MppKeyConfig {
    /// The hex-encoded private key.
    pub key: String,
    /// Smart wallet address (for keychain signing mode).
    pub wallet_address: Option<Address>,
    /// Key address / signer address (for keychain authorized signer).
    pub key_address: Option<Address>,
    /// RLP-encoded signed key authorization (hex string).
    pub key_authorization: Option<String>,
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
            return Some(MppKeyConfig {
                key,
                wallet_address: None,
                key_address: None,
                key_authorization: None,
            });
        }
    }

    // 2. Read $TEMPO_HOME/wallet/keys.toml (default: ~/.tempo/wallet/keys.toml)
    let keys_file = read_tempo_keys_file()?;

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
            debug!("using MPP key from tempo wallet keys file");
            return Some(MppKeyConfig {
                key,
                wallet_address: Some(entry.wallet_address),
                key_address: entry.key_address,
                key_authorization: entry.key_authorization.clone(),
            });
        }
    }

    debug!("no usable key found in tempo keys file");
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tempo::KeysFile;
    use std::{io::Write, path::PathBuf};

    /// Write a keys.toml to a temp dir and set TEMPO_HOME to point at it.
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

        unsafe {
            std::env::set_var("TEMPO_HOME", dir.path());
            std::env::remove_var("TEMPO_PRIVATE_KEY");
        }

        let discovered = discover_mpp_key();
        assert_eq!(discovered.as_deref(), Some(key));

        unsafe { std::env::remove_var("TEMPO_HOME") };
    }

    #[test]
    fn discover_env_var_takes_priority_over_keys_toml() {
        let file_key = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        let env_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let toml_content = format!(
            r#"
[[keys]]
wallet_address = "0x0000000000000000000000000000000000000001"
key = "{file_key}"
"#
        );
        let (dir, _) = setup_keys_toml(&toml_content);

        unsafe {
            std::env::set_var("TEMPO_HOME", dir.path());
            std::env::set_var("TEMPO_PRIVATE_KEY", env_key);
        }

        let discovered = discover_mpp_key();
        assert_eq!(discovered.as_deref(), Some(env_key));

        unsafe {
            std::env::remove_var("TEMPO_HOME");
            std::env::remove_var("TEMPO_PRIVATE_KEY");
        }
    }

    #[test]
    fn discover_returns_none_when_no_keys() {
        let (dir, _) = setup_keys_toml("");

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
wallet_address = "0x0000000000000000000000000000000000000001"
chain_id = 4217

[[keys]]
wallet_address = "0x0000000000000000000000000000000000000002"
key = "{key}"
chain_id = 4217
"#
        );
        let (dir, _) = setup_keys_toml(&toml_content);

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
wallet_address = "0x0000000000000000000000000000000000000001"
key_address = "0x0000000000000000000000000000000000000002"
chain_id = 4217

[[keys]]
wallet_address = "0x0000000000000000000000000000000000000003"
key_address = "0x0000000000000000000000000000000000000004"
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
wallet_address = "0x0000000000000000000000000000000000000001"
key = "0xpasskey_secret"
chain_id = 4217

[[keys]]
wallet_type = "local"
wallet_address = "0x0000000000000000000000000000000000000002"
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
wallet_address = "0x0000000000000000000000000000000000000001"
key = "0xlocal_key"

[[keys]]
wallet_type = "passkey"
wallet_address = "0x0000000000000000000000000000000000000002"
key = "0xpasskey_key"
"#;
        let keys_file: KeysFile = toml::from_str(toml_str).unwrap();
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
wallet_address = "0x0000000000000000000000000000000000000001"

[[keys]]
wallet_address = "0x0000000000000000000000000000000000000002"
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
wallet_address = "0x0000000000000000000000000000000000000001"
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
