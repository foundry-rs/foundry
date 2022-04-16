use crate::eth::{
    backend::{info::StorageInfo, notifications::NewBlockNotifications},
    error::BlockchainError,
};
use ethers::types::{H256, U256};
use futures::{Stream, StreamExt};
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

/// Maximum number of entries in the fee history cache
pub const MAX_FEE_HISTORY_CACHE_SIZE: u64 = 2048u64;

/// Initial base fee for EIP-1559 blocks.
pub const INITIAL_BASE_FEE: u64 = 1_000_000_000;

/// Bounds the amount the base fee can change between blocks.
pub const BASE_FEE_CHANGE_DENOMINATOR: u64 = 8;

pub fn default_elasticity() -> f64 {
    1f64 / BASE_FEE_CHANGE_DENOMINATOR as f64
}

/// Stores the fee related information
#[derive(Debug, Clone)]
pub struct FeeManager {
    base_fee: Arc<RwLock<U256>>,
    gas_price: Arc<RwLock<U256>>,
    elasticity: Arc<RwLock<f64>>,
}

// === impl FeeConfig ===

impl FeeManager {
    pub fn new(base_fee: U256, gas_price: U256) -> Self {
        Self {
            base_fee: Arc::new(RwLock::new(base_fee)),
            gas_price: Arc::new(RwLock::new(gas_price)),
            elasticity: Arc::new(RwLock::new(default_elasticity())),
        }
    }

    pub fn elasticity(&self) -> f64 {
        *self.elasticity.read()
    }

    pub fn gas_price(&self) -> U256 {
        *self.gas_price.read()
    }

    pub fn base_fee(&self) -> U256 {
        *self.base_fee.read()
    }

    /// Returns the current gas price
    pub fn set_gas_price(&self, price: U256) {
        let mut gas = self.gas_price.write();
        *gas = price;
    }

    /// Returns the current base fee
    pub fn set_base_fee(&self, fee: U256) {
        let mut base = self.base_fee.write();
        *base = fee;
    }
}

/// An async service that takes care of the `FeeHistory` cache
pub struct FeeHistoryService {
    /// incoming notifications about new blocks
    new_blocks: NewBlockNotifications,
    /// contains all fee history related entries
    cache: FeeHistoryCache,
    /// number of items to consider
    fee_history_limit: u64,
    // current fee info
    fees: FeeManager,
    /// a type that can fetch ethereum-storage data
    storage_info: StorageInfo,
}

// === impl FeeHistoryService ===

impl FeeHistoryService {
    pub fn new(
        new_blocks: NewBlockNotifications,
        cache: FeeHistoryCache,
        fees: FeeManager,
        storage_info: StorageInfo,
    ) -> Self {
        Self {
            new_blocks,
            cache,
            fee_history_limit: MAX_FEE_HISTORY_CACHE_SIZE,
            fees,
            storage_info,
        }
    }

    fn create_cache_entry(
        &self,
        hash: H256,
        elasticity: f64,
    ) -> (FeeHistoryCacheItem, Option<u64>) {
        // percentile list from 0.0 to 100.0 with a 0.5 resolution.
        // this will create 200 percentile points
        let reward_percentiles: Vec<f64> = {
            let mut percentile: f64 = 0.0;
            (0..=200)
                .into_iter()
                .map(|_| {
                    let val = percentile;
                    percentile += 0.5;
                    val
                })
                .collect()
        };

        let base_fee = self.fees.base_fee();
        let mut item = FeeHistoryCacheItem {
            base_fee: base_fee.as_u64(),
            gas_used_ratio: 0f64,
            rewards: Vec::new(),
        };

        todo!()
    }

    fn insert_cache_entry(&mut self, item: FeeHistoryCacheItem, block_number: Option<u64>) {}
}

// An endless future that listens for new blocks and updates the cache
impl Future for FeeHistoryService {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();

        while let Poll::Ready(Some(notifiaction)) = pin.new_blocks.poll_next_unpin(cx) {
            let hash = notifiaction.hash;
            let elasticity = default_elasticity();

            // add the imported block.
            let (result, block_number) = pin.create_cache_entry(hash, elasticity);
            pin.insert_cache_entry(result, block_number)
        }

        Poll::Pending
    }
}

/// Response of `eth_feeHistory`
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
// See also <https://docs.alchemy.com/alchemy/apis/ethereum/eth_feehistory>
pub struct FeeHistory {
    ///  Lowest number block of the returned range.
    pub oldest_block: U256,
    /// An array of block base fees per gas. This includes the next block after the newest of the
    /// returned range, because this value can be derived from the newest block. Zeroes are
    /// returned for pre-EIP-1559 blocks.
    pub base_fee_per_gas: Vec<U256>,
    /// An array of block gas used ratios. These are calculated as the ratio of gasUsed and
    /// gasLimit.
    pub gas_used_ratio: Vec<f64>,
    /// (Optional) An array of effective priority fee per gas data points from a single block. All
    /// zeroes are returned if the block is empty.
    pub reward: Option<Vec<Vec<U256>>>,
}

pub type FeeHistoryCache = Arc<Mutex<BTreeMap<u64, FeeHistoryCacheItem>>>;

/// A single item in the whole fee history cache
pub struct FeeHistoryCacheItem {
    pub base_fee: u64,
    pub gas_used_ratio: f64,
    pub rewards: Vec<u64>,
}

#[derive(Debug, Default, Clone)]
pub struct FeeDetails {
    pub gas_price: Option<U256>,
    pub max_fee_per_gas: Option<U256>,
    pub max_priority_fee_per_gas: Option<U256>,
}

impl FeeDetails {
    pub fn zero() -> Self {
        Self {
            gas_price: Some(U256::zero()),
            max_fee_per_gas: Some(U256::zero()),
            max_priority_fee_per_gas: Some(U256::zero()),
        }
    }

    pub fn new(
        request_gas_price: Option<U256>,
        request_max_fee: Option<U256>,
        request_priority: Option<U256>,
    ) -> Result<FeeDetails, BlockchainError> {
        match (request_gas_price, request_max_fee, request_priority) {
            (gas_price, None, None) => {
                // Legacy request, all default to gas price.
                Ok(FeeDetails {
                    gas_price,
                    max_fee_per_gas: gas_price,
                    max_priority_fee_per_gas: gas_price,
                })
            }
            (_, max_fee, max_priority) => {
                // eip-1559
                // Ensure `max_priority_fee_per_gas` is less or equal to `max_fee_per_gas`.
                if let Some(max_priority) = max_priority {
                    let max_fee = max_fee.unwrap_or_default();
                    if max_priority > max_fee {
                        return Err(BlockchainError::InvalidFeeInput)
                    }
                }
                Ok(FeeDetails {
                    gas_price: max_fee,
                    max_fee_per_gas: max_fee,
                    max_priority_fee_per_gas: max_priority,
                })
            }
        }
    }
}
