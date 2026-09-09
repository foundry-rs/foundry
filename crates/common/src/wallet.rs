use alloy_primitives::U256;
use alloy_signer_local::{
    MnemonicBuilder, PrivateKeySigner,
    coins_bip39::{
        ChineseSimplified, ChineseTraditional, Czech, English, French, Italian, Japanese, Korean,
        Portuguese, Spanish, Wordlist,
    },
};

/// BIP-32 harden bit (`0x8000_0000`). Indices at or above this already encode hardened.
pub const BIP32_HARDEN: u32 = 0x8000_0000;

/// Reject BIP-32 path components whose parsed index overflows the harden bit.
///
/// `coins-bip32` parses `2147483648'` as `harden_index(0x8000_0000)` which wraps to `0`,
/// silently selecting the wrong (unhardened) child. Raw `2147483648` is also rejected
/// because it already equals the harden bit (`0'`).
///
/// This is only an overflow guard. One trailing `'` or `h` is stripped so the numeric
/// index can be checked; malformed syntax is left to [`MnemonicBuilder`].
pub fn validate_bip32_path(path: &str) -> Result<(), String> {
    for c in path.split('/') {
        let num = c.strip_suffix('\'').or_else(|| c.strip_suffix('h')).unwrap_or(c);
        if let Ok(v) = num.parse::<u32>()
            && v >= BIP32_HARDEN
        {
            return Err(format!(
                "BIP32 component {c} overflows harden bit (index must be < {BIP32_HARDEN})"
            ));
        }
    }
    Ok(())
}

/// Appends `index` to `path`, inserting a `/` separator when needed.
pub fn derive_key_path(path: &str, index: u32) -> String {
    let mut out = path.to_string();
    if !out.ends_with('/') {
        out.push('/');
    }
    out.push_str(&index.to_string());
    out
}

/// [`derive_key_path`] plus the harden-bit overflow guard for mnemonic derive consumers.
pub fn derive_key_path_checked(path: &str, index: u32) -> Result<String, String> {
    if index >= BIP32_HARDEN {
        return Err(format!(
            "BIP32 index {index} already sets harden bit (must be < {BIP32_HARDEN})"
        ));
    }
    validate_bip32_path(path)?;
    Ok(derive_key_path(path, index))
}

/// Derives a private key from a BIP-39 mnemonic using the given BIP-32 path and index.
pub fn derive_private_key<W: Wordlist>(
    mnemonic: &str,
    path: &str,
    index: u32,
) -> Result<U256, String> {
    let wallet = MnemonicBuilder::<W>::default()
        .phrase(mnemonic)
        .derivation_path(derive_key_path_checked(path, index)?)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    Ok(U256::from_be_bytes(wallet.credential().to_bytes().into()))
}

/// Derives a private key from a BIP-39 mnemonic, selecting the wordlist by name.
///
/// Recognised language names: `chinese_simplified`, `chinese_traditional`, `czech`, `english`,
/// `french`, `italian`, `japanese`, `korean`, `portuguese`, `spanish`.
pub fn derive_private_key_with_language(
    mnemonic: &str,
    path: &str,
    index: u32,
    language: &str,
) -> Result<U256, String> {
    match language {
        "chinese_simplified" => derive_private_key::<ChineseSimplified>(mnemonic, path, index),
        "chinese_traditional" => derive_private_key::<ChineseTraditional>(mnemonic, path, index),
        "czech" => derive_private_key::<Czech>(mnemonic, path, index),
        "english" => derive_private_key::<English>(mnemonic, path, index),
        "french" => derive_private_key::<French>(mnemonic, path, index),
        "italian" => derive_private_key::<Italian>(mnemonic, path, index),
        "japanese" => derive_private_key::<Japanese>(mnemonic, path, index),
        "korean" => derive_private_key::<Korean>(mnemonic, path, index),
        "portuguese" => derive_private_key::<Portuguese>(mnemonic, path, index),
        "spanish" => derive_private_key::<Spanish>(mnemonic, path, index),
        _ => Err(format!("unsupported mnemonic language: {language:?}")),
    }
}

/// Constructs a [`PrivateKeySigner`] from a raw private key value.
///
/// Returns `Err` when `private_key` is zero or its bytes are not a valid secp256k1 scalar.
pub fn private_key_from_u256(private_key: U256) -> Result<PrivateKeySigner, String> {
    if private_key.is_zero() {
        return Err("private key cannot be zero".to_string());
    }
    PrivateKeySigner::from_slice(&private_key.to_be_bytes::<32>()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // anvil default test mnemonic
    const MNEMONIC: &str = "test test test test test test test test test test test junk";

    #[test]
    fn validate_rejects_overflow_and_leaves_malformed_to_builder() {
        assert!(validate_bip32_path("m/44'/60'/0'/0/2147483648").is_err());
        assert!(validate_bip32_path("m/2147483648'").is_err());
        assert!(validate_bip32_path("m/2147483648h").is_err());
        assert!(validate_bip32_path("m/02147483648").is_err());
        assert!(validate_bip32_path(&format!("m/{}", u32::MAX)).is_err());
        assert!(validate_bip32_path("m/44'/60'/0'/0/0").is_ok());
        assert!(validate_bip32_path("m/44'/60'/0'/0/0'").is_ok());
        assert!(validate_bip32_path(&format!("m/{}", BIP32_HARDEN - 1)).is_ok());
        assert!(validate_bip32_path("m/+2147483648").is_err());
        assert!(validate_bip32_path("m/not-a-number").is_ok());
    }

    #[test]
    fn derive_key_path_stays_string() {
        assert_eq!(derive_key_path("m/44'/60'/0'/0", 0), "m/44'/60'/0'/0/0");
        assert_eq!(derive_key_path("m/44'/60'/0'/0/", 1), "m/44'/60'/0'/0/1");
    }

    #[test]
    fn derive_key_path_checked_rejects_overflowing_index() {
        assert!(derive_key_path_checked("m/44'/60'/0'/0", BIP32_HARDEN).is_err());
        assert_eq!(derive_key_path_checked("m/44'/60'/0'/0", 0).unwrap(), "m/44'/60'/0'/0/0");
    }

    #[test]
    fn derive_private_key_rejects_overflowing_path_and_index() {
        let err =
            derive_private_key::<English>(MNEMONIC, "m/44'/60'/0'/0", BIP32_HARDEN).unwrap_err();
        assert!(err.contains("harden bit"), "{err}");

        let err =
            derive_private_key::<English>(MNEMONIC, "m/44'/60'/0'/0/2147483648'", 0).unwrap_err();
        assert!(err.contains("harden bit") || err.contains("overflow"), "{err}");

        let key = derive_private_key::<English>(MNEMONIC, "m/44'/60'/0'/0", 0).unwrap();
        assert!(!key.is_zero());
    }
}
