//! The block environment recorded in fork caches and Anvil state dumps.
//!
//! Its serde layout matches the block environment previously written by REVM, so existing fork
//! caches and state dumps stay valid. The fork cache compares it to decide whether a cache matches.

use alloy_eips::{
    eip4844::{BLOB_TX_MIN_BLOB_GASPRICE, fake_exponential},
    eip7691::BLOB_GASPRICE_UPDATE_FRACTION_PECTRA,
};
use alloy_primitives::{Address, B256, U256};
use serde::{Deserialize, Serialize};

/// Excess blob gas of a block and the blob gas price derived from it.
///
/// Its serde layout is persisted and must stay stable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlobExcessGasAndPrice {
    /// The excess blob gas of the block.
    pub excess_blob_gas: u64,
    /// The blob gas price derived from `excess_blob_gas`.
    pub blob_gasprice: u128,
}

impl BlobExcessGasAndPrice {
    /// Creates a new instance, deriving the blob gas price from the excess blob gas.
    pub fn new(excess_blob_gas: u64, blob_base_fee_update_fraction: u64) -> Self {
        Self {
            excess_blob_gas,
            blob_gasprice: fake_exponential(
                BLOB_TX_MIN_BLOB_GASPRICE,
                excess_blob_gas as u128,
                blob_base_fee_update_fraction as u128,
            ),
        }
    }
}

/// A persisted block environment.
///
/// Its serde layout is persisted and must stay stable.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlockEnv {
    /// Block number.
    pub number: U256,
    /// Block producer.
    pub beneficiary: Address,
    /// Block timestamp in seconds.
    pub timestamp: U256,
    /// Block gas limit.
    pub gas_limit: u64,
    /// Block base fee.
    pub basefee: u64,
    /// Block difficulty, before the merge.
    pub difficulty: U256,
    /// Block `prevrandao`, after the merge.
    pub prevrandao: Option<B256>,
    /// Excess blob gas and blob gas price, after Cancun.
    pub blob_excess_gas_and_price: Option<BlobExcessGasAndPrice>,
    /// Slot number, after Amsterdam.
    #[serde(default)]
    pub slot_num: u64,
}

impl Default for BlockEnv {
    fn default() -> Self {
        Self {
            number: U256::ZERO,
            beneficiary: Address::ZERO,
            timestamp: U256::ONE,
            gas_limit: u64::MAX,
            basefee: 0,
            difficulty: U256::ZERO,
            prevrandao: Some(B256::ZERO),
            blob_excess_gas_and_price: Some(BlobExcessGasAndPrice::new(
                0,
                BLOB_GASPRICE_UPDATE_FRACTION_PECTRA as u64,
            )),
            slot_num: 0,
        }
    }
}

impl BlockEnv {
    /// Sets the excess blob gas and derives the blob gas price from it.
    pub fn set_blob_excess_gas_and_price(
        &mut self,
        excess_blob_gas: u64,
        base_fee_update_fraction: u64,
    ) {
        self.blob_excess_gas_and_price =
            Some(BlobExcessGasAndPrice::new(excess_blob_gas, base_fee_update_fraction));
    }

    /// Returns the excess blob gas, if blobs are enabled.
    pub fn blob_excess_gas(&self) -> Option<u64> {
        self.blob_excess_gas_and_price.map(|blob| blob.excess_blob_gas)
    }

    /// Returns the blob gas price, if blobs are enabled.
    pub fn blob_gasprice(&self) -> Option<u128> {
        self.blob_excess_gas_and_price.map(|blob| blob.blob_gasprice)
    }
}
