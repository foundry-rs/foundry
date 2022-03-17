use crate::abi::HEVMCalls;
use bytes::Bytes;
use ethers::{
    abi::AbiEncode,
    prelude::{k256::ecdsa::SigningKey, LocalWallet, Signer},
    types::{H256, U256},
    utils,
};
use revm::{Database, EVMData};

use super::Cheatcodes;

fn addr(private_key: U256) -> Result<Bytes, Bytes> {
    if private_key.is_zero() {
        return Err("Private key cannot be 0.".to_string().encode().into())
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

    let mut bytes: [u8; 32] = [0; 32];
    private_key.to_big_endian(&mut bytes);

    let key = SigningKey::from_bytes(&bytes).map_err(|err| err.to_string().encode())?;
    let wallet = LocalWallet::from(key).with_chain_id(chain_id.as_u64());

    // The `ecrecover` precompile does not use EIP-155
    let sig = wallet.sign_hash(digest, false);
    let recovered = sig.recover(digest).map_err(|err| err.to_string().encode())?;

    assert_eq!(recovered, wallet.address());

    let mut r_bytes = [0u8; 32];
    let mut s_bytes = [0u8; 32];
    sig.r.to_big_endian(&mut r_bytes);
    sig.s.to_big_endian(&mut s_bytes);

    Ok((sig.v, r_bytes, s_bytes).encode().into())
}

pub fn apply<DB: Database>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    call: &HEVMCalls,
) -> Option<Result<Bytes, Bytes>> {
    Some(match call {
        HEVMCalls::Addr(inner) => addr(inner.0),
        HEVMCalls::Sign(inner) => sign(inner.0, inner.1.into(), data.env.cfg.chain_id),
        HEVMCalls::Label(inner) => {
            state.labels.insert(inner.0, inner.1.clone());
            Ok(Bytes::new())
        }
        _ => return None,
    })
}
