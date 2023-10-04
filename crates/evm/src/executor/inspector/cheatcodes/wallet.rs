use super::{ensure, Cheatcodes, Result};
use crate::abi::HEVMCalls;
use alloy_dyn_abi::DynSolValue;
use alloy_primitives::{keccak256, B256, U256};
use ethers::{
    core::k256::elliptic_curve::Curve,
    prelude::{
        k256::{
            ecdsa::SigningKey,
            elliptic_curve::{bigint::Encoding, sec1::ToEncodedPoint},
            Secp256k1,
        },
        LocalWallet, Signer,
    },
    signers::{
        coins_bip39::{
            ChineseSimplified, ChineseTraditional, Czech, English, French, Italian, Japanese,
            Korean, Portuguese, Spanish, Wordlist,
        },
        MnemonicBuilder,
    },
    utils,
};
use foundry_utils::types::{ToAlloy, ToEthers};
use revm::{Database, EVMData};
use std::str::FromStr;

/// The BIP32 default derivation path prefix.
const DEFAULT_DERIVATION_PATH_PREFIX: &str = "m/44'/60'/0'/0/";

pub fn parse_private_key(private_key: U256) -> Result<SigningKey> {
    ensure!(private_key != U256::ZERO, "Private key cannot be 0.");
    ensure!(
        private_key < U256::from_be_bytes(Secp256k1::ORDER.to_be_bytes()),
        "Private key must be less than the secp256k1 curve order \
        (115792089237316195423570985008687907852837564279074904382605163141518161494337).",
    );
    let bytes = private_key.to_be_bytes();
    SigningKey::from_bytes((&bytes).into()).map_err(Into::into)
}

fn addr(private_key: U256) -> Result {
    let key = parse_private_key(private_key)?;
    let addr = utils::secret_key_to_address(&key);
    Ok(DynSolValue::Address(addr.to_alloy()).abi_encode().into())
}

fn sign(private_key: U256, digest: B256, chain_id: U256) -> Result {
    let key = parse_private_key(private_key)?;
    let wallet = LocalWallet::from(key).with_chain_id(chain_id.to::<u64>());

    // The `ecrecover` precompile does not use EIP-155
    let sig = wallet.sign_hash(digest.to_ethers())?;
    let recovered = sig.recover(digest.to_ethers())?.to_alloy();

    assert_eq!(recovered, wallet.address().to_alloy());

    let mut r_bytes = [0u8; 32];
    let mut s_bytes = [0u8; 32];
    sig.r.to_big_endian(&mut r_bytes);
    sig.s.to_big_endian(&mut s_bytes);

    Ok(DynSolValue::Tuple(vec![
        DynSolValue::Uint(U256::from(sig.v), 8),
        DynSolValue::FixedBytes(r_bytes.into(), 32),
        DynSolValue::FixedBytes(s_bytes.into(), 32),
    ])
    .abi_encode()
    .into())
}

/// Using a given private key, return its public ETH address, its public key affine x and y
/// coodinates, and its private key (see the 'Wallet' struct)
///
/// If 'label' is set to 'Some()', assign that label to the associated ETH address in state
fn create_wallet(private_key: U256, label: Option<String>, state: &mut Cheatcodes) -> Result {
    let key = parse_private_key(private_key)?;
    let addr = utils::secret_key_to_address(&key);

    let pub_key = key.verifying_key().as_affine().to_encoded_point(false);
    let pub_key_x = pub_key.x().ok_or("No x coordinate was found")?;
    let pub_key_y = pub_key.y().ok_or("No y coordinate was found")?;

    let pub_key_x = U256::from_be_bytes((*pub_key_x).into());
    let pub_key_y = U256::from_be_bytes((*pub_key_y).into());

    if let Some(label) = label {
        state.labels.insert(addr.to_alloy(), label);
    }

    Ok(DynSolValue::Tuple(vec![
        DynSolValue::Address(addr.to_alloy()),
        DynSolValue::Uint(pub_key_x, 256),
        DynSolValue::Uint(pub_key_y, 256),
        DynSolValue::Uint(private_key, 256),
    ])
    .abi_encode()
    .into())
}

enum WordlistLang {
    ChineseSimplified,
    ChineseTraditional,
    Czech,
    English,
    French,
    Italian,
    Japanese,
    Korean,
    Portuguese,
    Spanish,
}

impl FromStr for WordlistLang {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input {
            "chinese_simplified" => Ok(Self::ChineseSimplified),
            "chinese_traditional" => Ok(Self::ChineseTraditional),
            "czech" => Ok(Self::Czech),
            "english" => Ok(Self::English),
            "french" => Ok(Self::French),
            "italian" => Ok(Self::Italian),
            "japanese" => Ok(Self::Japanese),
            "korean" => Ok(Self::Korean),
            "portuguese" => Ok(Self::Portuguese),
            "spanish" => Ok(Self::Spanish),
            _ => Err(format!("the language `{}` has no wordlist", input)),
        }
    }
}

fn derive_key<W: Wordlist>(mnemonic: &str, path: &str, index: u32) -> Result {
    let derivation_path =
        if path.ends_with('/') { format!("{path}{index}") } else { format!("{path}/{index}") };

    let wallet = MnemonicBuilder::<W>::default()
        .phrase(mnemonic)
        .derivation_path(&derivation_path)?
        .build()?;

    let private_key = match U256::try_from_be_slice(wallet.signer().to_bytes().as_slice()) {
        Some(key) => key,
        None => return Err("Failed to parse private key.".to_string().into()),
    };

    Ok(DynSolValue::Uint(private_key, 256).abi_encode().into())
}

fn derive_key_with_wordlist(mnemonic: &str, path: &str, index: u32, lang: &str) -> Result {
    let lang = WordlistLang::from_str(lang)?;
    match lang {
        WordlistLang::ChineseSimplified => derive_key::<ChineseSimplified>(mnemonic, path, index),
        WordlistLang::ChineseTraditional => derive_key::<ChineseTraditional>(mnemonic, path, index),
        WordlistLang::Czech => derive_key::<Czech>(mnemonic, path, index),
        WordlistLang::English => derive_key::<English>(mnemonic, path, index),
        WordlistLang::French => derive_key::<French>(mnemonic, path, index),
        WordlistLang::Italian => derive_key::<Italian>(mnemonic, path, index),
        WordlistLang::Japanese => derive_key::<Japanese>(mnemonic, path, index),
        WordlistLang::Korean => derive_key::<Korean>(mnemonic, path, index),
        WordlistLang::Portuguese => derive_key::<Portuguese>(mnemonic, path, index),
        WordlistLang::Spanish => derive_key::<Spanish>(mnemonic, path, index),
    }
}

fn remember_key(state: &mut Cheatcodes, private_key: U256, chain_id: U256) -> Result {
    let key = parse_private_key(private_key)?;
    let wallet = LocalWallet::from(key).with_chain_id(chain_id.to::<u64>());
    let address = wallet.address();

    state.script_wallets.push(wallet);

    Ok(DynSolValue::Address(address.to_alloy()).abi_encode().into())
}

#[instrument(level = "error", name = "util", target = "evm::cheatcodes", skip_all)]
pub fn apply<DB: Database>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    call: &HEVMCalls,
) -> Option<Result> {
    Some(match call {
        HEVMCalls::Addr(inner) => addr(inner.0.to_alloy()),
        // [function sign(uint256,bytes32)] Used to sign bytes32 digests using the given private key
        HEVMCalls::Sign0(inner) => {
            sign(inner.0.to_alloy(), inner.1.into(), U256::from(data.env.cfg.chain_id))
        }
        // [function createWallet(string)] Used to derive private key and label the wallet with the
        // same string
        HEVMCalls::CreateWallet0(inner) => {
            create_wallet(U256::from_be_bytes(keccak256(&inner.0).0), Some(inner.0.clone()), state)
        }
        // [function createWallet(uint256)] creates a new wallet with the given private key
        HEVMCalls::CreateWallet1(inner) => create_wallet(inner.0.to_alloy(), None, state),
        // [function createWallet(uint256,string)] creates a new wallet with the given private key
        // and labels it with the given string
        HEVMCalls::CreateWallet2(inner) => {
            create_wallet(inner.0.to_alloy(), Some(inner.1.clone()), state)
        }
        // [function sign(uint256,bytes32)] Used to sign bytes32 digests using the given Wallet's
        // private key
        HEVMCalls::Sign1(inner) => {
            sign(inner.0.private_key.to_alloy(), inner.1.into(), U256::from(data.env.cfg.chain_id))
        }
        HEVMCalls::DeriveKey0(inner) => {
            derive_key::<English>(&inner.0, DEFAULT_DERIVATION_PATH_PREFIX, inner.1)
        }
        HEVMCalls::DeriveKey1(inner) => derive_key::<English>(&inner.0, &inner.1, inner.2),
        HEVMCalls::DeriveKey2(inner) => {
            derive_key_with_wordlist(&inner.0, DEFAULT_DERIVATION_PATH_PREFIX, inner.1, &inner.2)
        }
        HEVMCalls::DeriveKey3(inner) => {
            derive_key_with_wordlist(&inner.0, &inner.1, inner.2, &inner.3)
        }
        HEVMCalls::RememberKey(inner) => {
            remember_key(state, inner.0.to_alloy(), U256::from(data.env.cfg.chain_id))
        }
        HEVMCalls::Label(inner) => {
            state.labels.insert(inner.0.to_alloy(), inner.1.clone());
            Ok(Default::default())
        }
        HEVMCalls::GetLabel(inner) => {
            let label = state
                .labels
                .get(&inner.0.to_alloy())
                .cloned()
                .unwrap_or_else(|| format!("unlabeled:{:?}", inner.0));
            Ok(DynSolValue::String(label).abi_encode().into())
        }
        _ => return None,
    })
}
