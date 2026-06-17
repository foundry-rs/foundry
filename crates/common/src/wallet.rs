use alloy_primitives::U256;
use alloy_signer_local::{
    MnemonicBuilder, PrivateKeySigner,
    coins_bip39::{
        ChineseSimplified, ChineseTraditional, Czech, English, French, Italian, Japanese, Korean,
        Portuguese, Spanish, Wordlist,
    },
};

/// Appends `index` to `path`, inserting a `/` separator when needed.
pub fn derive_key_path(path: &str, index: u32) -> String {
    let mut out = path.to_string();
    if !out.ends_with('/') {
        out.push('/');
    }
    out.push_str(&index.to_string());
    out
}

/// Derives a private key from a BIP-39 mnemonic using the given BIP-32 path and index.
pub fn derive_private_key<W: Wordlist>(
    mnemonic: &str,
    path: &str,
    index: u32,
) -> Result<U256, String> {
    let wallet = MnemonicBuilder::<W>::default()
        .phrase(mnemonic)
        .derivation_path(derive_key_path(path, index))
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
