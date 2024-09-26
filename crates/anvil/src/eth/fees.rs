use crate::eth::{
    backend::{info::StorageInfo, notifications::NewBlockNotifications},
    error::BlockchainError,
};
use alloy_consensus::Header;
use alloy_eips::{
    calc_next_block_base_fee, eip1559::BaseFeeParams, eip4844::MAX_DATA_GAS_PER_BLOCK,
};
use alloy_primitives::B256;
use anvil_core::eth::transaction::TypedTransaction;
use foundry_evm::revm::primitives::{BlobExcessGasAndPrice, SpecId};
use futures::StreamExt;
use parking_lot::{Mutex, RwLock};
use std::{
    collections::BTreeMap,
    fmt,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

/// Maximum number of entries in the fee history cache
pub const MAX_FEE_HISTORY_CACHE_SIZE: u64 = 2048u64;

/// Initial base fee for EIP-1559 blocks.
pub const INITIAL_BASE_FEE: u128 = 1_000_000_000;

/// Initial default gas price for the first block
pub const INITIAL_GAS_PRICE: u128 = 1_875_000_000;

/// Bounds the amount the base fee can change between blocks.
pub const BASE_FEE_CHANGE_DENOMINATOR: u128 = 8;

/// Minimum suggested priority fee
pub const MIN_SUGGESTED_PRIORITY_FEE: u128 = 1e9 as u128;

pub fn default_elasticity() -> f64 {
    1f64 / BaseFeeParams::ethereum().elasticity_multiplier as f64
}

/// Stores the fee related information
#[derive(Clone, Debug)]
pub struct FeeManager {
    /// Hardfork identifier
    spec_id: SpecId,
    /// Tracks the base fee for the next block post London
    ///
    /// This value will be updated after a new block was mined
    base_fee: Arc<RwLock<u128>>,
    /// Tracks the excess blob gas, and the base fee, for the next block post Cancun
    ///
    /// This value will be updated after a new block was mined
    blob_excess_gas_and_price: Arc<RwLock<foundry_evm::revm::primitives::BlobExcessGasAndPrice>>,
    /// The base price to use Pre London
    ///
    /// This will be constant value unless changed manually
    gas_price: Arc<RwLock<u128>>,
    elasticity: Arc<RwLock<f64>>,
}

impl FeeManager {
    pub fn new(
        spec_id: SpecId,
        base_fee: u128,
        gas_price: u128,
        blob_excess_gas_and_price: BlobExcessGasAndPrice,
    ) -> Self {
        Self {
            spec_id,
            base_fee: Arc::new(RwLock::new(base_fee)),
            gas_price: Arc::new(RwLock::new(gas_price)),
            blob_excess_gas_and_price: Arc::new(RwLock::new(blob_excess_gas_and_price)),
            elasticity: Arc::new(RwLock::new(default_elasticity())),
        }
    }

    pub fn elasticity(&self) -> f64 {
        *self.elasticity.read()
    }

    /// Returns true for post London
    pub fn is_eip1559(&self) -> bool {
        (self.spec_id as u8) >= (SpecId::LONDON as u8)
    }

    pub fn is_eip4844(&self) -> bool {
        (self.spec_id as u8) >= (SpecId::CANCUN as u8)
    }

    /// Calculates the current blob gas price
    pub fn blob_gas_price(&self) -> u128 {
        if self.is_eip4844() {
            self.base_fee_per_blob_gas()
        } else {
            0
        }
    }

    pub fn base_fee(&self) -> u128 {
        if self.is_eip1559() {
            *self.base_fee.read()
        } else {
            0
        }
    }

    /// Raw base gas price
    pub fn raw_gas_price(&self) -> u128 {
        *self.gas_price.read()
    }

    pub fn excess_blob_gas_and_price(&self) -> Option<BlobExcessGasAndPrice> {
        if self.is_eip4844() {
            Some(self.blob_excess_gas_and_price.read().clone())
        } else {
            None
        }
    }

    pub fn base_fee_per_blob_gas(&self) -> u128 {
        if self.is_eip4844() {
            self.blob_excess_gas_and_price.read().blob_gasprice
        } else {
            0
        }
    }

    /// Returns the current gas price
    pub fn set_gas_price(&self, price: u128) {
        let mut gas = self.gas_price.write();
        *gas = price;
    }

    /// Returns the current base fee
    pub fn set_base_fee(&self, fee: u128) {
        trace!(target: "backend::fees", "updated base fee {:?}", fee);
        let mut base = self.base_fee.write();
        *base = fee;
    }

    /// Sets the current blob excess gas and price
    pub fn set_blob_excess_gas_and_price(&self, blob_excess_gas_and_price: BlobExcessGasAndPrice) {
        trace!(target: "backend::fees", "updated blob base fee {:?}", blob_excess_gas_and_price);
        let mut base = self.blob_excess_gas_and_price.write();
        *base = blob_excess_gas_and_price;
    }

    /// Calculates the base fee for the next block
    pub fn get_next_block_base_fee_per_gas(
        &self,
        gas_used: u128,
        gas_limit: u128,
        last_fee_per_gas: u128,
    ) -> u128 {
        // It's naturally impossible for base fee to be 0;
        // It means it was set by the user deliberately and therefore we treat it as a constant.
        // Therefore, we skip the base fee calculation altogether and we return 0.
        if self.base_fee() == 0 {
            return 0
        }
        calculate_next_block_base_fee(gas_used, gas_limit, last_fee_per_gas)
    }

    /// Calculates the next block blob base fee, using the provided excess blob gas
    pub fn get_next_block_blob_base_fee_per_gas(&self, excess_blob_gas: u128) -> u128 {
        crate::revm::primitives::calc_blob_gasprice(excess_blob_gas as u64)
    }

    /// Calculates the next block blob excess gas, using the provided parent blob gas used and
    /// parent blob excess gas
    pub fn get_next_block_blob_excess_gas(
        &self,
        blob_gas_used: u128,
        blob_excess_gas: u128,
    ) -> u64 {
        crate::revm::primitives::calc_excess_blob_gas(blob_gas_used as u64, blob_excess_gas as u64)
    }
}

/// Calculate base fee for next block. [EIP-1559](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1559.md) spec
pub fn calculate_next_block_base_fee(gas_used: u128, gas_limit: u128, base_fee: u128) -> u128 {
    calc_next_block_base_fee(gas_used, gas_limit, base_fee, BaseFeeParams::ethereum())
}

/// An async service that takes care of the `FeeHistory` cache
pub struct FeeHistoryService {
    /// incoming notifications about new blocks
    new_blocks: NewBlockNotifications,
    /// contains all fee history related entries
    cache: FeeHistoryCache,
    /// number of items to consider
    fee_history_limit: u64,
    /// a type that can fetch ethereum-storage data
    storage_info: StorageInfo,
}

impl FeeHistoryService {
    pub fn new(
        new_blocks: NewBlockNotifications,
        cache: FeeHistoryCache,
        storage_info: StorageInfo,
    ) -> Self {
        Self { new_blocks, cache, fee_history_limit: MAX_FEE_HISTORY_CACHE_SIZE, storage_info }
    }

    /// Returns the configured history limit
    pub fn fee_history_limit(&self) -> u64 {
        self.fee_history_limit
    }

    /// Inserts a new cache entry for the given block
    pub(crate) fn insert_cache_entry_for_block(&self, hash: B256, header: &Header) {
        let (result, block_number) = self.create_cache_entry(hash, header);
        self.insert_cache_entry(result, block_number);
    }

    /// Create a new history entry for the block
    fn create_cache_entry(
        &self,
        hash: B256,
        header: &Header,
    ) -> (FeeHistoryCacheItem, Option<u64>) {
        // percentile list from 0.0 to 100.0 with a 0.5 resolution.
        // this will create 200 percentile points
        let reward_percentiles: Vec<f64> = {
            let mut percentile: f64 = 0.0;
            (0..=200)
                .map(|_| {
                    let val = percentile;
                    percentile += 0.5;
                    val
                })
                .collect()
        };

        let mut block_number: Option<u64> = None;
        let base_fee = header.base_fee_per_gas.unwrap_or_default();
        let excess_blob_gas = header.excess_blob_gas;
        let blob_gas_used = header.blob_gas_used;
        let base_fee_per_blob_gas = header.blob_fee();
        let mut item = FeeHistoryCacheItem {
            base_fee,
            gas_used_ratio: 0f64,
            blob_gas_used_ratio: 0f64,
            rewards: Vec::new(),
            excess_blob_gas,
            base_fee_per_blob_gas,
            blob_gas_used,
        };

        let current_block = self.storage_info.block(hash);
        let current_receipts = self.storage_info.receipts(hash);

        if let (Some(block), Some(receipts)) = (current_block, current_receipts) {
            block_number = Some(block.header.number);

            let gas_used = block.header.gas_used as f64;
            let blob_gas_used = block.header.blob_gas_used.map(|g| g as f64);
            item.gas_used_ratio = gas_used / block.header.gas_limit as f64;
            item.blob_gas_used_ratio =
                blob_gas_used.map(|g| g / MAX_DATA_GAS_PER_BLOCK as f64).unwrap_or(0 as f64);

            // extract useful tx info (gas_used, effective_reward)
            let mut transactions: Vec<(u128, u128)> = receipts
                .iter()
                .enumerate()
                .map(|(i, receipt)| {
                    let gas_used = receipt.cumulative_gas_used();
                    let effective_reward = match block.transactions.get(i).map(|tx| &tx.transaction)
                    {
                        Some(TypedTransaction::Legacy(t)) => {
                            t.tx().gas_price.saturating_sub(base_fee)
                        }
                        Some(TypedTransaction::EIP2930(t)) => {
                            t.tx().gas_price.saturating_sub(base_fee)
                        }
                        Some(TypedTransaction::EIP1559(t)) => t
                            .tx()
                            .max_priority_fee_per_gas
                            .min(t.tx().max_fee_per_gas.saturating_sub(base_fee)),
                        // TODO: This probably needs to be extended to extract 4844 info.
                        Some(TypedTransaction::EIP4844(t)) => t
                            .tx()
                            .tx()
                            .max_priority_fee_per_gas
                            .min(t.tx().tx().max_fee_per_gas.saturating_sub(base_fee)),
                        Some(TypedTransaction::EIP7702(t)) => t
                            .tx()
                            .max_priority_fee_per_gas
                            .min(t.tx().max_fee_per_gas.saturating_sub(base_fee)),
                        Some(TypedTransaction::Deposit(_)) => 0,
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
                    let target_gas = (p * gas_used / 100f64) as u128;
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
            // add the imported block.
            pin.insert_cache_entry_for_block(notification.hash, notification.header.as_ref());
        }

        Poll::Pending
    }
}

pub type FeeHistoryCache = Arc<Mutex<BTreeMap<u64, FeeHistoryCacheItem>>>;

/// A single item in the whole fee history cache
#[derive(Clone, Debug)]
pub struct FeeHistoryCacheItem {
    pub base_fee: u128,
    pub gas_used_ratio: f64,
    pub base_fee_per_blob_gas: Option<u128>,
    pub blob_gas_used_ratio: f64,
    pub excess_blob_gas: Option<u128>,
    pub blob_gas_used: Option<u128>,
    pub rewards: Vec<u128>,
}

#[derive(Clone, Default)]
pub struct FeeDetails {
    pub gas_price: Option<u128>,
    pub max_fee_per_gas: Option<u128>,
    pub max_priority_fee_per_gas: Option<u128>,
    pub max_fee_per_blob_gas: Option<u128>,
}

impl FeeDetails {
    /// All values zero
    pub fn zero() -> Self {
        Self {
            gas_price: Some(0),
            max_fee_per_gas: Some(0),
            max_priority_fee_per_gas: Some(0),
            max_fee_per_blob_gas: None,
        }
    }

    /// If neither `gas_price` nor `max_fee_per_gas` is `Some`, this will set both to `0`
    pub fn or_zero_fees(self) -> Self {
        let Self { gas_price, max_fee_per_gas, max_priority_fee_per_gas, max_fee_per_blob_gas } =
            self;

        let no_fees = gas_price.is_none() && max_fee_per_gas.is_none();
        let gas_price = if no_fees { Some(0) } else { gas_price };
        let max_fee_per_gas = if no_fees { Some(0) } else { max_fee_per_gas };
        let max_fee_per_blob_gas = if no_fees { None } else { max_fee_per_blob_gas };

        Self { gas_price, max_fee_per_gas, max_priority_fee_per_gas, max_fee_per_blob_gas }
    }

    /// Turns this type into a tuple
    pub fn split(self) -> (Option<u128>, Option<u128>, Option<u128>, Option<u128>) {
        let Self { gas_price, max_fee_per_gas, max_priority_fee_per_gas, max_fee_per_blob_gas } =
            self;
        (gas_price, max_fee_per_gas, max_priority_fee_per_gas, max_fee_per_blob_gas)
    }

    /// Creates a new instance from the request's gas related values
    pub fn new(
        request_gas_price: Option<u128>,
        request_max_fee: Option<u128>,
        request_priority: Option<u128>,
        max_fee_per_blob_gas: Option<u128>,
    ) -> Result<Self, BlockchainError> {
        match (request_gas_price, request_max_fee, request_priority, max_fee_per_blob_gas) {
            (gas_price, None, None, None) => {
                // Legacy request, all default to gas price.
                Ok(Self {
                    gas_price,
                    max_fee_per_gas: gas_price,
                    max_priority_fee_per_gas: gas_price,
                    max_fee_per_blob_gas: None,
                })
            }
            (_, max_fee, max_priority, None) => {
                // eip-1559
                // Ensure `max_priority_fee_per_gas` is less or equal to `max_fee_per_gas`.
                if let Some(max_priority) = max_priority {
                    let max_fee = max_fee.unwrap_or_default();
                    if max_priority > max_fee {
                        return Err(BlockchainError::InvalidFeeInput)
                    }
                }
                Ok(Self {
                    gas_price: max_fee,
                    max_fee_per_gas: max_fee,
                    max_priority_fee_per_gas: max_priority,
                    max_fee_per_blob_gas: None,
                })
            }
            (_, max_fee, max_priority, max_fee_per_blob_gas) => {
                // eip-1559
                // Ensure `max_priority_fee_per_gas` is less or equal to `max_fee_per_gas`.
                if let Some(max_priority) = max_priority {
                    let max_fee = max_fee.unwrap_or_default();
                    if max_priority > max_fee {
                        return Err(BlockchainError::InvalidFeeInput)
                    }
                }
                Ok(Self {
                    gas_price: max_fee,
                    max_fee_per_gas: max_fee,
                    max_priority_fee_per_gas: max_priority,
                    max_fee_per_blob_gas,
                })
            }
        }
    }
}

impl fmt::Debug for FeeDetails {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(fmt, "Fees {{ ")?;
        write!(fmt, "gas_price: {:?}, ", self.gas_price)?;
        write!(fmt, "max_fee_per_gas: {:?}, ", self.max_fee_per_gas)?;
        write!(fmt, "max_priority_fee_per_gas: {:?}, ", self.max_priority_fee_per_gas)?;
        write!(fmt, "}}")?;
        Ok(())
    }
}
