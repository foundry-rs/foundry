//! Contains constants and utility functions for [EIP-7691](https://eips.ethereum.org/EIPS/eip-7691)

use crate::eip4844::{fake_exponential, BLOB_TX_MIN_BLOB_GASPRICE};

/// CL-enforced target blobs per block after Pectra hardfork activation.
pub const TARGET_BLOBS_PER_BLOCK_ELECTRA: u64 = 6;

/// CL-enforced maximum blobs per block after Pectra hardfork activation.
pub const MAX_BLOBS_PER_BLOCK_ELECTRA: u64 = 9;

/// Determines the maximum rate of change for blob fee after Pectra hardfork activation.
pub const BLOB_GASPRICE_UPDATE_FRACTION_PECTRA: u128 = 5007716;

/// Same as [`crate::eip4844::calc_blob_gasprice`] but uses the
/// [`BLOB_GASPRICE_UPDATE_FRACTION_PECTRA`].
#[inline]
pub const fn calc_blob_gasprice(excess_blob_gas: u64) -> u128 {
    fake_exponential(
        BLOB_TX_MIN_BLOB_GASPRICE,
        excess_blob_gas as u128,
        BLOB_GASPRICE_UPDATE_FRACTION_PECTRA,
    )
}
