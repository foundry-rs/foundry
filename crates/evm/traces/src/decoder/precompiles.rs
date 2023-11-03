use crate::{CallTrace, RawOrDecodedCall};
use alloy_primitives::{B256, U256};
use alloy_sol_types::{abi, sol, SolCall};
use itertools::Itertools;

sol! {
/// Ethereum precompiles interface. For illustration purposes only, as precompiles don't follow the
/// Solidity ABI codec.
///
/// Parameter names and types are taken from [evm.codes](https://www.evm.codes/precompiled).
interface EthereumPrecompiles {
    /* 0x01 */ function ecrecover(bytes32 hash, uint8 v, uint256 r, uint256 s) returns (address publicAddress);
    /* 0x02 */ function sha256(bytes data) returns (bytes32 hash);
    /* 0x03 */ function ripemd(bytes data) returns (bytes20 hash);
    /* 0x04 */ function identity(bytes data) returns (bytes data);
    /* 0x05 */ function modexp(uint256 Bsize, uint256 Esize, uint256 Msize, bytes B, bytes E, bytes M) returns (bytes value);
    /* 0x06 */ function ecadd(uint256 x1, uint256 y1, uint256 x2, uint256 y2) returns (uint256 x, uint256 y);
    /* 0x07 */ function ecmul(uint256 x1, uint256 y1, uint256 s) returns (uint256 x, uint256 y);
    /* 0x08 */ function ecpairing(uint256[] x, uint256[] y) returns (bool success);
    /* 0x09 */ function blake2f(uint32 rounds, uint64[8] h, uint64[16] m, uint64[2] t, bool f) returns (uint64[8] h);
    /* 0x0a */ function pointEvaluation(bytes32 versionedHash, bytes32 z, bytes32 y, bytes1[48] commitment, bytes1[48] proof) returns (bytes value);
}
}
use EthereumPrecompiles::*;

macro_rules! tri {
    ($e:expr) => {
        match $e {
            Ok(x) => x,
            Err(_) => return false,
        }
    };
}

/// Tries to decode a precompile call. Returns `true` if successful.
pub(super) fn decode(trace: &mut CallTrace, _chain_id: u64) -> bool {
    let [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, x @ 0x01..=0x0a] =
        trace.address.0 .0
    else {
        return false
    };

    let RawOrDecodedCall::Raw(data) = &trace.data else { return false };

    let (signature, args) = match x {
        0x01 => {
            let (sig, ecrecoverCall { hash, v, r, s }) = tri!(abi_decode_call(data));
            (sig, vec![hash.to_string(), v.to_string(), r.to_string(), s.to_string()])
        }
        0x02 => (sha256Call::SIGNATURE, vec![data.to_string()]),
        0x03 => (ripemdCall::SIGNATURE, vec![data.to_string()]),
        0x04 => (identityCall::SIGNATURE, vec![data.to_string()]),
        0x05 => (modexpCall::SIGNATURE, tri!(decode_modexp(data))),
        0x06 => {
            let (sig, ecaddCall { x1, y1, x2, y2 }) = tri!(abi_decode_call(data));
            (sig, vec![x1.to_string(), y1.to_string(), x2.to_string(), y2.to_string()])
        }
        0x07 => {
            let (sig, ecmulCall { x1, y1, s }) = tri!(abi_decode_call(data));
            (sig, vec![x1.to_string(), y1.to_string(), s.to_string()])
        }
        0x08 => (ecpairingCall::SIGNATURE, tri!(decode_ecpairing(data))),
        0x09 => (blake2fCall::SIGNATURE, tri!(decode_blake2f(data))),
        0x0a => (pointEvaluationCall::SIGNATURE, tri!(decode_kzg(data))),
        _ => unreachable!(),
    };

    // TODO: Other chain precompiles

    trace.data = RawOrDecodedCall::Decoded { signature: signature.to_string(), args };

    trace.contract = Some("PRECOMPILES".into());

    true
}

// Note: we use the ABI decoder, but this is not necessarily ABI-encoded data. It's just a
// convenient way to decode the data.

fn decode_modexp(data: &[u8]) -> alloy_sol_types::Result<Vec<String>> {
    let mut decoder = abi::Decoder::new(data, false);
    let b_size = decoder.take_offset()?;
    let e_size = decoder.take_offset()?;
    let m_size = decoder.take_offset()?;
    let b = decoder.take_slice_unchecked(b_size)?;
    let e = decoder.take_slice_unchecked(e_size)?;
    let m = decoder.take_slice_unchecked(m_size)?;
    Ok(vec![
        b_size.to_string(),
        e_size.to_string(),
        m_size.to_string(),
        hex::encode_prefixed(b),
        hex::encode_prefixed(e),
        hex::encode_prefixed(m),
    ])
}

fn decode_ecpairing(data: &[u8]) -> alloy_sol_types::Result<Vec<String>> {
    let mut decoder = abi::Decoder::new(data, false);
    let mut values = Vec::new();
    // input must be either empty or a multiple of 6 32-byte values
    let mut tmp = <[&B256; 6]>::default();
    while decoder.peek(0).is_ok() {
        for tmp in &mut tmp {
            *tmp = decoder.take_word()?;
        }
        values.push(iter_to_string(tmp.iter().map(|x| U256::from_be_bytes(x.0))));
    }
    Ok(values)
}

fn decode_blake2f<'a>(data: &'a [u8]) -> alloy_sol_types::Result<Vec<String>> {
    let mut decoder = abi::Decoder::new(data, false);
    let rounds = u32::from_be_bytes(decoder.take_slice_unchecked(4)?.try_into().unwrap());
    let u64_le_list =
        |x: &'a [u8]| x.chunks_exact(8).map(|x| u64::from_le_bytes(x.try_into().unwrap()));
    let h = u64_le_list(decoder.take_slice_unchecked(64)?);
    let m = u64_le_list(decoder.take_slice_unchecked(128)?);
    let t = u64_le_list(decoder.take_slice_unchecked(16)?);
    let f = decoder.take_slice_unchecked(1)?[0];
    Ok(vec![
        rounds.to_string(),
        iter_to_string(h),
        iter_to_string(m),
        iter_to_string(t),
        f.to_string(),
    ])
}

fn decode_kzg(data: &[u8]) -> alloy_sol_types::Result<Vec<String>> {
    let mut decoder = abi::Decoder::new(data, false);
    let versioned_hash = decoder.take_word()?;
    let z = decoder.take_word()?;
    let y = decoder.take_word()?;
    let commitment = decoder.take_slice_unchecked(48)?;
    let proof = decoder.take_slice_unchecked(48)?;
    Ok(vec![
        versioned_hash.to_string(),
        z.to_string(),
        y.to_string(),
        hex::encode_prefixed(commitment),
        hex::encode_prefixed(proof),
    ])
}

fn abi_decode_call<T: SolCall>(data: &[u8]) -> alloy_sol_types::Result<(&'static str, T)> {
    // raw because there are no selectors here
    Ok((T::SIGNATURE, T::abi_decode_raw(data, false)?))
}

fn iter_to_string<I: Iterator<Item = T>, T: std::fmt::Display>(iter: I) -> String {
    format!("[{}]", iter.format(", "))
}
