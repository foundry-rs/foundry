use crate::eth::{
    backend::{info::StorageInfo, notifications::NewBlockNotifications},
    error::BlockchainError,
};
use anvil_core::eth::transaction::TypedTransaction;
use ethers::types::{H256, U256};
use futures::StreamExt;
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tracing::trace;

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

// === impl FeeManager ===

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

    /// Returns the configured history limit
    pub fn fee_history_limit(&self) -> u64 {
        self.fee_history_limit
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

        let mut block_number: Option<u64> = None;
        let base_fee = self.fees.base_fee();
        let mut item = FeeHistoryCacheItem {
            base_fee: base_fee.as_u64(),
            gas_used_ratio: 0f64,
            rewards: Vec::new(),
        };

        let current_block = self.storage_info.block(hash);
        let current_receipts = self.storage_info.receipts(hash);

        if let (Some(block), Some(receipts)) = (current_block, current_receipts) {
            block_number = Some(block.header.number.as_u64());

            let gas_used = block.header.gas_used.as_u64() as f64;
            let gas_limit = block.header.gas_limit.as_u64() as f64;

            let gas_target = gas_limit / elasticity;
            item.gas_used_ratio = gas_used / (gas_target * elasticity);

            // extract useful tx info (gas_used, effective_reward)
            let mut transactions: Vec<(u64, u64)> = receipts
                .iter()
                .enumerate()
                .map(|(i, receipt)| {
                    let gas_used = receipt.gas_used().as_u64();
                    let effective_reward = match block.transactions.get(i) {
                        Some(&TypedTransaction::Legacy(ref t)) => {
                            t.gas_price.saturating_sub(base_fee).as_u64()
                        }
                        Some(&TypedTransaction::EIP2930(ref t)) => {
                            t.gas_price.saturating_sub(base_fee).as_u64()
                        }
                        Some(&TypedTransaction::EIP1559(ref t)) => t
                            .max_priority_fee_per_gas
                            .min(t.max_fee_per_gas.saturating_sub(base_fee))
                            .as_u64(),
                        None => 0,
                    };

                    (gas_used, effective_reward)
                })
                .collect();

            // sort by effective reward asc
            transactions.sort_by(|(_, a), (_, b)| a.cmp(b));

            // calculate percentile rewards
            item.rewards = reward_percentiles
                .into_iter()
                .filter_map(|p| {
                    let target_gas = (p * gas_used / 100f64) as u64;
                    let mut sum_gas = 0;
                    for (gas_used, effective_reward) in transactions.iter().cloned() {
                        sum_gas += gas_used;
                        if target_gas <= sum_gas {
                            return Some(effective_reward)
                        }
                    }
                    None
                })
                .collect();
        } else {
            item.rewards = reward_percentiles.iter().map(|_| 0).collect();
        }
        (item, block_number)
    }

    fn insert_cache_entry(&self, item: FeeHistoryCacheItem, block_number: Option<u64>) {
        if let Some(block_number) = block_number {
            trace!(target: "fees", "insert new history item={:?} for {}", item, block_number);
            let mut cache = self.cache.lock();
            cache.insert(block_number, item);

            // adhere to cache limit
            let pop_next = block_number.saturating_sub(self.fee_history_limit);

            let num_remove = (cache.len() as u64).saturating_sub(self.fee_history_limit);
            for num in 0..num_remove {
                let key = pop_next - num;
                cache.remove(&key);
            }
        }
    }
}

// An endless future that listens for new blocks and updates the cache
impl Future for FeeHistoryService {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();

        while let Poll::Ready(Some(notification)) = pin.new_blocks.poll_next_unpin(cx) {
            let hash = notification.hash;
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
#[derive(Debug, Clone)]
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
    /// All values zero
    pub fn zero() -> Self {
        Self {
            gas_price: Some(U256::zero()),
            max_fee_per_gas: Some(U256::zero()),
            max_priority_fee_per_gas: Some(U256::zero()),
        }
    }

    /// If neither `gas_price` nor `max_fee_per_gas` is `Some`, this will set both to `0`
    pub fn or_zero_fees(self) -> Self {
        let FeeDetails { gas_price, max_fee_per_gas, max_priority_fee_per_gas } = self;

        let no_fees = gas_price.is_none() && max_fee_per_gas.is_none();
        let gas_price = if no_fees { Some(U256::zero()) } else { gas_price };
        let max_fee_per_gas = if no_fees { Some(U256::zero()) } else { max_fee_per_gas };

        Self { gas_price, max_fee_per_gas, max_priority_fee_per_gas }
    }

    /// Turns this type into a tuple
    pub fn split(self) -> (Option<U256>, Option<U256>, Option<U256>) {
        let Self { gas_price, max_fee_per_gas, max_priority_fee_per_gas } = self;
        (gas_price, max_fee_per_gas, max_priority_fee_per_gas)
    }

    /// Creates a new instance from the request's gas related values
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
