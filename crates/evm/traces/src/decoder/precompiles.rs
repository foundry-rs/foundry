use crate::{CallTrace, DecodedCallData};
use alloy_primitives::{hex, B256, U256};
use alloy_sol_types::{abi, sol, SolCall};
use foundry_evm_core::precompiles::{
    BLAKE_2F, EC_ADD, EC_MUL, EC_PAIRING, EC_RECOVER, IDENTITY, MOD_EXP, POINT_EVALUATION,
    RIPEMD_160, SHA_256,
};
use itertools::Itertools;
use revm_inspectors::tracing::types::DecodedCallTrace;

sol! {
/// EVM precompiles interface. For illustration purposes only, as precompiles don't follow the
/// Solidity ABI codec.
///
/// Parameter names and types are taken from [evm.codes](https://www.evm.codes/precompiled).
interface Precompiles {
    struct EcPairingInput {
        uint256 x1;
        uint256 y1;
        uint256 x2;
        uint256 y2;
        uint256 x3;
        uint256 y3;
    }

    /* 0x01 */ function ecrecover(bytes32 hash, uint8 v, uint256 r, uint256 s) returns (address publicAddress);
    /* 0x02 */ function sha256(bytes data) returns (bytes32 hash);
    /* 0x03 */ function ripemd(bytes data) returns (bytes20 hash);
    /* 0x04 */ function identity(bytes data) returns (bytes data);
    /* 0x05 */ function modexp(uint256 Bsize, uint256 Esize, uint256 Msize, bytes B, bytes E, bytes M) returns (bytes value);
    /* 0x06 */ function ecadd(uint256 x1, uint256 y1, uint256 x2, uint256 y2) returns (uint256 x, uint256 y);
    /* 0x07 */ function ecmul(uint256 x1, uint256 y1, uint256 s) returns (uint256 x, uint256 y);
    /* 0x08 */ function ecpairing(EcPairingInput[] input) returns (bool success);
    /* 0x09 */ function blake2f(uint32 rounds, uint64[8] h, uint64[16] m, uint64[2] t, bool f) returns (uint64[8] h);
    /* 0x0a */ function pointEvaluation(bytes32 versionedHash, bytes32 z, bytes32 y, bytes1[48] commitment, bytes1[48] proof) returns (bytes value);
}
}
use Precompiles::*;

macro_rules! tri {
    ($e:expr) => {
        match $e {
            Ok(x) => x,
            Err(_) => return None,
        }
    };
}

/// Tries to decode a precompile call. Returns `Some` if successful.
pub(super) fn decode(trace: &CallTrace, _chain_id: u64) -> Option<DecodedCallTrace> {
    if !trace.address[..19].iter().all(|&x| x == 0) {
        return None;
    }

    let data = &trace.data;

    let (signature, args) = match trace.address {
        EC_RECOVER => {
            let (sig, ecrecoverCall { hash, v, r, s }) = tri!(abi_decode_call(data));
            (sig, vec![hash.to_string(), v.to_string(), r.to_string(), s.to_string()])
        }
        SHA_256 => (sha256Call::SIGNATURE, vec![data.to_string()]),
        RIPEMD_160 => (ripemdCall::SIGNATURE, vec![data.to_string()]),
        IDENTITY => (identityCall::SIGNATURE, vec![data.to_string()]),
        MOD_EXP => (modexpCall::SIGNATURE, tri!(decode_modexp(data))),
        EC_ADD => {
            let (sig, ecaddCall { x1, y1, x2, y2 }) = tri!(abi_decode_call(data));
            (sig, vec![x1.to_string(), y1.to_string(), x2.to_string(), y2.to_string()])
        }
        EC_MUL => {
            let (sig, ecmulCall { x1, y1, s }) = tri!(abi_decode_call(data));
            (sig, vec![x1.to_string(), y1.to_string(), s.to_string()])
        }
        EC_PAIRING => (ecpairingCall::SIGNATURE, tri!(decode_ecpairing(data))),
        BLAKE_2F => (blake2fCall::SIGNATURE, tri!(decode_blake2f(data))),
        POINT_EVALUATION => (pointEvaluationCall::SIGNATURE, tri!(decode_kzg(data))),
        _ => return None,
    };

    Some(DecodedCallTrace {
        label: Some("PRECOMPILES".to_string()),
        call_data: Some(DecodedCallData { signature: signature.to_string(), args }),
        // TODO: Decode return data too.
        return_data: None,
    })
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
    while !decoder.is_empty() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::hex;

    #[test]
    fn ecpairing() {
        // https://github.com/foundry-rs/foundry/issues/5337#issuecomment-1627384480
        let data = hex!(
            "
            26bbb723f965460ca7282cd75f0e3e7c67b15817f7cee60856b394936ed02917
            0fbe873ac672168143a91535450bab6c412dce8dc8b66a88f2da6e245f9282df
            13cd4f0451538ece5014fe6688b197aefcc611a5c6a7c319f834f2188ba04b08
            126ff07e81490a1b6ae92b2d9e700c8e23e9d5c7f6ab857027213819a6c9ae7d
            04183624c9858a56c54deb237c26cb4355bc2551312004e65fc5b299440b15a3
            2e4b11aa549ad6c667057b18be4f4437fda92f018a59430ebb992fa3462c9ca1
            2d4d9aa7e302d9df41749d5507949d05dbea33fbb16c643b22f599a2be6df2e2
            14bedd503c37ceb061d8ec60209fe345ce89830a19230301f076caff004d1926
            0967032fcbf776d1afc985f88877f182d38480a653f2decaa9794cbc3bf3060c
            0e187847ad4c798374d0d6732bf501847dd68bc0e071241e0213bc7fc13db7ab
            304cfbd1e08a704a99f5e847d93f8c3caafddec46b7a0d379da69a4d112346a7
            1739c1b1a457a8c7313123d24d2f9192f896b7c63eea05a9d57f06547ad0cec8
            001d6fedb032f70e377635238e0563f131670001f6abf439adb3a9d5d52073c6
            1889afe91e4e367f898a7fcd6464e5ca4e822fe169bccb624f6aeb87e4d060bc
            198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c2
            1800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed
            090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b
            12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa
            2dde6d7baf0bfa09329ec8d44c38282f5bf7f9ead1914edd7dcaebb498c84519
            0c359f868a85c6e6c1ea819cfab4a867501a3688324d74df1fe76556558b1937
            29f41c6e0e30802e2749bfb0729810876f3423e6f24829ad3e30adb1934f1c8a
            030e7a5f70bb5daa6e18d80d6d447e772efb0bb7fb9d0ffcd54fc5a48af1286d
            0ea726b117e48cda8bce2349405f006a84cdd3dcfba12efc990df25970a27b6d
            30364cd4f8a293b1a04f0153548d3e01baad091c69097ca4e9f26be63e4095b5
        "
        );
        let decoded = decode_ecpairing(&data).unwrap();
        // 4 arrays of 6 32-byte values
        assert_eq!(decoded.len(), 4);
    }
}
