//! Implementations of [`Crypto`](spec::Group::Crypto) Cheatcodes.

use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_primitives::{Address, B256, U256, keccak256};
use alloy_signer::{Signer, SignerSync};
use alloy_signer_local::{
    LocalSigner, MnemonicBuilder, PrivateKeySigner,
    coins_bip39::{
        ChineseSimplified, ChineseTraditional, Czech, English, French, Italian, Japanese, Korean,
        Portuguese, Spanish, Wordlist,
    },
};
use alloy_sol_types::SolValue;
use k256::{
    FieldBytes, Scalar,
    ecdsa::{SigningKey, hazmat},
    elliptic_curve::{bigint::ArrayEncoding, sec1::ToEncodedPoint},
};

use p256::ecdsa::{
    Signature as P256Signature, SigningKey as P256SigningKey, signature::hazmat::PrehashSigner,
};

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

impl Cheatcode for signWithNonceUnsafeCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let pk: U256 = self.privateKey;
        let digest: B256 = self.digest;
        let nonce: U256 = self.nonce;
        let sig: alloy_primitives::Signature = sign_with_nonce(&pk, &digest, &nonce)?;
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
        let address = inject_wallet(state, wallet);
        Ok(address.abi_encode())
    }
}

impl Cheatcode for rememberKeys_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { mnemonic, derivationPath, count } = self;
        let wallets = derive_wallets::<English>(mnemonic, derivationPath, *count)?;
        let mut addresses = Vec::<Address>::with_capacity(wallets.len());
        for wallet in wallets {
            let addr = inject_wallet(state, wallet);
            addresses.push(addr);
        }

        Ok(addresses.abi_encode())
    }
}

impl Cheatcode for rememberKeys_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { mnemonic, derivationPath, language, count } = self;
        let wallets = derive_wallets_str(mnemonic, derivationPath, language, *count)?;
        let mut addresses = Vec::<Address>::with_capacity(wallets.len());
        for wallet in wallets {
            let addr = inject_wallet(state, wallet);
            addresses.push(addr);
        }

        Ok(addresses.abi_encode())
    }
}

fn inject_wallet(state: &mut Cheatcodes, wallet: LocalSigner<SigningKey>) -> Address {
    let address = wallet.address();
    state.wallets().add_local_signer(wallet);
    address
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
    let v = U256::from(sig.v() as u64 + 27);
    let r = B256::from(sig.r());
    let s = B256::from(sig.s());
    (v, r, s).abi_encode()
}

fn encode_compact_sig(sig: alloy_primitives::Signature) -> Vec<u8> {
    // Implement EIP-2098 compact signature.
    let r = B256::from(sig.r());
    let mut vs = sig.s();
    vs.set_bit(255, sig.v());
    (r, vs).abi_encode()
}

fn sign(private_key: &U256, digest: &B256) -> Result<alloy_primitives::Signature> {
    // The `ecrecover` precompile does not use EIP-155. No chain ID is needed.
    let wallet = parse_wallet(private_key)?;
    let sig = wallet.sign_hash_sync(digest)?;
    debug_assert_eq!(sig.recover_address_from_prehash(digest)?, wallet.address());
    Ok(sig)
}

/// Signs `digest` on secp256k1 using a user-supplied ephemeral nonce `k` (no RFC6979).
/// - `private_key` and `nonce` must be in (0, n)
/// - `digest` is a 32-byte prehash.
///
/// # Warning
///
/// Use [`sign_with_nonce`] with extreme caution!
/// Reusing the same nonce (`k`) with the same private key in ECDSA will leak the private key.
/// Always generate `nonce` with a cryptographically secure RNG, and never reuse it across
/// signatures.
fn sign_with_nonce(
    private_key: &U256,
    digest: &B256,
    nonce: &U256,
) -> Result<alloy_primitives::Signature> {
    let d_scalar: Scalar =
        <Scalar as k256::elliptic_curve::PrimeField>::from_repr(private_key.to_be_bytes().into())
            .into_option()
            .ok_or_else(|| fmt_err!("invalid private key scalar"))?;
    if bool::from(d_scalar.is_zero()) {
        return Err(fmt_err!("private key cannot be 0"));
    }

    let k_scalar: Scalar =
        <Scalar as k256::elliptic_curve::PrimeField>::from_repr(nonce.to_be_bytes().into())
            .into_option()
            .ok_or_else(|| fmt_err!("invalid nonce scalar"))?;
    if bool::from(k_scalar.is_zero()) {
        return Err(fmt_err!("nonce cannot be 0"));
    }

    let mut z = [0u8; 32];
    z.copy_from_slice(digest.as_slice());
    let z_fb: FieldBytes = FieldBytes::from(z);

    // Hazmat signing using the scalar `d` (SignPrimitive is implemented for `Scalar`)
    // Note: returns (Signature, Option<RecoveryId>)
    let (sig_raw, recid_opt) =
        <Scalar as hazmat::SignPrimitive<k256::Secp256k1>>::try_sign_prehashed(
            &d_scalar, k_scalar, &z_fb,
        )
        .map_err(|e| fmt_err!("sign_prehashed failed: {e}"))?;

    // Enforce low-s; if mirrored, parity flips (we’ll account for it below if we use recid)
    let (sig_low, flipped) =
        if let Some(norm) = sig_raw.normalize_s() { (norm, true) } else { (sig_raw, false) };

    let r_u256 = U256::from_be_bytes(sig_low.r().to_bytes().into());
    let s_u256 = U256::from_be_bytes(sig_low.s().to_bytes().into());

    // Determine v parity in {0,1}
    let v_parity = if let Some(id) = recid_opt {
        let mut v = id.to_byte() & 1;
        if flipped {
            v ^= 1;
        }
        v
    } else {
        // Fallback: choose parity by recovery to expected address
        let expected_addr = {
            let sk: SigningKey = parse_private_key(private_key)?;
            alloy_signer::utils::secret_key_to_address(&sk)
        };
        // Try v = 0
        let cand0 = alloy_primitives::Signature::new(r_u256, s_u256, false);
        if cand0.recover_address_from_prehash(digest).ok() == Some(expected_addr) {
            return Ok(cand0);
        }
        // Try v = 1
        let cand1 = alloy_primitives::Signature::new(r_u256, s_u256, true);
        if cand1.recover_address_from_prehash(digest).ok() == Some(expected_addr) {
            return Ok(cand1);
        }
        return Err(fmt_err!("failed to determine recovery id for signature"));
    };

    let y_parity = v_parity != 0;
    Ok(alloy_primitives::Signature::new(r_u256, s_u256, y_parity))
}

fn sign_with_wallet(
    state: &mut Cheatcodes,
    signer: Option<Address>,
    digest: &B256,
) -> Result<alloy_primitives::Signature> {
    if state.wallets().is_empty() {
        bail!("no wallets available");
    }

    let mut wallets = state.wallets().inner.lock();
    let maybe_provided_sender = wallets.provided_sender;
    let signers = wallets.multi_wallet.signers()?;

    let signer = if let Some(signer) = signer {
        signer
    } else if let Some(provided_sender) = maybe_provided_sender {
        provided_sender
    } else if signers.len() == 1 {
        *signers.keys().next().unwrap()
    } else {
        bail!(
            "could not determine signer, there are multiple signers available use vm.sign(signer, digest) to specify one"
        );
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
    let signature: P256Signature = signing_key.sign_prehash(digest.as_slice())?;
    let signature = signature.normalize_s().unwrap_or(signature);
    let r_bytes: [u8; 32] = signature.r().to_bytes().into();
    let s_bytes: [u8; 32] = signature.s().to_bytes().into();

    Ok((r_bytes, s_bytes).abi_encode())
}

fn validate_private_key<C: ecdsa::PrimeCurve>(private_key: &U256) -> Result<()> {
    ensure!(*private_key != U256::ZERO, "private key cannot be 0");
    let order = U256::from_be_slice(&C::ORDER.to_be_byte_array());
    ensure!(
        *private_key < order,
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

fn derive_wallets_str(
    mnemonic: &str,
    path: &str,
    language: &str,
    count: u32,
) -> Result<Vec<LocalSigner<SigningKey>>> {
    match language {
        "chinese_simplified" => derive_wallets::<ChineseSimplified>(mnemonic, path, count),
        "chinese_traditional" => derive_wallets::<ChineseTraditional>(mnemonic, path, count),
        "czech" => derive_wallets::<Czech>(mnemonic, path, count),
        "english" => derive_wallets::<English>(mnemonic, path, count),
        "french" => derive_wallets::<French>(mnemonic, path, count),
        "italian" => derive_wallets::<Italian>(mnemonic, path, count),
        "japanese" => derive_wallets::<Japanese>(mnemonic, path, count),
        "korean" => derive_wallets::<Korean>(mnemonic, path, count),
        "portuguese" => derive_wallets::<Portuguese>(mnemonic, path, count),
        "spanish" => derive_wallets::<Spanish>(mnemonic, path, count),
        _ => Err(fmt_err!("unsupported mnemonic language: {language:?}")),
    }
}

fn derive_wallets<W: Wordlist>(
    mnemonic: &str,
    path: &str,
    count: u32,
) -> Result<Vec<LocalSigner<SigningKey>>> {
    let mut out = path.to_string();

    if !out.ends_with('/') {
        out.push('/');
    }

    let mut wallets = Vec::with_capacity(count as usize);
    for idx in 0..count {
        let wallet = MnemonicBuilder::<W>::default()
            .phrase(mnemonic)
            .derivation_path(format!("{out}{idx}"))?
            .build()?;
        wallets.push(wallet);
    }

    Ok(wallets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{FixedBytes, hex::FromHex};
    use k256::elliptic_curve::Curve;
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
        let signature = P256Signature::from_bytes(&result_bytes.into()).unwrap();
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
        assert_eq!(
            result.err().unwrap().to_string(),
            "private key must be less than the NistP256 curve order (115792089210356248762697446949407573529996955224135760342422259061068512044369)"
        );
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

    #[test]
    fn test_sign_with_nonce_varies_and_recovers() {
        // Given a fixed private key and digest
        let pk_u256: U256 = U256::from(1u64);
        let digest = FixedBytes::from_hex(
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();

        // Two distinct nonces
        let n1: U256 = U256::from(123u64);
        let n2: U256 = U256::from(456u64);

        // Sign with both nonces
        let sig1 = sign_with_nonce(&pk_u256, &digest, &n1).expect("sig1");
        let sig2 = sign_with_nonce(&pk_u256, &digest, &n2).expect("sig2");

        // (r,s) must differ when nonce differs
        assert!(
            sig1.r() != sig2.r() || sig1.s() != sig2.s(),
            "signatures should differ with different nonces"
        );

        // ecrecover must yield the address for both signatures
        let sk = parse_private_key(&pk_u256).unwrap();
        let expected = alloy_signer::utils::secret_key_to_address(&sk);

        assert_eq!(sig1.recover_address_from_prehash(&digest).unwrap(), expected);
        assert_eq!(sig2.recover_address_from_prehash(&digest).unwrap(), expected);
    }

    #[test]
    fn test_sign_with_nonce_zero_nonce_errors() {
        // nonce = 0 should be rejected
        let pk_u256: U256 = U256::from(1u64);
        let digest = FixedBytes::from_hex(
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .unwrap();
        let n0: U256 = U256::ZERO;

        let err = sign_with_nonce(&pk_u256, &digest, &n0).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("nonce cannot be 0"), "unexpected error: {msg}");
    }

    #[test]
    fn test_sign_with_nonce_nonce_ge_order_errors() {
        // nonce >= n should be rejected
        use k256::Secp256k1;
        // Curve order n as U256
        let n_u256 = U256::from_be_slice(&Secp256k1::ORDER.to_be_byte_array());

        let pk_u256: U256 = U256::from(1u64);
        let digest = FixedBytes::from_hex(
            "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        )
        .unwrap();

        // Try exactly n (>= n invalid)
        let err = sign_with_nonce(&pk_u256, &digest, &n_u256).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("invalid nonce scalar"), "unexpected error: {msg}");
    }
}
