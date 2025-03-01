//! Contains constants and utility functions for [EIP-7840](https://github.com/ethereum/EIPs/tree/master/EIPS/eip-7840.md)

use crate::{eip4844, eip4844::DATA_GAS_PER_BLOB, eip7691};

// helpers for serde
#[cfg(feature = "serde")]
const DEFAULT_BLOB_FEE_GETTER: fn() -> u128 = || eip4844::BLOB_TX_MIN_BLOB_GASPRICE;
#[cfg(feature = "serde")]
const IS_DEFAULT_BLOB_FEE: fn(&u128) -> bool = |&x| x == eip4844::BLOB_TX_MIN_BLOB_GASPRICE;

/// Configuration for the blob-related calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BlobParams {
    /// Target blob count for the block.
    #[cfg_attr(feature = "serde", serde(rename = "target"))]
    pub target_blob_count: u64,
    /// Max blob count for the block.
    #[cfg_attr(feature = "serde", serde(rename = "max"))]
    pub max_blob_count: u64,
    /// Update fraction for excess blob gas calculation.
    #[cfg_attr(feature = "serde", serde(rename = "baseFeeUpdateFraction"))]
    pub update_fraction: u128,
    /// Minimum gas price for a data blob.
    ///
    /// Not required per EIP-7840 and assumed to be the default
    /// [`eip4844::BLOB_TX_MIN_BLOB_GASPRICE`] if not set.
    #[cfg_attr(
        feature = "serde",
        serde(default = "DEFAULT_BLOB_FEE_GETTER", skip_serializing_if = "IS_DEFAULT_BLOB_FEE")
    )]
    pub min_blob_fee: u128,
}

impl BlobParams {
    /// Returns [`BlobParams`] configuration activated with Cancun hardfork.
    pub const fn cancun() -> Self {
        Self {
            target_blob_count: eip4844::TARGET_BLOBS_PER_BLOCK,
            max_blob_count: eip4844::MAX_BLOBS_PER_BLOCK as u64,
            update_fraction: eip4844::BLOB_GASPRICE_UPDATE_FRACTION,
            min_blob_fee: eip4844::BLOB_TX_MIN_BLOB_GASPRICE,
        }
    }

    /// Returns [`BlobParams`] configuration activated with Prague hardfork.
    pub const fn prague() -> Self {
        Self {
            target_blob_count: eip7691::TARGET_BLOBS_PER_BLOCK_ELECTRA,
            max_blob_count: eip7691::MAX_BLOBS_PER_BLOCK_ELECTRA,
            update_fraction: eip7691::BLOB_GASPRICE_UPDATE_FRACTION_PECTRA,
            min_blob_fee: eip4844::BLOB_TX_MIN_BLOB_GASPRICE,
        }
    }

    /// Returns the maximum available blob gas in a block.
    ///
    /// This is `max blob count * DATA_GAS_PER_BLOB`
    pub const fn max_blob_gas_per_block(&self) -> u64 {
        self.max_blob_count * DATA_GAS_PER_BLOB
    }

    /// Returns the blob gas target per block.
    ///
    /// This is `target blob count * DATA_GAS_PER_BLOB`
    pub const fn target_blob_gas_per_block(&self) -> u64 {
        self.target_blob_count * DATA_GAS_PER_BLOB
    }

    /// Calculates the `excess_blob_gas` value for the next block based on the current block
    /// `excess_blob_gas` and `blob_gas_used`.
    #[inline]
    pub const fn next_block_excess_blob_gas(
        &self,
        excess_blob_gas: u64,
        blob_gas_used: u64,
    ) -> u64 {
        (excess_blob_gas + blob_gas_used)
            .saturating_sub(eip4844::DATA_GAS_PER_BLOB * self.target_blob_count)
    }

    /// Calculates the blob fee for block based on its `excess_blob_gas`.
    #[inline]
    pub const fn calc_blob_fee(&self, excess_blob_gas: u64) -> u128 {
        eip4844::fake_exponential(self.min_blob_fee, excess_blob_gas as u128, self.update_fraction)
    }
}
