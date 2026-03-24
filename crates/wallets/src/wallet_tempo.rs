//! Tempo wallet keystore integration.
//!
//! Resolves a signer by matching the `--from` address against entries in the
//! Tempo CLI wallet keystore. Types and discovery helpers are defined in
//! [`foundry_common::tempo`].

use alloy_primitives::Address;
use alloy_signer::Signer;
use eyre::Result;
use foundry_common::tempo::{KeysFile, TEMPO_PRIVATE_KEY_ENV, read_tempo_keys_file};
use std::env;

use crate::{WalletSigner, utils::create_private_key_signer};

/// Try to resolve a signer from the Tempo wallet keystore for the given address.
///
/// First checks `TEMPO_PRIVATE_KEY` env var, then reads
/// `$TEMPO_HOME/wallet/keys.toml` (default `~/.tempo/wallet/keys.toml`) and looks
/// for a key entry whose `wallet_address` or `key_address` matches `sender`. If a match
/// with an inline private key is found, returns the corresponding [`WalletSigner`].
///
/// Read/parse errors are treated as warnings and skipped rather than
/// failing the signer resolution.
pub fn try_resolve_tempo_signer(sender: Address) -> Result<Option<WalletSigner>> {
    // 1. Check TEMPO_PRIVATE_KEY env var
    if let Ok(key) = env::var(TEMPO_PRIVATE_KEY_ENV) {
        let key = key.trim().to_string();
        if !key.is_empty() {
            trace!("checking TEMPO_PRIVATE_KEY env var");
            let signer = create_private_key_signer(&key)?;
            if signer.address() == sender {
                trace!("using signer from TEMPO_PRIVATE_KEY env var");
                return Ok(Some(signer));
            }
            trace!(
                derived = %signer.address(),
                requested = %sender,
                "TEMPO_PRIVATE_KEY address does not match requested sender"
            );
        }
    }

    // 2. Read $TEMPO_HOME/wallet/keys.toml
    let Some(keys_file) = read_tempo_keys_file() else {
        return Ok(None);
    };

    resolve_from_keys_file(&keys_file, sender)
}

/// Resolve a signer from a parsed [`KeysFile`].
fn resolve_from_keys_file(keys_file: &KeysFile, sender: Address) -> Result<Option<WalletSigner>> {
    let sender_lower = format!("{sender:#x}");

    for entry in &keys_file.keys {
        let wallet_match =
            entry.wallet_address.as_deref().is_some_and(|addr| addr.to_lowercase() == sender_lower);
        let key_match =
            entry.key_address.as_deref().is_some_and(|addr| addr.to_lowercase() == sender_lower);

        if (wallet_match || key_match) && entry.has_inline_key() {
            let signer = create_private_key_signer(entry.key.as_ref().unwrap())?;
            // Only return the signer if the derived address matches the requested sender.
            // This prevents returning a signer for a different address (e.g. when
            // wallet_address != key_address in keychain-style entries).
            if signer.address() == sender {
                trace!("found matching key in tempo keystore");
                return Ok(Some(signer));
            }
            trace!(
                derived = %signer.address(),
                requested = %sender,
                "tempo keystore entry matched but derived address differs, skipping"
            );
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const TEST_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const TEST_ADDR: &str = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";

    fn resolve_from_toml(contents: &str, sender: Address) -> Result<Option<WalletSigner>> {
        let keys_file: KeysFile = toml::from_str(contents)?;
        resolve_from_keys_file(&keys_file, sender)
    }

    /// Write a keys.toml to a temp dir and set TEMPO_HOME to point at it.
    /// Returns the tempdir (must be kept alive for the duration of the test).
    fn setup_keys_toml(toml_content: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let wallet_dir = dir.path().join("wallet");
        std::fs::create_dir_all(&wallet_dir).expect("create wallet dir");
        let keys_path = wallet_dir.join("keys.toml");
        let mut f = std::fs::File::create(&keys_path).expect("create keys.toml");
        f.write_all(toml_content.as_bytes()).expect("write keys.toml");
        dir
    }

    #[test]
    fn resolves_by_wallet_address() {
        let toml = format!(
            r#"[[keys]]
wallet_address = "{TEST_ADDR}"
key = "{TEST_KEY}"
"#
        );
        let sender: Address = TEST_ADDR.parse().unwrap();
        let signer = resolve_from_toml(&toml, sender).unwrap().unwrap();
        assert_eq!(signer.address(), sender);
    }

    #[test]
    fn resolves_by_key_address() {
        let toml = format!(
            r#"[[keys]]
wallet_address = "0x0000000000000000000000000000000000000001"
key_address = "{TEST_ADDR}"
key = "{TEST_KEY}"
"#
        );
        let sender: Address = TEST_ADDR.parse().unwrap();
        let signer = resolve_from_toml(&toml, sender).unwrap().unwrap();
        assert_eq!(signer.address(), sender);
    }

    #[test]
    fn returns_none_when_no_match() {
        let toml = format!(
            r#"[[keys]]
wallet_address = "0x1111111111111111111111111111111111111111"
key = "{TEST_KEY}"
"#
        );
        let sender: Address = "0x2222222222222222222222222222222222222222".parse().unwrap();
        assert!(resolve_from_toml(&toml, sender).unwrap().is_none());
    }

    #[test]
    fn skips_entry_without_key() {
        let toml = format!(
            r#"[[keys]]
wallet_type = "passkey"
wallet_address = "{TEST_ADDR}"
key_type = "webauthn"
"#
        );
        let sender: Address = TEST_ADDR.parse().unwrap();
        assert!(resolve_from_toml(&toml, sender).unwrap().is_none());
    }

    #[test]
    fn case_insensitive_match() {
        let toml = format!(
            r#"[[keys]]
wallet_address = "0xF39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
key = "{TEST_KEY}"
"#
        );
        let sender: Address = TEST_ADDR.parse().unwrap();
        assert!(resolve_from_toml(&toml, sender).unwrap().is_some());
    }

    #[test]
    fn empty_keystore() {
        let sender: Address = TEST_ADDR.parse().unwrap();
        assert!(resolve_from_toml("", sender).unwrap().is_none());
    }

    #[test]
    fn skips_when_wallet_address_differs_from_derived_key() {
        // wallet_address matches sender, but the private key derives to a different address.
        // This simulates a keychain-style entry where wallet_address != key_address.
        let other_addr = "0x0000000000000000000000000000000000000042";
        let toml = format!(
            r#"[[keys]]
wallet_address = "{other_addr}"
key_address = "{TEST_ADDR}"
key = "{TEST_KEY}"
"#
        );
        // Looking up by wallet_address (0x42) — the key derives to TEST_ADDR, not 0x42.
        let sender: Address = other_addr.parse().unwrap();
        assert!(resolve_from_toml(&toml, sender).unwrap().is_none());
    }

    #[test]
    fn parse_keys_toml_unknown_fields_ignored() {
        let toml_str = r#"
[[keys]]
wallet_address = "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
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

    /// Integration tests that exercise the full discovery chain via env vars.
    /// Combined into a single test to avoid parallel env var mutation.
    #[test]
    fn discover_from_tempo_home() {
        let sender: Address = TEST_ADDR.parse().unwrap();

        // 1. Resolves from keys.toml via TEMPO_HOME
        {
            let toml_content = format!(
                r#"
[[keys]]
wallet_address = "{TEST_ADDR}"
key = "{TEST_KEY}"
chain_id = 4217
"#
            );
            let dir = setup_keys_toml(&toml_content);

            // SAFETY: test-only env manipulation, single-threaded within this test.
            unsafe {
                std::env::set_var("TEMPO_HOME", dir.path());
                std::env::remove_var("TEMPO_PRIVATE_KEY");
            }

            let signer = try_resolve_tempo_signer(sender).unwrap().unwrap();
            assert_eq!(signer.address(), sender);
        }

        // 2. TEMPO_PRIVATE_KEY env var takes priority over keys.toml
        {
            let other_key = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
            let other_addr: Address = "0x70997970c51812dc3a010c7d01b50e0d17dc79c8".parse().unwrap();
            let toml_content = format!(
                r#"
[[keys]]
wallet_address = "{TEST_ADDR}"
key = "{TEST_KEY}"
"#
            );
            let dir = setup_keys_toml(&toml_content);

            // SAFETY: test-only env manipulation, single-threaded within this test.
            unsafe {
                std::env::set_var("TEMPO_HOME", dir.path());
                std::env::set_var("TEMPO_PRIVATE_KEY", other_key);
            }

            // The env var key derives to other_addr, so looking up other_addr should
            // use the env var key (priority over keys.toml).
            let signer = try_resolve_tempo_signer(other_addr).unwrap().unwrap();
            assert_eq!(signer.address(), other_addr);
        }

        // 3. Skips entries without an inline key
        {
            let toml_content = format!(
                r#"
[[keys]]
wallet_address = "{TEST_ADDR}"
chain_id = 4217
"#
            );
            let dir = setup_keys_toml(&toml_content);

            // SAFETY: test-only env manipulation, single-threaded within this test.
            unsafe {
                std::env::set_var("TEMPO_HOME", dir.path());
                std::env::remove_var("TEMPO_PRIVATE_KEY");
            }

            assert!(try_resolve_tempo_signer(sender).unwrap().is_none());
        }

        // Cleanup
        unsafe {
            std::env::remove_var("TEMPO_HOME");
            std::env::remove_var("TEMPO_PRIVATE_KEY");
        }
    }
}
