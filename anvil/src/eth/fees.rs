use crate::eth::{
    backend::{info::StorageInfo, notifications::NewBlockNotifications},
    error::BlockchainError,
};
use anvil_core::eth::transaction::TypedTransaction;
use ethers::types::{H256, U256};
use foundry_evm::revm::primitives::SpecId;
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
use tracing::trace;

/// Maximum number of entries in the fee history cache
pub const MAX_FEE_HISTORY_CACHE_SIZE: u64 = 2048u64;

/// Initial base fee for EIP-1559 blocks.
pub const INITIAL_BASE_FEE: u64 = 1_000_000_000;

/// Initial default gas price for the first block
pub const INITIAL_GAS_PRICE: u64 = 1_875_000_000;

/// Bounds the amount the base fee can change between blocks.
pub const BASE_FEE_CHANGE_DENOMINATOR: u64 = 8;

/// Elasticity multiplier as defined in [EIP-1559](https://eips.ethereum.org/EIPS/eip-1559)
pub const EIP1559_ELASTICITY_MULTIPLIER: u64 = 2;

pub fn default_elasticity() -> f64 {
    1f64 / BASE_FEE_CHANGE_DENOMINATOR as f64
}

/// Stores the fee related information
#[derive(Debug, Clone)]
pub struct FeeManager {
    /// Hardfork identifier
    spec_id: SpecId,
    /// Tracks the base fee for the next block post London
    ///
    /// This value will be updated after a new block was mined
    base_fee: Arc<RwLock<U256>>,
    /// The base price to use Pre London
    ///
    /// This will be constant value unless changed manually
    gas_price: Arc<RwLock<U256>>,
    elasticity: Arc<RwLock<f64>>,
}

// === impl FeeManager ===

impl FeeManager {
    pub fn new(spec_id: SpecId, base_fee: U256, gas_price: U256) -> Self {
        Self {
            spec_id,
            base_fee: Arc::new(RwLock::new(base_fee)),
            gas_price: Arc::new(RwLock::new(gas_price)),
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

    /// Calculates the current gas price
    pub fn gas_price(&self) -> U256 {
        if self.is_eip1559() {
            self.base_fee().saturating_add(self.suggested_priority_fee())
        } else {
            *self.gas_price.read()
        }
    }

    /// Suggested priority fee to add to the base fee
    pub fn suggested_priority_fee(&self) -> U256 {
        U256::from(1e9 as u64)
    }

    pub fn base_fee(&self) -> U256 {
        if self.is_eip1559() {
            *self.base_fee.read()
        } else {
            U256::zero()
        }
    }

    /// Returns the suggested fee cap
    ///
    /// This mirrors geth's auto values for `SuggestGasTipCap` which is: `priority fee + 2x current
    /// basefee`.
    pub fn max_priority_fee_per_gas(&self) -> U256 {
        self.suggested_priority_fee() + *self.base_fee.read() * 2
    }

    /// Returns the current gas price
    pub fn set_gas_price(&self, price: U256) {
        let mut gas = self.gas_price.write();
        *gas = price;
    }

    /// Returns the current base fee
    pub fn set_base_fee(&self, fee: U256) {
        trace!(target: "backend::fees", "updated base fee {:?}", fee);
        let mut base = self.base_fee.write();
        *base = fee;
    }

    /// Calculates the base fee for the next block
    pub fn get_next_block_base_fee_per_gas(
        &self,
        gas_used: U256,
        gas_limit: U256,
        last_fee_per_gas: U256,
    ) -> u64 {
        // It's naturally impossible for base fee to be 0;
        // It means it was set by the user deliberately and therefore we treat it as a constant.
        // Therefore, we skip the base fee calculation altogether and we return 0.
        if self.base_fee() == U256::zero() {
            return 0
        }
        calculate_next_block_base_fee(
            gas_used.as_u64(),
            gas_limit.as_u64(),
            last_fee_per_gas.as_u64(),
        )
    }
}

/// Calculate base fee for next block. [EIP-1559](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1559.md) spec
pub fn calculate_next_block_base_fee(gas_used: u64, gas_limit: u64, base_fee: u64) -> u64 {
    let gas_target = gas_limit / EIP1559_ELASTICITY_MULTIPLIER;

    if gas_used == gas_target {
        return base_fee
    }
    if gas_used > gas_target {
        let gas_used_delta = gas_used - gas_target;
        let base_fee_delta = std::cmp::max(
            1,
            base_fee as u128 * gas_used_delta as u128 /
                gas_target as u128 /
                BASE_FEE_CHANGE_DENOMINATOR as u128,
        );
        base_fee + (base_fee_delta as u64)
    } else {
        let gas_used_delta = gas_target - gas_used;
        let base_fee_per_gas_delta = base_fee as u128 * gas_used_delta as u128 /
            gas_target as u128 /
            BASE_FEE_CHANGE_DENOMINATOR as u128;

        base_fee.saturating_sub(base_fee_per_gas_delta as u64)
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

    /// Create a new history entry for the block
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
                    let effective_reward = match block.transactions.get(i).map(|tx| &tx.transaction)
                    {
                        Some(TypedTransaction::Legacy(t)) => {
                            t.gas_price.saturating_sub(base_fee).as_u64()
                        }
                        Some(TypedTransaction::EIP2930(t)) => {
                            t.gas_price.saturating_sub(base_fee).as_u64()
                        }
                        Some(TypedTransaction::EIP1559(t)) => t
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

pub type FeeHistoryCache = Arc<Mutex<BTreeMap<u64, FeeHistoryCacheItem>>>;

/// A single item in the whole fee history cache
#[derive(Debug, Clone)]
pub struct FeeHistoryCacheItem {
    pub base_fee: u64,
    pub gas_used_ratio: f64,
    pub rewards: Vec<u64>,
}

#[derive(Default, Clone)]
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

impl fmt::Debug for FeeDetails {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "Fees {{ ")?;
        write!(fmt, "gaPrice: {:?}, ", self.gas_price)?;
        write!(fmt, "max_fee_per_gas: {:?}, ", self.max_fee_per_gas)?;
        write!(fmt, "max_priority_fee_per_gas: {:?}, ", self.max_priority_fee_per_gas)?;
        write!(fmt, "}}")?;
        Ok(())
    }
}
