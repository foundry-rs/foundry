use alloy_primitives::{address, Address, Bytes, B256};
use revm::{
    precompile::{secp256r1::p256_verify as revm_p256_verify, PrecompileWithAddress},
    primitives::{Precompile, PrecompileResult},
};

/// The ECRecover precompile address.
pub const EC_RECOVER: Address = address!("0000000000000000000000000000000000000001");

/// The SHA-256 precompile address.
pub const SHA_256: Address = address!("0000000000000000000000000000000000000002");

/// The RIPEMD-160 precompile address.
pub const RIPEMD_160: Address = address!("0000000000000000000000000000000000000003");

/// The Identity precompile address.
pub const IDENTITY: Address = address!("0000000000000000000000000000000000000004");

/// The ModExp precompile address.
pub const MOD_EXP: Address = address!("0000000000000000000000000000000000000005");

/// The ECAdd precompile address.
pub const EC_ADD: Address = address!("0000000000000000000000000000000000000006");

/// The ECMul precompile address.
pub const EC_MUL: Address = address!("0000000000000000000000000000000000000007");

/// The ECPairing precompile address.
pub const EC_PAIRING: Address = address!("0000000000000000000000000000000000000008");

/// The Blake2F precompile address.
pub const BLAKE_2F: Address = address!("0000000000000000000000000000000000000009");

/// The PointEvaluation precompile address.
pub const POINT_EVALUATION: Address = address!("000000000000000000000000000000000000000a");

/// Precompile addresses.
pub const PRECOMPILES: &[Address] = &[
    EC_RECOVER,
    SHA_256,
    RIPEMD_160,
    IDENTITY,
    MOD_EXP,
    EC_ADD,
    EC_MUL,
    EC_PAIRING,
    BLAKE_2F,
    POINT_EVALUATION,
    ALPHANET_P256_ADDRESS,
];

/// [EIP-7212](https://eips.ethereum.org/EIPS/eip-7212) secp256r1 precompile address on Alphanet.
///
/// <https://github.com/paradigmxyz/alphanet/blob/5b675ee2b5214f157a62aee2b28fc7ca73e23561/crates/precompile/src/addresses.rs#L3>
pub const ALPHANET_P256_ADDRESS: Address = address!("0000000000000000000000000000000000000014");

/// Wrapper around revm P256 precompile, matching EIP-7212 spec.
///
/// Per Optimism implementation, P256 precompile returns empty bytes on failure, but per EIP-7212 it
/// should be 32 bytes of zeros instead.
pub fn p256_verify(input: &Bytes, gas_limit: u64) -> PrecompileResult {
    revm_p256_verify(input, gas_limit).map(|mut result| {
        if result.bytes.is_empty() {
            result.bytes = B256::default().into();
        }

        result
    })
}

/// [EIP-7212](https://eips.ethereum.org/EIPS/eip-7212#specification) secp256r1 precompile.
pub const ALPHANET_P256: PrecompileWithAddress =
    PrecompileWithAddress(ALPHANET_P256_ADDRESS, Precompile::Standard(p256_verify));
