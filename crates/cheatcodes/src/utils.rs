//! Implementations of [`Utils`](crate::Group::Utils) cheatcodes.

use crate::{Cheatcode, Cheatcodes, CheatsCtxt, DatabaseExt, Result, Vm::*};
use alloy_primitives::{keccak256, B256, U256};
use alloy_sol_types::SolValue;
use ethers_core::k256::{
    ecdsa::SigningKey,
    elliptic_curve::{sec1::ToEncodedPoint, Curve},
    Secp256k1,
};
use ethers_signers::{
    coins_bip39::{
        ChineseSimplified, ChineseTraditional, Czech, English, French, Italian, Japanese, Korean,
        Portuguese, Spanish, Wordlist,
    },
    LocalWallet, MnemonicBuilder, Signer,
};
use foundry_common::types::{ToAlloy, ToEthers};
use foundry_evm_core::constants::DEFAULT_CREATE2_DEPLOYER;
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

impl Cheatcode for getNonce_1Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { wallet } = self;
        super::evm::get_nonce(ccx, &wallet.addr)
    }
}

impl Cheatcode for sign_1Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { wallet, digest } = self;
        sign(&wallet.privateKey, digest, ccx.data.env.cfg.chain_id)
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
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { privateKey } = self;
        let wallet = parse_wallet(privateKey)?.with_chain_id(ccx.data.env.cfg.chain_id);
        let address = wallet.address();
        ccx.state.script_wallets.push(wallet);
        Ok(address.to_alloy().abi_encode())
    }
}

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

/// Using a given private key, return its public ETH address, its public key affine x and y
/// coordinates, and its private key (see the 'Wallet' struct)
///
/// If 'label' is set to 'Some()', assign that label to the associated ETH address in state
fn create_wallet(private_key: &U256, label: Option<&str>, state: &mut Cheatcodes) -> Result {
    let key = parse_private_key(private_key)?;
    let addr = ethers_core::utils::secret_key_to_address(&key).to_alloy();

    let pub_key = key.verifying_key().as_affine().to_encoded_point(false);
    let pub_key_x = U256::from_be_bytes((*pub_key.x().unwrap()).into());
    let pub_key_y = U256::from_be_bytes((*pub_key.y().unwrap()).into());

    if let Some(label) = label {
        state.labels.insert(addr, label.into());
    }

    Ok(Wallet { addr, publicKeyX: pub_key_x, publicKeyY: pub_key_y, privateKey: *private_key }
        .abi_encode())
}

pub(super) fn sign(private_key: &U256, digest: &B256, chain_id: u64) -> Result {
    let wallet = parse_wallet(private_key)?.with_chain_id(chain_id);

    // The `ecrecover` precompile does not use EIP-155
    let sig = wallet.sign_hash(digest.to_ethers())?;
    let recovered = sig.recover(digest.to_ethers())?.to_alloy();

    assert_eq!(recovered, wallet.address().to_alloy());

    let mut r_bytes = [0u8; 32];
    let mut s_bytes = [0u8; 32];
    sig.r.to_big_endian(&mut r_bytes);
    sig.s.to_big_endian(&mut s_bytes);

    Ok((sig.v, r_bytes, s_bytes).abi_encode())
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

pub(super) fn parse_wallet(private_key: &U256) -> Result<LocalWallet> {
    parse_private_key(private_key).map(LocalWallet::from)
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
        .derivation_path(&derive_key_path(path, index))?
        .build()?;
    let private_key = U256::from_be_bytes(wallet.signer().to_bytes().into());
    Ok(private_key.abi_encode())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CheatsConfig;
    use alloy_primitives::FixedBytes;
    use hex::FromHex;
    use p256::ecdsa::{signature::hazmat::PrehashVerifier, Signature};
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
}
