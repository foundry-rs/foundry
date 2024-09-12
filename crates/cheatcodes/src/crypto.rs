//! Implementations of [`Crypto`](spec::Group::Crypto) Cheatcodes.

use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_primitives::{keccak256, Address, B256, U256};
use alloy_signer::{Signer, SignerSync};
use alloy_signer_local::{
    coins_bip39::{
        ChineseSimplified, ChineseTraditional, Czech, English, French, Italian, Japanese, Korean,
        Portuguese, Spanish, Wordlist,
    },
    MnemonicBuilder, PrivateKeySigner,
};
use alloy_sol_types::SolValue;
use k256::{
    ecdsa::SigningKey,
    elliptic_curve::{bigint::ArrayEncoding, sec1::ToEncodedPoint},
};
use p256::ecdsa::{signature::hazmat::PrehashSigner, Signature, SigningKey as P256SigningKey};

/// The BIP32 default derivation path prefix.
const DEFAULT_DERIVATION_PATH_PREFIX: &str = "m/44'/60'/0'/0/";

impl Cheatcode for createWallet_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { walletLabel } = self;
        create_wallet(&U256::from_be_bytes(keccak256(walletLabel).0), Some(walletLabel), state)
    }
}

impl Cheatcode for createWallet_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { privateKey } = self;
        create_wallet(privateKey, None, state)
    }
}

impl Cheatcode for createWallet_2Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { privateKey, walletLabel } = self;
        create_wallet(privateKey, Some(walletLabel), state)
    }
}

impl Cheatcode for sign_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { wallet, digest } = self;
        let sig = sign(&wallet.privateKey, digest)?;
        Ok(encode_full_sig(sig))
    }
}

impl Cheatcode for signCompact_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { wallet, digest } = self;
        let sig = sign(&wallet.privateKey, digest)?;
        Ok(encode_compact_sig(sig))
    }
}

impl Cheatcode for deriveKey_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { mnemonic, index } = self;
        derive_key::<English>(mnemonic, DEFAULT_DERIVATION_PATH_PREFIX, *index)
    }
}

impl Cheatcode for deriveKey_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { mnemonic, derivationPath, index } = self;
        derive_key::<English>(mnemonic, derivationPath, *index)
    }
}

impl Cheatcode for deriveKey_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { mnemonic, index, language } = self;
        derive_key_str(mnemonic, DEFAULT_DERIVATION_PATH_PREFIX, *index, language)
    }
}

impl Cheatcode for deriveKey_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { mnemonic, derivationPath, index, language } = self;
        derive_key_str(mnemonic, derivationPath, *index, language)
    }
}

impl Cheatcode for rememberKeyCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { privateKey } = self;
        let wallet = parse_wallet(privateKey)?;
        let address = wallet.address();
        if let Some(script_wallets) = state.script_wallets() {
            script_wallets.add_local_signer(wallet);
        }
        Ok(address.abi_encode())
    }
}

impl Cheatcode for sign_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { privateKey, digest } = self;
        let sig = sign(privateKey, digest)?;
        Ok(encode_full_sig(sig))
    }
}

impl Cheatcode for signCompact_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { privateKey, digest } = self;
        let sig = sign(privateKey, digest)?;
        Ok(encode_compact_sig(sig))
    }
}

impl Cheatcode for sign_2Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { digest } = self;
        let sig = sign_with_wallet(state, None, digest)?;
        Ok(encode_full_sig(sig))
    }
}

impl Cheatcode for signCompact_2Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { digest } = self;
        let sig = sign_with_wallet(state, None, digest)?;
        Ok(encode_compact_sig(sig))
    }
}

impl Cheatcode for sign_3Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { signer, digest } = self;
        let sig = sign_with_wallet(state, Some(*signer), digest)?;
        Ok(encode_full_sig(sig))
    }
}

impl Cheatcode for signCompact_3Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { signer, digest } = self;
        let sig = sign_with_wallet(state, Some(*signer), digest)?;
        Ok(encode_compact_sig(sig))
    }
}

impl Cheatcode for signP256Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { privateKey, digest } = self;
        sign_p256(privateKey, digest)
    }
}

impl Cheatcode for publicKeyP256Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { privateKey } = self;
        let pub_key =
            parse_private_key_p256(privateKey)?.verifying_key().as_affine().to_encoded_point(false);
        let pub_key_x = U256::from_be_bytes((*pub_key.x().unwrap()).into());
        let pub_key_y = U256::from_be_bytes((*pub_key.y().unwrap()).into());

        Ok((pub_key_x, pub_key_y).abi_encode())
    }
}

/// Using a given private key, return its public ETH address, its public key affine x and y
/// coordinates, and its private key (see the 'Wallet' struct)
///
/// If 'label' is set to 'Some()', assign that label to the associated ETH address in state
fn create_wallet(private_key: &U256, label: Option<&str>, state: &mut Cheatcodes) -> Result {
    let key = parse_private_key(private_key)?;
    let addr = alloy_signer::utils::secret_key_to_address(&key);

    let pub_key = key.verifying_key().as_affine().to_encoded_point(false);
    let pub_key_x = U256::from_be_bytes((*pub_key.x().unwrap()).into());
    let pub_key_y = U256::from_be_bytes((*pub_key.y().unwrap()).into());

    if let Some(label) = label {
        state.labels.insert(addr, label.into());
    }

    Ok(Wallet { addr, publicKeyX: pub_key_x, publicKeyY: pub_key_y, privateKey: *private_key }
        .abi_encode())
}

fn encode_full_sig(sig: alloy_primitives::Signature) -> Vec<u8> {
    // Retrieve v, r and s from signature.
    let v = U256::from(sig.v().y_parity_byte_non_eip155().unwrap_or(sig.v().y_parity_byte()));
    let r = B256::from(sig.r());
    let s = B256::from(sig.s());
    (v, r, s).abi_encode()
}

fn encode_compact_sig(sig: alloy_primitives::Signature) -> Vec<u8> {
    // Implement EIP-2098 compact signature.
    let r = B256::from(sig.r());
    let mut vs = sig.s();
    vs.set_bit(255, sig.v().y_parity());
    (r, vs).abi_encode()
}

fn sign(private_key: &U256, digest: &B256) -> Result<alloy_primitives::Signature> {
    // The `ecrecover` precompile does not use EIP-155. No chain ID is needed.
    let wallet = parse_wallet(private_key)?;
    let sig = wallet.sign_hash_sync(digest)?;
    debug_assert_eq!(sig.recover_address_from_prehash(digest)?, wallet.address());
    Ok(sig)
}

fn sign_with_wallet(
    state: &mut Cheatcodes,
    signer: Option<Address>,
    digest: &B256,
) -> Result<alloy_primitives::Signature> {
    let Some(script_wallets) = state.script_wallets() else {
        bail!("no wallets are available");
    };

    let mut script_wallets = script_wallets.inner.lock();
    let maybe_provided_sender = script_wallets.provided_sender;
    let signers = script_wallets.multi_wallet.signers()?;

    let signer = if let Some(signer) = signer {
        signer
    } else if let Some(provided_sender) = maybe_provided_sender {
        provided_sender
    } else if signers.len() == 1 {
        *signers.keys().next().unwrap()
    } else {
        bail!("could not determine signer");
    };

    let wallet = signers
        .get(&signer)
        .ok_or_else(|| fmt_err!("signer with address {signer} is not available"))?;

    let sig = foundry_common::block_on(wallet.sign_hash(digest))?;
    debug_assert_eq!(sig.recover_address_from_prehash(digest)?, signer);
    Ok(sig)
}

fn sign_p256(private_key: &U256, digest: &B256) -> Result {
    let signing_key = parse_private_key_p256(private_key)?;
    let signature: Signature = signing_key.sign_prehash(digest.as_slice())?;
    let r_bytes: [u8; 32] = signature.r().to_bytes().into();
    let s_bytes: [u8; 32] = signature.s().to_bytes().into();

    Ok((r_bytes, s_bytes).abi_encode())
}

fn validate_private_key<C: ecdsa::PrimeCurve>(private_key: &U256) -> Result<()> {
    ensure!(*private_key != U256::ZERO, "private key cannot be 0");
    let order = U256::from_be_slice(&C::ORDER.to_be_byte_array());
    ensure!(
        *private_key < U256::from_be_slice(&C::ORDER.to_be_byte_array()),
        "private key must be less than the {curve:?} curve order ({order})",
        curve = C::default(),
    );

    Ok(())
}

fn parse_private_key(private_key: &U256) -> Result<SigningKey> {
    validate_private_key::<k256::Secp256k1>(private_key)?;
    Ok(SigningKey::from_bytes((&private_key.to_be_bytes()).into())?)
}

fn parse_private_key_p256(private_key: &U256) -> Result<P256SigningKey> {
    validate_private_key::<p256::NistP256>(private_key)?;
    Ok(P256SigningKey::from_bytes((&private_key.to_be_bytes()).into())?)
}

pub(super) fn parse_wallet(private_key: &U256) -> Result<PrivateKeySigner> {
    parse_private_key(private_key).map(PrivateKeySigner::from)
}

fn derive_key_str(mnemonic: &str, path: &str, index: u32, language: &str) -> Result {
    match language {
        "chinese_simplified" => derive_key::<ChineseSimplified>(mnemonic, path, index),
        "chinese_traditional" => derive_key::<ChineseTraditional>(mnemonic, path, index),
        "czech" => derive_key::<Czech>(mnemonic, path, index),
        "english" => derive_key::<English>(mnemonic, path, index),
        "french" => derive_key::<French>(mnemonic, path, index),
        "italian" => derive_key::<Italian>(mnemonic, path, index),
        "japanese" => derive_key::<Japanese>(mnemonic, path, index),
        "korean" => derive_key::<Korean>(mnemonic, path, index),
        "portuguese" => derive_key::<Portuguese>(mnemonic, path, index),
        "spanish" => derive_key::<Spanish>(mnemonic, path, index),
        _ => Err(fmt_err!("unsupported mnemonic language: {language:?}")),
    }
}

fn derive_key<W: Wordlist>(mnemonic: &str, path: &str, index: u32) -> Result {
    fn derive_key_path(path: &str, index: u32) -> String {
        let mut out = path.to_string();
        if !out.ends_with('/') {
            out.push('/');
        }
        out.push_str(&index.to_string());
        out
    }

    let wallet = MnemonicBuilder::<W>::default()
        .phrase(mnemonic)
        .derivation_path(derive_key_path(path, index))?
        .build()?;
    let private_key = U256::from_be_bytes(wallet.credential().to_bytes().into());
    Ok(private_key.abi_encode())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{hex::FromHex, FixedBytes};
    use p256::ecdsa::signature::hazmat::PrehashVerifier;

    #[test]
    fn test_sign_p256() {
        use p256::ecdsa::VerifyingKey;

        let pk_u256: U256 = "1".parse().unwrap();
        let signing_key = P256SigningKey::from_bytes(&pk_u256.to_be_bytes().into()).unwrap();
        let digest = FixedBytes::from_hex(
            "0x44acf6b7e36c1342c2c5897204fe09504e1e2efb1a900377dbc4e7a6a133ec56",
        )
        .unwrap();

        let result = sign_p256(&pk_u256, &digest).unwrap();
        let result_bytes: [u8; 64] = result.try_into().unwrap();
        let signature = Signature::from_bytes(&result_bytes.into()).unwrap();
        let verifying_key = VerifyingKey::from(&signing_key);
        assert!(verifying_key.verify_prehash(digest.as_slice(), &signature).is_ok());
    }

    #[test]
    fn test_sign_p256_pk_too_large() {
        // max n from https://neuromancer.sk/std/secg/secp256r1
        let pk =
            "0xffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551".parse().unwrap();
        let digest = FixedBytes::from_hex(
            "0x54705ba3baafdbdfba8c5f9a70f7a89bee98d906b53e31074da7baecdc0da9ad",
        )
        .unwrap();
        let result = sign_p256(&pk, &digest);
        assert_eq!(result.err().unwrap().to_string(), "private key must be less than the NistP256 curve order (115792089210356248762697446949407573529996955224135760342422259061068512044369)");
    }

    #[test]
    fn test_sign_p256_pk_0() {
        let digest = FixedBytes::from_hex(
            "0x54705ba3baafdbdfba8c5f9a70f7a89bee98d906b53e31074da7baecdc0da9ad",
        )
        .unwrap();
        let result = sign_p256(&U256::ZERO, &digest);
        assert_eq!(result.err().unwrap().to_string(), "private key cannot be 0");
    }
}
