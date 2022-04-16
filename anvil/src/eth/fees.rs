use crate::eth::{backend::notifications::NewBlockNotifications, error::BlockchainError};
use ethers::types::U256;
use parking_lot::Mutex;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

/// An async service that takes care of the `FeeHistory` cache
pub struct FeeHistoryService {
    /// incoming notifications about new blocks
    new_blocks: NewBlockNotifications,
    /// contains all fee history related entries
    cache: FeeHistoryCache,
}

// === impl FeeHistoryService ===

impl FeeHistoryService {
    pub fn new(new_blocks: NewBlockNotifications, cache: FeeHistoryCache) -> Self {
        Self { new_blocks, cache }
    }
}

// An endless future that listens for new blocks and updates the cache
impl Future for FeeHistoryService {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
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
