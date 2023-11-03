use crate::CallTrace;
use alloy_primitives::U256;
use alloy_sol_types::{abi, abi::token, sol};

sol! {
    /// Ethereum precompiles interface.
    ///
    /// Parameter names and types are taken from [evm.codes](https://www.evm.codes/precompiled).
    ///
    /// Note that this interface should not be used directly for decoding, but rather through
    /// [CallTraceDecoder].
    /// This is because `modexp`, `ecpairing`, and `blake2f` don't strictly follow the ABI codec.
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
    }
}

macro_rules! try_ {
    ($e:expr) => {
        match $e {
            Ok(x) => x,
            Err(_) => return false,
        }
    };
}

#[must_use]
pub(super) fn decode(trace: &mut CallTrace) -> bool {
    let [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, x] = trace.address.as_slice()
    else {
        return false
    };

    match *x {
        0x01 => {}
        0x02 => {}
        0x03 => {}
        0x04 => {}
        0x05 => {}
        0x06 => {}
        0x07 => {}
        0x08 => {}
        0x09 => {}
        _ => return false,
    }

    trace.contract = Some("PRECOMPILES".into());

    true
}

fn decode_modexp(data: &[u8]) -> alloy_sol_types::Result<Vec<String>> {
    let mut decoder = abi::Decoder::new(data, false);
    let b_size = decode_usize(&mut decoder)?;
    let e_size = decode_usize(&mut decoder)?;
    let m_size = decode_usize(&mut decoder)?;
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

fn decode_usize(decoder: &mut abi::Decoder<'_>) -> alloy_sol_types::Result<usize> {
    let word = decoder.decode::<token::WordToken>()?;
    usize::try_from(<U256 as From<_>>::from(word.0))
        .map_err(|e| alloy_sol_types::Error::custom(e.to_string()))
}
