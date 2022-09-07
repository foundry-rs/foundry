use super::Cheatcodes;
use crate::{
    abi::HEVMCalls,
    executor::backend::error::{DatabaseError, DatabaseResult},
};
use bytes::{BufMut, Bytes, BytesMut};
use ethers::{
    abi::{AbiEncode, Address, ParamType, Token},
    core::k256::elliptic_curve::Curve,
    prelude::{
        k256::{ecdsa::SigningKey, elliptic_curve::bigint::Encoding, Secp256k1},
        LocalWallet, Signer, H160, *,
    },
    signers::{coins_bip39::English, MnemonicBuilder},
    types::{NameOrAddress, H256, U256},
    utils,
};
use foundry_common::fmt::*;
use hex::FromHex;
use revm::{Account, CreateInputs, Database, EVMData, JournaledState};
use std::str::FromStr;

const DEFAULT_DERIVATION_PATH_PREFIX: &str = "m/44'/60'/0'/0/";

pub const DEFAULT_CREATE2_DEPLOYER: H160 = H160([
    78, 89, 180, 72, 71, 179, 121, 87, 133, 136, 146, 12, 167, 143, 191, 38, 192, 180, 149, 108,
]);

/// Applies the given function `f` to the `revm::Account` belonging to the `addr`
///
/// This will ensure the `Account` is loaded and `touched`, see [`JournaledState::touch`]
pub fn with_journaled_account<F, R, DB: Database>(
    journaled_state: &mut JournaledState,
    db: &mut DB,
    addr: Address,
    mut f: F,
) -> Result<R, DB::Error>
where
    F: FnMut(&mut Account) -> R,
{
    journaled_state.load_account(addr, db)?;
    journaled_state.touch(&addr);
    let account = journaled_state.state.get_mut(&addr).expect("account loaded;");
    Ok(f(account))
}

fn addr(private_key: U256) -> Result<Bytes, Bytes> {
    if private_key.is_zero() {
        return Err("Private key cannot be 0.".to_string().encode().into())
    }

    if private_key >= U256::from_big_endian(&Secp256k1::ORDER.to_be_bytes()) {
        return Err("Private key must be less than 115792089237316195423570985008687907852837564279074904382605163141518161494337 (the secp256k1 curve order).".to_string().encode().into());
    }

    let mut bytes: [u8; 32] = [0; 32];
    private_key.to_big_endian(&mut bytes);

    let key = SigningKey::from_bytes(&bytes).map_err(|err| err.to_string().encode())?;
    let addr = utils::secret_key_to_address(&key);
    Ok(addr.encode().into())
}

fn sign(private_key: U256, digest: H256, chain_id: U256) -> Result<Bytes, Bytes> {
    if private_key.is_zero() {
        return Err("Private key cannot be 0.".to_string().encode().into())
    }

    if private_key >= U256::from_big_endian(&Secp256k1::ORDER.to_be_bytes()) {
        return Err("Private key must be less than 115792089237316195423570985008687907852837564279074904382605163141518161494337 (the secp256k1 curve order).".to_string().encode().into());
    }

    let mut bytes: [u8; 32] = [0; 32];
    private_key.to_big_endian(&mut bytes);

    let key = SigningKey::from_bytes(&bytes).map_err(|err| err.to_string().encode())?;
    let wallet = LocalWallet::from(key).with_chain_id(chain_id.as_u64());

    // The `ecrecover` precompile does not use EIP-155
    let sig = wallet.sign_hash(digest);
    let recovered = sig.recover(digest).map_err(|err| err.to_string().encode())?;

    assert_eq!(recovered, wallet.address());

    let mut r_bytes = [0u8; 32];
    let mut s_bytes = [0u8; 32];
    sig.r.to_big_endian(&mut r_bytes);
    sig.s.to_big_endian(&mut s_bytes);

    Ok((sig.v, r_bytes, s_bytes).encode().into())
}

fn derive_key(mnemonic: &str, path: &str, index: u32) -> Result<Bytes, Bytes> {
    let derivation_path = if path.ends_with('/') {
        format!("{}{}", path, index)
    } else {
        format!("{}/{}", path, index)
    };

    let wallet = MnemonicBuilder::<English>::default()
        .phrase(mnemonic)
        .derivation_path(&derivation_path)
        .map_err(|err| err.to_string().encode())?
        .build()
        .map_err(|err| err.to_string().encode())?;

    let private_key = U256::from_big_endian(wallet.signer().to_bytes().as_slice());

    Ok(private_key.encode().into())
}

fn remember_key(state: &mut Cheatcodes, private_key: U256, chain_id: U256) -> Result<Bytes, Bytes> {
    if private_key.is_zero() {
        return Err("Private key cannot be 0.".to_string().encode().into())
    }

    if private_key > U256::from_big_endian(&Secp256k1::ORDER.to_be_bytes()) {
        return Err("Private key must be less than 115792089237316195423570985008687907852837564279074904382605163141518161494337 (the secp256k1 curve order).".to_string().encode().into());
    }

    let mut bytes: [u8; 32] = [0; 32];
    private_key.to_big_endian(&mut bytes);

    let key = SigningKey::from_bytes(&bytes).map_err(|err| err.to_string().encode())?;
    let wallet = LocalWallet::from(key).with_chain_id(chain_id.as_u64());

    state.script_wallets.push(wallet.clone());

    Ok(wallet.address().encode().into())
}

fn parse(
    val: Vec<impl AsRef<str> + Clone>,
    r#type: ParamType,
    is_array: bool,
) -> Result<Bytes, Bytes> {
    let msg = format!("Failed to parse `{}` as type `{}`", &val[0].as_ref(), &r#type);
    value_to_abi(val, r#type, is_array).map_err(|e| format!("{}: {}", msg, e).encode().into())
}

pub fn apply<DB: Database>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    call: &HEVMCalls,
) -> Option<Result<Bytes, Bytes>> {
    Some(match call {
        HEVMCalls::Addr(inner) => addr(inner.0),
        HEVMCalls::Sign(inner) => sign(inner.0, inner.1.into(), data.env.cfg.chain_id),
        HEVMCalls::DeriveKey0(inner) => {
            derive_key(&inner.0, DEFAULT_DERIVATION_PATH_PREFIX, inner.1)
        }
        HEVMCalls::DeriveKey1(inner) => derive_key(&inner.0, &inner.1, inner.2),
        HEVMCalls::RememberKey(inner) => remember_key(state, inner.0, data.env.cfg.chain_id),
        HEVMCalls::Label(inner) => {
            state.labels.insert(inner.0, inner.1.clone());
            Ok(Bytes::new())
        }
        HEVMCalls::ToString0(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ToString1(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ToString2(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ToString3(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ToString4(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ToString5(inner) => {
            Ok(ethers::abi::encode(&[Token::String(inner.0.pretty())]).into())
        }
        HEVMCalls::ParseBytes(inner) => parse(vec![&inner.0], ParamType::Bytes, false),
        HEVMCalls::ParseAddress(inner) => parse(vec![&inner.0], ParamType::Address, false),
        HEVMCalls::ParseUint(inner) => parse(vec![&inner.0], ParamType::Uint(256), false),
        HEVMCalls::ParseInt(inner) => parse(vec![&inner.0], ParamType::Int(256), false),
        HEVMCalls::ParseBytes32(inner) => parse(vec![&inner.0], ParamType::FixedBytes(32), false),
        HEVMCalls::ParseBool(inner) => parse(vec![&inner.0], ParamType::Bool, false),
        _ => return None,
    })
}

pub fn process_create<DB>(
    broadcast_sender: Address,
    bytecode: Bytes,
    data: &mut EVMData<'_, DB>,
    call: &mut CreateInputs,
) -> DatabaseResult<(Bytes, Option<NameOrAddress>, u64)>
where
    DB: Database<Error = DatabaseError>,
{
    match call.scheme {
        revm::CreateScheme::Create => {
            call.caller = broadcast_sender;

            Ok((bytecode, None, data.journaled_state.account(broadcast_sender).info.nonce))
        }
        revm::CreateScheme::Create2 { salt } => {
            // Sanity checks for our CREATE2 deployer
            data.journaled_state.load_account(DEFAULT_CREATE2_DEPLOYER, data.db)?;

            let info = &data.journaled_state.account(DEFAULT_CREATE2_DEPLOYER).info;
            match &info.code {
                Some(code) => {
                    if code.is_empty() {
                        return Err(DatabaseError::MissingCreate2Deployer)
                    }
                }
                None => {
                    // forked db
                    if data.db.code_by_hash(info.code_hash)?.is_empty() {
                        return Err(DatabaseError::MissingCreate2Deployer)
                    }
                }
            }

            call.caller = DEFAULT_CREATE2_DEPLOYER;

            // We have to increment the nonce of the user address, since this create2 will be done
            // by the create2_deployer
            let account = data.journaled_state.state().get_mut(&broadcast_sender).unwrap();
            let nonce = account.info.nonce;
            account.info.nonce += 1;

            // Proxy deployer requires the data to be on the following format `salt.init_code`
            let mut calldata = BytesMut::with_capacity(32 + bytecode.len());
            let mut salt_bytes = [0u8; 32];
            salt.to_big_endian(&mut salt_bytes);
            calldata.put_slice(&salt_bytes);
            calldata.put(bytecode);

            Ok((calldata.freeze(), Some(NameOrAddress::Address(DEFAULT_CREATE2_DEPLOYER)), nonce))
        }
    }
}

pub fn value_to_abi(
    val: Vec<impl AsRef<str>>,
    r#type: ParamType,
    is_array: bool,
) -> Result<Bytes, String> {
    let parse_bool = |v: &str| v.to_lowercase().parse::<bool>();
    let parse_uint = |v: &str| {
        if v.starts_with("0x") {
            let v = Vec::from_hex(v.strip_prefix("0x").unwrap()).map_err(|e| e.to_string())?;
            Ok(U256::from_little_endian(&v))
        } else {
            U256::from_dec_str(v).map_err(|e| e.to_string())
        }
    };
    let parse_int = |v: &str| {
        // hex string may start with "0x", "+0x", or "-0x"
        if v.starts_with("0x") || v.starts_with("+0x") || v.starts_with("-0x") {
            I256::from_hex_str(&v.replacen("0x", "", 1)).map(|v| v.into_raw())
        } else {
            I256::from_dec_str(v).map(|v| v.into_raw())
        }
    };
    let parse_address = |v: &str| Address::from_str(v);
    let parse_string = |v: &str| -> Result<String, ()> { Ok(v.to_string()) };
    let parse_bytes = |v: &str| Vec::from_hex(v.strip_prefix("0x").unwrap_or(v));

    val.iter()
        .map(AsRef::as_ref)
        .map(|v| match r#type {
            ParamType::Bool => parse_bool(v).map(Token::Bool).map_err(|e| e.to_string()),
            ParamType::Uint(256) => parse_uint(v).map(Token::Uint),
            ParamType::Int(256) => parse_int(v).map(Token::Int).map_err(|e| e.to_string()),
            ParamType::Address => parse_address(v).map(Token::Address).map_err(|e| e.to_string()),
            ParamType::FixedBytes(32) => {
                parse_bytes(v).map(Token::FixedBytes).map_err(|e| e.to_string())
            }
            ParamType::String => parse_string(v).map(Token::String).map_err(|_| "".to_string()),
            ParamType::Bytes => parse_bytes(v).map(Token::Bytes).map_err(|e| e.to_string()),
            _ => Err(format!("{} is not a supported type", r#type)),
        })
        .collect::<Result<Vec<Token>, String>>()
        .map(|mut tokens| {
            if is_array {
                abi::encode(&[Token::Array(tokens)]).into()
            } else {
                abi::encode(&[tokens.remove(0)]).into()
            }
        })
}
