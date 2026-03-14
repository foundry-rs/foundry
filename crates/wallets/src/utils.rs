use crate::{PendingSigner, WalletSigner, error::PrivateKeyError};
use alloy_primitives::{Address, B256, hex::FromHex};
use alloy_signer_ledger::HDPath as LedgerHDPath;
use alloy_signer_local::PrivateKeySigner;
use alloy_signer_trezor::HDPath as TrezorHDPath;
use eyre::{Context, Result};
use foundry_config::Config;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn ensure_pk_not_env(pk: &str) -> Result<()> {
    if !pk.starts_with("0x") && std::env::var(pk).is_ok() {
        return Err(PrivateKeyError::ExistsAsEnvVar(pk.to_string()).into());
    }
    Ok(())
}

/// Validates and sanitizes user inputs, returning configured [WalletSigner].
pub fn create_private_key_signer(private_key_str: &str) -> Result<WalletSigner> {
    let Ok(private_key) = B256::from_hex(private_key_str) else {
        ensure_pk_not_env(private_key_str)?;
        eyre::bail!("Failed to decode private key")
    };
    match PrivateKeySigner::from_bytes(&private_key) {
        Ok(pk) => Ok(WalletSigner::Local(pk)),
        Err(err) => {
            ensure_pk_not_env(private_key_str)?;
            eyre::bail!("Failed to create wallet from private key: {err}")
        }
    }
}

/// Creates [WalletSigner] instance from given mnemonic parameters.
///
/// Mnemonic can be either a file path or a mnemonic phrase.
pub fn create_mnemonic_signer(
    mnemonic: &str,
    passphrase: Option<&str>,
    hd_path: Option<&str>,
    index: u32,
) -> Result<WalletSigner> {
    let mnemonic = if Path::new(mnemonic).is_file() {
        fs::read_to_string(mnemonic)?
    } else {
        mnemonic.to_owned()
    };
    let mnemonic = mnemonic.split_whitespace().collect::<Vec<_>>().join(" ");

    Ok(WalletSigner::from_mnemonic(&mnemonic, passphrase, hd_path, index)?)
}

/// Creates [WalletSigner] instance from given Ledger parameters.
pub async fn create_ledger_signer(
    hd_path: Option<&str>,
    mnemonic_index: u32,
) -> Result<WalletSigner> {
    let derivation = if let Some(hd_path) = hd_path {
        LedgerHDPath::Other(hd_path.to_owned())
    } else {
        LedgerHDPath::LedgerLive(mnemonic_index as usize)
    };

    WalletSigner::from_ledger_path(derivation).await.wrap_err_with(|| {
        "\
Could not connect to Ledger device.
Make sure it's connected and unlocked, with no other desktop wallet apps open."
    })
}

/// Creates [WalletSigner] instance from given Trezor parameters.
pub async fn create_trezor_signer(
    hd_path: Option<&str>,
    mnemonic_index: u32,
) -> Result<WalletSigner> {
    let derivation = if let Some(hd_path) = hd_path {
        TrezorHDPath::Other(hd_path.to_owned())
    } else {
        TrezorHDPath::TrezorLive(mnemonic_index as usize)
    };

    WalletSigner::from_trezor_path(derivation).await.wrap_err_with(|| {
        "\
Could not connect to Trezor device.
Make sure it's connected and unlocked, with no other conflicting desktop wallet apps open."
    })
}

pub fn maybe_get_keystore_path(
    maybe_path: Option<&str>,
    maybe_name: Option<&str>,
) -> Result<Option<PathBuf>> {
    let default_keystore_dir = Config::foundry_keystores_dir()
        .ok_or_else(|| eyre::eyre!("Could not find the default keystore directory."))?;

    if let Some(path) = maybe_path {
        return Ok(Some(PathBuf::from(path)));
    }

    if let Some(name) = maybe_name {
        if let Some(found_path) = find_keystore_by_name(&default_keystore_dir, name) {
            return Ok(Some(found_path));
        }

        // Return the direct path even if it doesn't exist (for better error messages)
        return Ok(Some(default_keystore_dir.join(name)));
    }

    Ok(None)
}

/// Finds a keystore file by account name, supporting both old and new filename formats
/// Returns the path if found, or None if not found
pub fn find_keystore_by_name(
    keystore_dir: &std::path::Path,
    account_name: &str,
) -> Option<std::path::PathBuf> {
    // Try exact match first (old format or exact filename)
    let direct_path = keystore_dir.join(account_name);
    if direct_path.exists() {
        return Some(direct_path);
    }

    // Try finding with address suffix (new format: account_name_0x<address>)
    if let Ok(entries) = std::fs::read_dir(keystore_dir) {
        let search_prefix = format!("{account_name}_");
        for entry in entries.flatten() {
            if let Some(file_name) = entry.file_name().to_str()
                && file_name.starts_with(&search_prefix)
                && Address::parse_checksummed(&file_name[search_prefix.len()..], None).is_ok()
            {
                return Some(entry.path());
            }
        }
    }
    None
}

/// Creates keystore signer from given parameters.
///
/// If correct password or password file is provided, the keystore is decrypted and a [WalletSigner]
/// is returned.
///
/// Otherwise, a [PendingSigner] is returned, which can be used to unlock the keystore later,
/// prompting user for password.
pub fn create_keystore_signer(
    path: &PathBuf,
    maybe_password: Option<&str>,
    maybe_password_file: Option<&str>,
) -> Result<(Option<WalletSigner>, Option<PendingSigner>)> {
    if !path.exists() {
        eyre::bail!("Keystore file `{path:?}` does not exist")
    }

    if path.is_dir() {
        eyre::bail!(
            "Keystore path `{path:?}` is a directory. Please specify the keystore file directly."
        )
    }

    let password = match (maybe_password, maybe_password_file) {
        (Some(password), _) => Ok(Some(password.to_string())),
        (_, Some(password_file)) => {
            let password_file = Path::new(password_file);
            if !password_file.is_file() {
                Err(eyre::eyre!("Keystore password file `{password_file:?}` does not exist"))
            } else {
                Ok(Some(
                    fs::read_to_string(password_file)
                        .wrap_err_with(|| {
                            format!("Failed to read keystore password file at {password_file:?}")
                        })?
                        .trim_end()
                        .to_string(),
                ))
            }
        }
        (None, None) => Ok(None),
    }?;

    if let Some(password) = password {
        let wallet = PrivateKeySigner::decrypt_keystore(path, password)
            .wrap_err_with(|| format!("Failed to decrypt keystore {path:?}"))?;
        Ok((Some(WalletSigner::Local(wallet)), None))
    } else {
        Ok((None, Some(PendingSigner::Keystore(path.clone()))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_private_key_signer() {
        let pk = B256::random();
        let pk_str = pk.to_string();
        assert!(create_private_key_signer(&pk_str).is_ok());
        // skip 0x
        assert!(create_private_key_signer(&pk_str[2..]).is_ok());
    }
}
