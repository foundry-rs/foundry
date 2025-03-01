//! [EIP-4844] constants and helpers.
//!
//! [EIP-4844]: https://eips.ethereum.org/EIPS/eip-4844

/// Module houses the KZG settings, enabling Custom and Default
#[cfg(feature = "kzg")]
pub mod env_settings;
/// This module contains functions and types used for parsing and utilizing the [Trusted Setup]( https://ceremony.ethereum.org/) for the `KzgSettings`.
#[cfg(feature = "kzg")]
pub mod trusted_setup_points;

/// Builder and utils for the [EIP-4844 Blob Transaction](https://eips.ethereum.org/EIPS/eip-4844#blob-transaction)
pub mod builder;
pub mod utils;

mod engine;
pub use engine::*;

/// Contains sidecar related types
#[cfg(feature = "kzg-sidecar")]
mod sidecar;
#[cfg(feature = "kzg-sidecar")]
pub use sidecar::*;

use alloy_primitives::{b256, FixedBytes, B256, U256};

use crate::eip7840;

/// The modulus of the BLS group used in the KZG commitment scheme. All field
/// elements contained in a blob MUST be STRICTLY LESS than this value.
pub const BLS_MODULUS_BYTES: B256 =
    b256!("73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001");

/// The modulus of the BLS group used in the KZG commitment scheme. All field
/// elements contained in a blob MUST be STRICTLY LESS than this value.
pub const BLS_MODULUS: U256 = U256::from_be_bytes(BLS_MODULUS_BYTES.0);

/// Size a single field element in bytes.
pub const FIELD_ELEMENT_BYTES: u64 = 32;

/// Size a single field element in bytes.
pub const FIELD_ELEMENT_BYTES_USIZE: usize = FIELD_ELEMENT_BYTES as usize;

/// How many field elements are stored in a single data blob.
pub const FIELD_ELEMENTS_PER_BLOB: u64 = 4096;

/// Number of usable bits in a field element. The top two bits are always zero.
pub const USABLE_BITS_PER_FIELD_ELEMENT: usize = 254;

/// The number of usable bytes in a single data blob. This is the number of
/// bytes you can encode in a blob without any field element being >=
/// [`BLS_MODULUS`].
pub const USABLE_BYTES_PER_BLOB: usize =
    USABLE_BITS_PER_FIELD_ELEMENT * FIELD_ELEMENTS_PER_BLOB as usize / 8;

/// Gas consumption of a single data blob.
pub const DATA_GAS_PER_BLOB: u64 = 131_072u64; // 32*4096 = 131072 == 2^17 == 0x20000

/// How many bytes are in a blob
/// Same as [DATA_GAS_PER_BLOB], but as an usize
pub const BYTES_PER_BLOB: usize = 131_072;

/// Maximum data gas for data blobs in a single block.
pub const MAX_DATA_GAS_PER_BLOCK: u64 = 786_432u64; // 0xC0000 = 6 * 0x20000

/// Target data gas for data blobs in a single block.
pub const TARGET_DATA_GAS_PER_BLOCK: u64 = 393_216u64; // 0x60000 = 3 * 0x20000

/// Maximum number of data blobs in a single block.
pub const MAX_BLOBS_PER_BLOCK: usize = (MAX_DATA_GAS_PER_BLOCK / DATA_GAS_PER_BLOB) as usize; // 786432 / 131072  = 6

/// Target number of data blobs in a single block.
pub const TARGET_BLOBS_PER_BLOCK: u64 = TARGET_DATA_GAS_PER_BLOCK / DATA_GAS_PER_BLOB; // 393216 / 131072 = 3

/// Determines the maximum rate of change for blob fee
pub const BLOB_GASPRICE_UPDATE_FRACTION: u128 = 3_338_477u128; // 3338477

/// Minimum gas price for a data blob
pub const BLOB_TX_MIN_BLOB_GASPRICE: u128 = 1u128;

/// Commitment version of a KZG commitment
pub const VERSIONED_HASH_VERSION_KZG: u8 = 0x01;

/// How many bytes are in a commitment
pub const BYTES_PER_COMMITMENT: usize = 48;

/// How many bytes are in a proof
pub const BYTES_PER_PROOF: usize = 48;

/// A Blob serialized as 0x-prefixed hex string
pub type Blob = FixedBytes<BYTES_PER_BLOB>;

/// Helper function to deserialize boxed blobs.
#[cfg(feature = "serde")]
pub fn deserialize_blob<'de, D>(deserializer: D) -> Result<alloc::boxed::Box<Blob>, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    use serde::Deserialize;
    let raw_blob = <alloy_primitives::Bytes>::deserialize(deserializer)?;
    let blob = alloc::boxed::Box::new(
        Blob::try_from(raw_blob.as_ref()).map_err(serde::de::Error::custom)?,
    );
    Ok(blob)
}

/// A commitment/proof serialized as 0x-prefixed hex string
pub type Bytes48 = FixedBytes<48>;

/// Calculates the versioned hash for a KzgCommitment of 48 bytes.
///
/// Specified in [EIP-4844](https://eips.ethereum.org/EIPS/eip-4844#header-extension)
///
/// # Panics
///
/// If the given commitment is not 48 bytes long.
#[cfg(feature = "sha2")]
pub fn kzg_to_versioned_hash(commitment: &[u8]) -> B256 {
    use sha2::Digest;

    debug_assert_eq!(commitment.len(), 48, "commitment length is not 48");
    let mut res = sha2::Sha256::digest(commitment);
    res[0] = VERSIONED_HASH_VERSION_KZG;
    B256::new(res.into())
}

/// Calculates the `excess_blob_gas` from the parent header's `blob_gas_used` and `excess_blob_gas`.
///
/// See also [the EIP-4844 helpers](https://eips.ethereum.org/EIPS/eip-4844#helpers)
/// (`calc_excess_blob_gas`).
#[inline]
pub const fn calc_excess_blob_gas(parent_excess_blob_gas: u64, parent_blob_gas_used: u64) -> u64 {
    eip7840::BlobParams::cancun()
        .next_block_excess_blob_gas(parent_excess_blob_gas, parent_blob_gas_used)
}

/// Calculates the blob gas price from the header's excess blob gas field.
///
/// See also [the EIP-4844 helpers](https://eips.ethereum.org/EIPS/eip-4844#helpers)
/// (`get_blob_gasprice`).
#[inline]
pub const fn calc_blob_gasprice(excess_blob_gas: u64) -> u128 {
    eip7840::BlobParams::cancun().calc_blob_fee(excess_blob_gas)
}

/// Approximates `factor * e ** (numerator / denominator)` using Taylor expansion.
///
/// This is used to calculate the blob price.
///
/// See also [the EIP-4844 helpers](https://eips.ethereum.org/EIPS/eip-4844#helpers)
/// (`fake_exponential`).
///
/// # Panics
///
/// This function panics if `denominator` is zero.
#[inline]
pub const fn fake_exponential(factor: u128, numerator: u128, denominator: u128) -> u128 {
    assert!(denominator != 0, "attempt to divide by zero");

    let mut i = 1;
    let mut output = 0;
    let mut numerator_accum = factor * denominator;
    while numerator_accum > 0 {
        output += numerator_accum;

        // Denominator is asserted as not zero at the start of the function.
        numerator_accum = (numerator_accum * numerator) / (denominator * i);
        i += 1;
    }
    output / denominator
}

#[cfg(test)]
mod tests {
    use super::*;

    // https://github.com/ethereum/go-ethereum/blob/28857080d732857030eda80c69b9ba2c8926f221/consensus/misc/eip4844/eip4844_test.go#L27
    #[test]
    fn test_calc_excess_blob_gas() {
        for t @ &(excess, blobs, expected) in &[
            // The excess blob gas should not increase from zero if the used blob
            // slots are below - or equal - to the target.
            (0, 0, 0),
            (0, 1, 0),
            (0, TARGET_DATA_GAS_PER_BLOCK / DATA_GAS_PER_BLOB, 0),
            // If the target blob gas is exceeded, the excessBlobGas should increase
            // by however much it was overshot
            (0, (TARGET_DATA_GAS_PER_BLOCK / DATA_GAS_PER_BLOB) + 1, DATA_GAS_PER_BLOB),
            (1, (TARGET_DATA_GAS_PER_BLOCK / DATA_GAS_PER_BLOB) + 1, DATA_GAS_PER_BLOB + 1),
            (1, (TARGET_DATA_GAS_PER_BLOCK / DATA_GAS_PER_BLOB) + 2, 2 * DATA_GAS_PER_BLOB + 1),
            // The excess blob gas should decrease by however much the target was
            // under-shot, capped at zero.
            (
                TARGET_DATA_GAS_PER_BLOCK,
                TARGET_DATA_GAS_PER_BLOCK / DATA_GAS_PER_BLOB,
                TARGET_DATA_GAS_PER_BLOCK,
            ),
            (
                TARGET_DATA_GAS_PER_BLOCK,
                (TARGET_DATA_GAS_PER_BLOCK / DATA_GAS_PER_BLOB) - 1,
                TARGET_DATA_GAS_PER_BLOCK - DATA_GAS_PER_BLOB,
            ),
            (
                TARGET_DATA_GAS_PER_BLOCK,
                (TARGET_DATA_GAS_PER_BLOCK / DATA_GAS_PER_BLOB) - 2,
                TARGET_DATA_GAS_PER_BLOCK - (2 * DATA_GAS_PER_BLOB),
            ),
            (DATA_GAS_PER_BLOB - 1, (TARGET_DATA_GAS_PER_BLOCK / DATA_GAS_PER_BLOB) - 1, 0),
        ] {
            let actual = calc_excess_blob_gas(excess, blobs * DATA_GAS_PER_BLOB);
            assert_eq!(actual, expected, "test: {t:?}");
        }
    }

    // https://github.com/ethereum/go-ethereum/blob/28857080d732857030eda80c69b9ba2c8926f221/consensus/misc/eip4844/eip4844_test.go#L60
    #[test]
    fn test_calc_blob_fee() {
        let blob_fee_vectors = &[
            (0, 1),
            (2314057, 1),
            (2314058, 2),
            (10 * 1024 * 1024, 23),
            // calc_blob_gasprice approximates `e ** (excess_blob_gas /
            // BLOB_GASPRICE_UPDATE_FRACTION)` using Taylor expansion
            //
            // to roughly find where boundaries will be hit:
            // 2 ** bits = e ** (excess_blob_gas / BLOB_GASPRICE_UPDATE_FRACTION)
            // excess_blob_gas = ln(2 ** bits) * BLOB_GASPRICE_UPDATE_FRACTION
            (148099578, 18446739238971471609), // output is just below the overflow
            (148099579, 18446744762204311910), // output is just after the overflow
            (161087488, 902580055246494526580),
        ];

        for &(excess, expected) in blob_fee_vectors {
            let actual = calc_blob_gasprice(excess);
            assert_eq!(actual, expected, "test: {excess}");
        }
    }

    // https://github.com/ethereum/go-ethereum/blob/28857080d732857030eda80c69b9ba2c8926f221/consensus/misc/eip4844/eip4844_test.go#L78
    #[test]
    fn fake_exp() {
        for t @ &(factor, numerator, denominator, expected) in &[
            (1u64, 0u64, 1u64, 1u128),
            (38493, 0, 1000, 38493),
            (0, 1234, 2345, 0),
            (1, 2, 1, 6), // approximate 7.389
            (1, 4, 2, 6),
            (1, 3, 1, 16), // approximate 20.09
            (1, 6, 2, 18),
            (1, 4, 1, 49), // approximate 54.60
            (1, 8, 2, 50),
            (10, 8, 2, 542), // approximate 540.598
            (11, 8, 2, 596), // approximate 600.58
            (1, 5, 1, 136),  // approximate 148.4
            (1, 5, 2, 11),   // approximate 12.18
            (2, 5, 2, 23),   // approximate 24.36
            (1, 50000000, 2225652, 5709098764),
            (1, 380928, BLOB_GASPRICE_UPDATE_FRACTION.try_into().unwrap(), 1),
        ] {
            let actual = fake_exponential(factor as u128, numerator as u128, denominator as u128);
            assert_eq!(actual, expected, "test: {t:?}");
        }
    }
}
