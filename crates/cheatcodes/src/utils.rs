//! Implementations of [`Utilities`](spec::Group::Utilities) cheatcodes.

use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_primitives::{Address, U256};
use alloy_sol_types::SolValue;
use foundry_common::ens::namehash;
use foundry_evm_core::constants::DEFAULT_CREATE2_DEPLOYER;
use rand::Rng;

impl Cheatcode for labelCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { account, newLabel } = self;
        state.labels.insert(*account, newLabel.clone());
        Ok(Default::default())
    }
}

impl Cheatcode for getLabelCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { account } = self;
        Ok(match state.labels.get(account) {
            Some(label) => label.abi_encode(),
            None => format!("unlabeled:{account}").abi_encode(),
        })
    }
}

impl Cheatcode for computeCreateAddressCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { nonce, deployer } = self;
        ensure!(*nonce <= U256::from(u64::MAX), "nonce must be less than 2^64 - 1");
        Ok(deployer.create(nonce.to()).abi_encode())
    }
}

impl Cheatcode for computeCreate2Address_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { salt, initCodeHash, deployer } = self;
        Ok(deployer.create2(salt, initCodeHash).abi_encode())
    }
}

impl Cheatcode for computeCreate2Address_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { salt, initCodeHash } = self;
        Ok(DEFAULT_CREATE2_DEPLOYER.create2(salt, initCodeHash).abi_encode())
    }
}

impl Cheatcode for ensNamehashCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { name } = self;
        Ok(namehash(name).abi_encode())
    }
}

impl Cheatcode for randomUint_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        let rng = state.rng();
        let random_number: U256 = rng.gen();
        Ok(random_number.abi_encode())
    }
}

impl Cheatcode for randomUint_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { min, max } = *self;
        ensure!(min <= max, "min must be less than or equal to max");
        // Generate random between range min..=max
        let exclusive_modulo = max - min;
        let rng = state.rng();
        let mut random_number = rng.gen::<U256>();
        if exclusive_modulo != U256::MAX {
            let inclusive_modulo = exclusive_modulo + U256::from(1);
            random_number %= inclusive_modulo;
        }
        random_number += min;
        Ok(random_number.abi_encode())
    }
}

impl Cheatcode for randomAddressCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        let rng = state.rng();
        let addr = Address::random_with(rng);
        Ok(addr.abi_encode())
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

fn encode_vrs(sig: alloy_primitives::Signature) -> Vec<u8> {
    let v = sig.v().y_parity_byte_non_eip155().unwrap_or(sig.v().y_parity_byte());

    (U256::from(v), B256::from(sig.r()), B256::from(sig.s())).abi_encode()
}

pub(super) fn sign(private_key: &U256, digest: &B256) -> Result {
    // The `ecrecover` precompile does not use EIP-155. No chain ID is needed.
    let wallet = parse_wallet(private_key)?;
    let sig = wallet.sign_hash_sync(digest)?;
    debug_assert_eq!(sig.recover_address_from_prehash(digest)?, wallet.address());
    Ok(encode_vrs(sig))
}

pub(super) fn sign_with_wallet<DB: DatabaseExt>(
    ccx: &mut CheatsCtxt<DB>,
    signer: Option<Address>,
    digest: &B256,
) -> Result {
    let Some(script_wallets) = ccx.state.script_wallets() else {
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
    Ok(encode_vrs(sig))
}

pub(super) fn sign_p256(private_key: &U256, digest: &B256, _state: &mut Cheatcodes) -> Result {
    ensure!(*private_key != U256::ZERO, "private key cannot be 0");
    let n = U256::from_limbs(*p256::NistP256::ORDER.as_words());
    ensure!(
        *private_key < n,
        format!("private key must be less than the secp256r1 curve order ({})", n),
    );
    let bytes = private_key.to_be_bytes();
    let signing_key = P256SigningKey::from_bytes((&bytes).into())?;
    let signature: Signature = signing_key.sign_prehash(digest.as_slice())?;
    let r_bytes: [u8; 32] = signature.r().to_bytes().into();
    let s_bytes: [u8; 32] = signature.s().to_bytes().into();

    Ok((r_bytes, s_bytes).abi_encode())
}

pub(super) fn generate_public_key_p256(private_key: &U256) -> Result {
    ensure!(*private_key != U256::ZERO, "private key cannot be 0");
    let n = U256::from_limbs(*p256::NistP256::ORDER.as_words());
    ensure!(
        *private_key < n,
        format!("private key must be less than the secp256r1 curve order ({})", n),
    );
    let sk_bytes = private_key.to_be_bytes();
    let secret_key = p256::SecretKey::from_bytes(&sk_bytes.into())
        .map_err(|_| "can't produce p256 secret key from bytes".to_string())?;
    let public_key = secret_key.public_key();
    let affine_points = public_key.as_affine();
    let points = affine_points.to_encoded_point(false);
    let pub_key_x = U256::from_be_bytes((*points.x().unwrap()).into());
    let pub_key_y = U256::from_be_bytes((*points.y().unwrap()).into());

    Ok((pub_key_x, pub_key_y).abi_encode())
}

pub(super) fn parse_private_key(private_key: &U256) -> Result<SigningKey> {
    ensure!(*private_key != U256::ZERO, "private key cannot be 0");
    ensure!(
        *private_key < U256::from_limbs(*Secp256k1::ORDER.as_words()),
        "private key must be less than the secp256k1 curve order \
         (115792089237316195423570985008687907852837564279074904382605163141518161494337)",
    );
    let bytes = private_key.to_be_bytes();
    SigningKey::from_bytes((&bytes).into()).map_err(Into::into)
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
    use crate::CheatsConfig;
    use alloy_primitives::{
        hex::{self, FromHex},
        FixedBytes,
    };
    use p256::{ecdsa::signature::hazmat::PrehashVerifier, SecretKey};
    use std::{path::PathBuf, sync::Arc};

    fn cheats() -> Cheatcodes {
        let config = CheatsConfig {
            ffi: true,
            root: PathBuf::from(&env!("CARGO_MANIFEST_DIR")),
            ..Default::default()
        };
        Cheatcodes { config: Arc::new(config), ..Default::default() }
    }

    #[test]
    fn test_sign_p256() {
        use p256::ecdsa::VerifyingKey;

        let pk_u256: U256 = "1".parse().unwrap();
        let signing_key = P256SigningKey::from_bytes(&pk_u256.to_be_bytes().into()).unwrap();
        let digest = FixedBytes::from_hex(
            "0x44acf6b7e36c1342c2c5897204fe09504e1e2efb1a900377dbc4e7a6a133ec56",
        )
        .unwrap();
        let mut cheats = cheats();

        let result = sign_p256(&pk_u256, &digest, &mut cheats).unwrap();
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
        let mut cheats = cheats();
        let result = sign_p256(&pk, &digest, &mut cheats);
        assert_eq!(result.err().unwrap().to_string(), "private key must be less than the secp256r1 curve order (115792089210356248762697446949407573529996955224135760342422259061068512044369)");
    }

    #[test]
    fn test_sign_p256_pk_0() {
        let digest = FixedBytes::from_hex(
            "0x54705ba3baafdbdfba8c5f9a70f7a89bee98d906b53e31074da7baecdc0da9ad",
        )
        .unwrap();
        let mut cheats = cheats();
        let result = sign_p256(&U256::ZERO, &digest, &mut cheats);
        assert_eq!(result.err().unwrap().to_string(), "private key cannot be 0");
    }

    #[test]
    fn test_affine_coordinates_p256() {
        let pk_u256: U256 = "100".parse().unwrap(); // Parse private key, assuming valid U256 creation
        let result = generate_public_key_p256(&pk_u256).unwrap(); // Get the ABI-encoded result

        // Ensure the result has exactly 64 bytes (32 for x and 32 for y)
        assert_eq!(result.len(), 64, "Resulting byte array must be exactly 64 bytes long.");

        // Convert Vec<u8> to a fixed-size array of 64 bytes
        let result_bytes: [u8; 64] = result.try_into().expect("Slice with incorrect length");

        // Extract x and y coordinates from the array
        let x_bytes: [u8; 32] = result_bytes[0..32].try_into().expect("Error slicing x bytes");
        let y_bytes: [u8; 32] = result_bytes[32..64].try_into().expect("Error slicing y bytes");

        // Convert byte arrays into U256 values
        let x_coord = U256::from_be_bytes(x_bytes);
        let y_coord = U256::from_be_bytes(y_bytes);

        // Use x_coord and y_coord as U256 values in your tests
        println!("x_coord: {x_coord}");
        println!("y_coord: {y_coord}");
    }

    #[test]
    fn test_p256_publickey_generation() {
        // Hexadecimal private key
        let hex_private_key = "8449b2b5f4dbeaf8b454fe16cb1f577c3d05df5224021390438b5c2c5a8ac235";

        // Convert the hex string to bytes
        let private_key_bytes = hex::decode(hex_private_key).expect("Decoding failed");

        // Ensure the byte array is of correct length (32 bytes for P-256)
        assert_eq!(private_key_bytes.len(), 32, "Private key must be 32 bytes long");

        // Convert the byte array to a fixed-size array
        let private_key_array: [u8; 32] =
            private_key_bytes.try_into().expect("Invalid length for private key");

        // Create a p256 SecretKey from the bytes
        let secret_key =
            SecretKey::from_bytes((&private_key_array).into()).expect("Invalid private key");

        // Derive the public key
        let public_key = secret_key.public_key();
        let public_key_encoded = public_key.to_encoded_point(false);

        // Extract the x and y coordinates
        let x = public_key_encoded.x().expect("Invalid public key").to_vec();
        let y = public_key_encoded.y().expect("Invalid public key").to_vec();

        // Convert x and y to U256
        let x_u256 = U256::from_be_bytes::<32>(x.try_into().expect("Invalid length for x"));
        let y_u256 = U256::from_be_bytes::<32>(y.try_into().expect("Invalid length for y"));

        println!("x: {x_u256:?}");
        println!("y: {y_u256:?}");

        // Expected public key coordinates
        let expected_x = U256::from_be_bytes::<32>(
            hex::decode("4590c8ef86344c05b8502e4dfc88fc7dce961766ce216781825879297d467d94")
                .unwrap()
                .try_into()
                .unwrap(),
        );
        let expected_y = U256::from_be_bytes::<32>(
            hex::decode("aa1eebf8ed7dcbbd9c467e6d4d6c9c22de0bf7307c50aedfc42c317b5fe4f7ac")
                .unwrap()
                .try_into()
                .unwrap(),
        );

        // Verify the result
        assert_eq!(x_u256, expected_x);
        assert_eq!(y_u256, expected_y);
    }
}
