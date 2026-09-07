use std::{
    collections::BTreeMap,
    fmt,
    pin::Pin,
    sync::{Arc, LazyLock},
    task::{Context, Poll},
};

use alloy_consensus::{BlockHeader, Transaction, TxReceipt};
use alloy_eips::{calc_next_block_base_fee, eip1559::BaseFeeParams, eip7840::BlobParams};
use alloy_network::Network;
use alloy_primitives::{B256, Bytes};
#[cfg(feature = "optimism")]
use foundry_evm::hardfork::FoundryHardfork;
use futures::StreamExt;
use parking_lot::{Mutex, RwLock};
use revm::{context_interface::block::BlobExcessGasAndPrice, primitives::hardfork::SpecId};
use tempo_hardfork::{TempoHardfork, constants::gas::tempo_t7_next_block_base_fee};

use crate::eth::{
    backend::{info::StorageInfo, notifications::ChainNotifications},
    error::BlockchainError,
};

#[cfg(feature = "optimism")]
mod optimism;

/// Maximum number of entries in the fee history cache
pub const MAX_FEE_HISTORY_CACHE_SIZE: u64 = 2048u64;

/// Number of cached reward samples per percentile.
pub(crate) const REWARD_PERCENTILE_RESOLUTION: f64 = 2.0;

/// Percentile list from 0.0 to 100.0 with a 0.5 resolution (201 points).
///
/// Constant across blocks, so it is computed once instead of being rebuilt on every
/// `create_fee_history_cache_item` call.
static REWARD_PERCENTILES: LazyLock<Vec<f64>> =
    LazyLock::new(|| (0..=200).map(|index| index as f64 / REWARD_PERCENTILE_RESOLUTION).collect());

/// Initial base fee for EIP-1559 blocks.
pub const INITIAL_BASE_FEE: u64 = 1_000_000_000;

/// Initial default gas price for the first block
pub const INITIAL_GAS_PRICE: u128 = 1_875_000_000;

/// Bounds the amount the base fee can change between blocks.
pub const BASE_FEE_CHANGE_DENOMINATOR: u128 = 8;

/// Minimum suggested priority fee
pub const MIN_SUGGESTED_PRIORITY_FEE: u128 = 1e9 as u128;

/// Stores the fee related information
#[derive(Clone, Debug)]
pub struct FeeManager {
    /// Fee state published as one coherent execution context.
    state: Arc<RwLock<FeeState>>,
    /// Whether the minimum suggested priority fee is enforced
    is_min_priority_fee_enforced: bool,
}

#[derive(Clone, Copy, Debug)]
struct FeeRules {
    spec_id: SpecId,
    base_fee: BaseFeeRules,
    /// The active Tempo hardfork, set only when running a Tempo chain.
    tempo_hardfork: Option<TempoHardfork>,
}

#[derive(Clone, Copy, Debug)]
enum BaseFeeRules {
    Standard(BaseFeeParams),
    #[cfg(feature = "optimism")]
    Optimism {
        inherited: Option<optimism::OptimismBaseFeeRules>,
        fallback: BaseFeeParams,
    },
}

impl BaseFeeRules {
    const fn params(self) -> BaseFeeParams {
        match self {
            Self::Standard(params) => params,
            #[cfg(feature = "optimism")]
            Self::Optimism { inherited, fallback } => {
                if let Some(rules) = inherited {
                    rules.params()
                } else {
                    fallback
                }
            }
        }
    }

    fn extra_data(self) -> Bytes {
        match self {
            Self::Standard(_) => Bytes::new(),
            #[cfg(feature = "optimism")]
            Self::Optimism { inherited, .. } => {
                inherited.map_or_else(Bytes::new, optimism::OptimismBaseFeeRules::extra_data)
            }
        }
    }

    fn parent_header_fees<H: BlockHeader>(self, header: &H) -> ParentHeaderFees {
        match self {
            Self::Standard(params) => ParentHeaderFees {
                base_fee: calc_next_block_base_fee(
                    header.gas_used(),
                    header.gas_limit(),
                    header.base_fee_per_gas().unwrap_or_default(),
                    params,
                ),
                ..Default::default()
            },
            #[cfg(feature = "optimism")]
            Self::Optimism { fallback, .. } => {
                let inherited = optimism::OptimismBaseFeeRules::decode(header.extra_data());
                ParentHeaderFees {
                    base_fee: inherited.map_or_else(
                        || {
                            calc_next_block_base_fee(
                                header.gas_used(),
                                header.gas_limit(),
                                header.base_fee_per_gas().unwrap_or_default(),
                                fallback,
                            )
                        },
                        |rules| rules.next_block_base_fee(header),
                    ),
                    extra_data: inherited
                        .map_or_else(Bytes::new, optimism::OptimismBaseFeeRules::extra_data),
                    optimism_jovian: inherited.map(optimism::OptimismBaseFeeRules::is_jovian),
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ParentHeaderFees {
    /// Base fee inherited by the child block.
    pub(crate) base_fee: u64,
    /// Dynamic fee parameters inherited by the child block.
    pub(crate) extra_data: Bytes,
    /// Whether the decoded Optimism fee parameters activate Jovian.
    pub(crate) optimism_jovian: Option<bool>,
}

#[derive(Clone, Copy, Debug)]
struct FeeState {
    rules: FeeRules,
    blob_params: BlobParams,
    /// Base fee for the next block.
    base_fee: u64,
    /// Excess blob gas and price for the next block.
    blob_excess_gas_and_price: BlobExcessGasAndPrice,
    /// Legacy gas price.
    gas_price: u128,
}

/// Chain-derived fee state for the next block.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FeeSnapshot {
    base_fee: u64,
    blob_excess_gas_and_price: BlobExcessGasAndPrice,
}

impl FeeManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        spec_id: SpecId,
        base_fee: u64,
        is_min_priority_fee_enforced: bool,
        gas_price: u128,
        blob_excess_gas_and_price: BlobExcessGasAndPrice,
        blob_params: BlobParams,
        base_fee_params: BaseFeeParams,
        tempo_hardfork: Option<TempoHardfork>,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(FeeState {
                rules: FeeRules {
                    spec_id,
                    base_fee: BaseFeeRules::Standard(base_fee_params),
                    tempo_hardfork,
                },
                blob_params,
                base_fee,
                blob_excess_gas_and_price,
                gas_price,
            })),
            is_min_priority_fee_enforced,
        }
    }

    /// Creates an independent copy suitable for staging a fork reset.
    pub(crate) fn detached(&self) -> Self {
        Self {
            state: Arc::new(RwLock::new(*self.state.read())),
            is_min_priority_fee_enforced: self.is_min_priority_fee_enforced,
        }
    }

    /// Replaces all mutable fee state with a staged manager's values.
    pub(crate) fn replace_from(&self, other: &Self) {
        *self.state.write() = *other.state.read();
    }

    /// Captures the chain-derived fee state for the next block.
    pub(crate) fn snapshot(&self) -> FeeSnapshot {
        let state = self.state.read();
        FeeSnapshot {
            base_fee: state.base_fee,
            blob_excess_gas_and_price: state.blob_excess_gas_and_price,
        }
    }

    /// Restores the chain-derived fee state for the next block.
    pub(crate) fn restore(&self, snapshot: FeeSnapshot) {
        let mut state = self.state.write();
        state.base_fee = snapshot.base_fee;
        state.blob_excess_gas_and_price = snapshot.blob_excess_gas_and_price;
    }

    /// Returns the active Tempo hardfork, if running a Tempo chain.
    pub fn tempo_hardfork(&self) -> Option<TempoHardfork> {
        self.state.read().rules.tempo_hardfork
    }

    /// Atomically replaces all execution-dependent fee rules.
    pub fn set_execution_rules(
        &self,
        spec_id: SpecId,
        base_fee_params: BaseFeeParams,
        tempo_hardfork: Option<TempoHardfork>,
    ) {
        self.state.write().rules =
            FeeRules { spec_id, base_fee: BaseFeeRules::Standard(base_fee_params), tempo_hardfork };
    }

    /// Applies the dynamic EIP-1559 parameters encoded in an Optimism-family parent header.
    #[cfg(feature = "optimism")]
    pub(crate) fn set_optimism_base_fee_rules(&self, extra_data: &[u8]) {
        let mut state = self.state.write();
        let fallback = match state.rules.base_fee {
            BaseFeeRules::Standard(params) | BaseFeeRules::Optimism { fallback: params, .. } => {
                params
            }
        };
        state.rules.base_fee = BaseFeeRules::Optimism {
            inherited: optimism::OptimismBaseFeeRules::decode(extra_data),
            fallback,
        };
    }

    /// Initializes Optimism-family fee rules for a node that is not inheriting a fork header.
    #[cfg(feature = "optimism")]
    pub(crate) fn set_optimism_hardfork(&self, hardfork: FoundryHardfork) {
        let mut state = self.state.write();
        let fallback = state.rules.base_fee.params();
        state.rules.base_fee = BaseFeeRules::Optimism {
            inherited: optimism::OptimismBaseFeeRules::for_hardfork(hardfork, fallback),
            fallback,
        };
    }

    /// Returns the Optimism-family EIP-1559 parameters inherited by locally built blocks.
    pub(crate) fn base_fee_extra_data(&self) -> Bytes {
        self.state.read().rules.base_fee.extra_data()
    }

    pub fn elasticity(&self) -> f64 {
        1f64 / self.state.read().rules.base_fee.params().elasticity_multiplier as f64
    }

    /// Returns true for post London
    pub fn is_eip1559(&self) -> bool {
        (self.state.read().rules.spec_id as u8) >= (SpecId::LONDON as u8)
    }

    pub fn is_eip4844(&self) -> bool {
        (self.state.read().rules.spec_id as u8) >= (SpecId::CANCUN as u8)
    }

    /// Calculates the current blob gas price
    pub fn blob_gas_price(&self) -> u128 {
        let state = self.state.read();
        if (state.rules.spec_id as u8) >= (SpecId::CANCUN as u8) {
            state.blob_excess_gas_and_price.blob_gasprice
        } else {
            0
        }
    }

    pub fn base_fee(&self) -> u64 {
        let state = self.state.read();
        if (state.rules.spec_id as u8) >= (SpecId::LONDON as u8) { state.base_fee } else { 0 }
    }

    pub const fn is_min_priority_fee_enforced(&self) -> bool {
        self.is_min_priority_fee_enforced
    }

    /// Raw base gas price
    pub fn raw_gas_price(&self) -> u128 {
        self.state.read().gas_price
    }

    pub fn excess_blob_gas_and_price(&self) -> Option<BlobExcessGasAndPrice> {
        let state = self.state.read();
        ((state.rules.spec_id as u8) >= (SpecId::CANCUN as u8))
            .then_some(state.blob_excess_gas_and_price)
    }

    pub fn base_fee_per_blob_gas(&self) -> u128 {
        let state = self.state.read();
        if (state.rules.spec_id as u8) >= (SpecId::CANCUN as u8) {
            state.blob_excess_gas_and_price.blob_gasprice
        } else {
            0
        }
    }

    /// Returns the current gas price
    pub fn set_gas_price(&self, price: u128) {
        self.state.write().gas_price = price;
    }

    /// Returns the current base fee
    pub fn set_base_fee(&self, fee: u64) {
        trace!(target: "backend::fees", "updated base fee {:?}", fee);
        self.state.write().base_fee = fee;
    }

    /// Sets the current blob excess gas and price
    pub fn set_blob_excess_gas_and_price(&self, blob_excess_gas_and_price: BlobExcessGasAndPrice) {
        trace!(target: "backend::fees", "updated blob base fee {:?}", blob_excess_gas_and_price);
        self.state.write().blob_excess_gas_and_price = blob_excess_gas_and_price;
    }

    /// Calculates the base fee for the next block
    pub fn get_next_block_base_fee_per_gas(
        &self,
        gas_used: u64,
        gas_limit: u64,
        last_fee_per_gas: u64,
    ) -> u64 {
        let state = self.state.read();
        // It's naturally impossible for base fee to be 0;
        // It means it was set by the user deliberately and therefore we treat it as a constant.
        // Therefore, we skip the base fee calculation altogether and we return 0.
        if (state.rules.spec_id as u8) < (SpecId::LONDON as u8) || state.base_fee == 0 {
            return 0;
        }
        calculate_next_block_base_fee_per_gas(state.rules, gas_used, gas_limit, last_fee_per_gas)
    }

    /// Calculates the next block base fee from the parent block without applying the configured
    /// zero-fee sentinel.
    #[cfg(test)]
    pub(crate) fn calculate_next_block_base_fee_per_gas(
        &self,
        gas_used: u64,
        gas_limit: u64,
        last_fee_per_gas: u64,
    ) -> u64 {
        let rules = self.state.read().rules;
        if (rules.spec_id as u8) < (SpecId::LONDON as u8) {
            return 0;
        }
        calculate_next_block_base_fee_per_gas(rules, gas_used, gas_limit, last_fee_per_gas)
    }

    /// Calculates the next block base fee from a complete parent header.
    pub(crate) fn get_next_block_base_fee_from_header<H: BlockHeader>(&self, header: &H) -> u64 {
        let state = self.state.read();
        if (state.rules.spec_id as u8) < (SpecId::LONDON as u8) || state.base_fee == 0 {
            return 0;
        }
        calculate_parent_header_fees(state.rules, header).base_fee
    }

    /// Returns all fee metadata inherited from a parent header, honoring the configured zero-fee
    /// sentinel.
    pub(crate) fn get_parent_header_fees<H: BlockHeader>(&self, header: &H) -> ParentHeaderFees {
        let state = self.state.read();
        let mut fees = calculate_parent_header_fees(state.rules, header);
        if (state.rules.spec_id as u8) < (SpecId::LONDON as u8) || state.base_fee == 0 {
            fees.base_fee = 0;
        }
        fees
    }

    /// Calculates the next block base fee from a complete parent header without applying the
    /// configured zero-fee sentinel.
    pub(crate) fn calculate_next_block_base_fee_from_header<H: BlockHeader>(
        &self,
        header: &H,
    ) -> u64 {
        let rules = self.state.read().rules;
        if (rules.spec_id as u8) < (SpecId::LONDON as u8) {
            return 0;
        }
        calculate_parent_header_fees(rules, header).base_fee
    }

    /// Returns all fee metadata inherited from a parent header without applying the configured
    /// zero-fee sentinel.
    pub(crate) fn calculate_parent_header_fees<H: BlockHeader>(
        &self,
        header: &H,
    ) -> ParentHeaderFees {
        let rules = self.state.read().rules;
        let mut fees = calculate_parent_header_fees(rules, header);
        if (rules.spec_id as u8) < (SpecId::LONDON as u8) {
            fees.base_fee = 0;
        }
        fees
    }

    /// Calculates the next block blob base fee.
    pub fn get_next_block_blob_base_fee_per_gas(&self) -> u128 {
        let state = self.state.read();
        state.blob_params.calc_blob_fee(state.blob_excess_gas_and_price.excess_blob_gas)
    }

    /// Configures the blob params
    pub fn set_blob_params(&self, blob_params: BlobParams) {
        self.state.write().blob_params = blob_params;
    }

    /// Returns the active [`BlobParams`]
    pub fn blob_params(&self) -> BlobParams {
        self.state.read().blob_params
    }
}

fn calculate_next_block_base_fee_per_gas(
    rules: FeeRules,
    gas_used: u64,
    gas_limit: u64,
    last_fee_per_gas: u64,
) -> u64 {
    // Tempo replaces EIP-1559 with its own hardfork-specific base fee rules.
    if let Some(hardfork) = rules.tempo_hardfork {
        return tempo_next_block_base_fee(hardfork, gas_used, last_fee_per_gas);
    }
    calc_next_block_base_fee(gas_used, gas_limit, last_fee_per_gas, rules.base_fee.params())
}

fn calculate_parent_header_fees<H: BlockHeader>(rules: FeeRules, header: &H) -> ParentHeaderFees {
    if let Some(hardfork) = rules.tempo_hardfork {
        return ParentHeaderFees {
            base_fee: tempo_next_block_base_fee(
                hardfork,
                header.gas_used(),
                header.base_fee_per_gas().unwrap_or_default(),
            ),
            ..Default::default()
        };
    }
    rules.base_fee.parent_header_fees(header)
}

/// Computes the next block's base fee for a Tempo chain.
///
/// - T7+: the TIP-1067 dynamic controller, an EIP-1559 update against a fixed 10M gas target
///   clamped to `[floor, cap]`.
/// - Pre-T7: the fixed hardfork base fee (10 gwei pre-T1, 20 gwei T1+).
fn tempo_next_block_base_fee(hardfork: TempoHardfork, gas_used: u64, parent_base_fee: u64) -> u64 {
    if hardfork.is_t7() {
        return tempo_t7_next_block_base_fee(parent_base_fee, gas_used);
    }
    crate::config::tempo_default_base_fee(hardfork)
}

/// An async service that takes care of the `FeeHistory` cache
pub struct FeeHistoryService<N: Network>
where
    N::ReceiptEnvelope: TxReceipt<Log = alloy_primitives::Log>,
{
    /// Live fee rules, including blob parameters replaced by fork resets.
    fees: FeeManager,
    /// incoming notifications about new blocks
    new_blocks: ChainNotifications,
    /// contains all fee history related entries
    cache: FeeHistoryCache,
    /// number of items to consider
    fee_history_limit: u64,
    /// a type that can fetch ethereum-storage data
    storage_info: StorageInfo<N>,
}

impl<N: Network> FeeHistoryService<N>
where
    N::ReceiptEnvelope: TxReceipt<Log = alloy_primitives::Log>,
{
    pub const fn new(
        fees: FeeManager,
        new_blocks: ChainNotifications,
        cache: FeeHistoryCache,
        storage_info: StorageInfo<N>,
    ) -> Self {
        Self {
            fees,
            new_blocks,
            cache,
            fee_history_limit: MAX_FEE_HISTORY_CACHE_SIZE,
            storage_info,
        }
    }

    /// Returns the configured history limit
    pub const fn fee_history_limit(&self) -> u64 {
        self.fee_history_limit
    }

    /// Inserts a new cache entry for the given block
    pub(crate) fn insert_cache_entry_for_block(&self, hash: B256, header: &impl BlockHeader) {
        let (result, block_number) = self.create_cache_entry(hash, header);
        self.insert_cache_entry(result, block_number);
    }

    /// Create a new history entry for the block
    fn create_cache_entry(
        &self,
        hash: B256,
        header: &impl BlockHeader,
    ) -> (FeeHistoryCacheItem, Option<u64>) {
        create_fee_history_cache_item(hash, header, &self.storage_info, self.fees.blob_params())
    }

    fn insert_cache_entry(&self, item: FeeHistoryCacheItem, block_number: Option<u64>) {
        insert_fee_history_cache_item(&self.cache, item, block_number, self.fee_history_limit);
    }
}

/// Inserts an entry into the fee history cache and trims it back to `fee_history_limit`.
///
/// Used by the async [`FeeHistoryService`]. The `eth_feeHistory` fallback applies the same bounded
/// insertion policy to a batch under one lock.
pub(crate) fn insert_fee_history_cache_item(
    cache: &FeeHistoryCache,
    item: FeeHistoryCacheItem,
    block_number: Option<u64>,
    fee_history_limit: u64,
) {
    if let Some(block_number) = block_number {
        trace!(target: "fees", "insert new history item={:?} for {}", item, block_number);
        let mut cache = cache.lock();
        cache.insert(block_number, item);

        // Trim to the cache limit by dropping the oldest entries (smallest block numbers).
        // `pop_first` is saturating and correct regardless of insertion order, unlike the
        // previous index math which could underflow when the `eth_feeHistory` fallback inserts
        // entries out of order.
        while cache.len() as u64 > fee_history_limit {
            cache.pop_first();
        }
    }
}

/// Calculates percentile rewards from transactions sorted by effective reward.
///
/// [`REWARD_PERCENTILES`] must remain ascending because the transaction cursor never rewinds.
fn reward_percentiles(transactions: &[(u64, u128)], block_gas_used: f64) -> Vec<u128> {
    let mut rewards = Vec::with_capacity(REWARD_PERCENTILES.len());
    let mut transactions = transactions.iter().copied();
    let Some((mut cumulative_gas, mut current_reward)) = transactions.next() else {
        return rewards;
    };

    for &percentile in REWARD_PERCENTILES.iter() {
        let target_gas = (percentile * block_gas_used / 100f64) as u64;
        while target_gas > cumulative_gas {
            let Some((tx_gas_used, effective_reward)) = transactions.next() else { return rewards };
            cumulative_gas += tx_gas_used;
            current_reward = effective_reward;
        }
        rewards.push(current_reward);
    }

    rewards
}

/// Builds the [`FeeHistoryCacheItem`] for a single block.
///
/// Shared by the async [`FeeHistoryService`] and by `eth_feeHistory` itself: the service can lag
/// the chain head (it only runs when the node task is polled), so the RPC handler computes any
/// missing entry on demand with the same logic instead of returning a short response.
pub(crate) fn create_fee_history_cache_item<N: Network>(
    hash: B256,
    header: &impl BlockHeader,
    storage_info: &StorageInfo<N>,
    blob_params: BlobParams,
) -> (FeeHistoryCacheItem, Option<u64>)
where
    N::ReceiptEnvelope: TxReceipt<Log = alloy_primitives::Log>,
{
    let mut block_number: Option<u64> = None;
    let base_fee = header.base_fee_per_gas().unwrap_or_default();
    let excess_blob_gas = header.excess_blob_gas().map(|g| g as u128);
    let blob_gas_used = header.blob_gas_used().map(|g| g as u128);
    let base_fee_per_blob_gas = header.blob_fee(blob_params);

    let mut item = FeeHistoryCacheItem {
        block_hash: hash,
        base_fee: base_fee as u128,
        gas_used_ratio: 0f64,
        blob_gas_used_ratio: 0f64,
        rewards: Vec::new(),
        excess_blob_gas,
        base_fee_per_blob_gas,
        blob_gas_used,
    };

    let current_block = storage_info.block(hash);
    let current_receipts = storage_info.receipts(hash);

    if let (Some(block), Some(receipts)) = (current_block, current_receipts) {
        block_number = Some(block.header.number());

        let gas_used = block.header.gas_used() as f64;
        let blob_gas_used = block.header.blob_gas_used().map(|g| g as f64);
        item.gas_used_ratio = gas_used / block.header.gas_limit() as f64;
        item.blob_gas_used_ratio = blob_gas_used
            .map(|g| {
                let max = blob_params.max_blob_gas_per_block() as f64;
                if max == 0.0 { 0.0 } else { g / max }
            })
            .unwrap_or(0.0);

        // extract useful tx info (gas_used, effective_reward)
        let mut transactions: Vec<(_, _)> = receipts
            .iter()
            .enumerate()
            .map(|(i, receipt)| {
                let cumulative = receipt.cumulative_gas_used();
                let prev_cumulative = if i > 0 { receipts[i - 1].cumulative_gas_used() } else { 0 };
                let gas_used = cumulative - prev_cumulative;
                let effective_reward = block
                    .body
                    .transactions
                    .get(i)
                    .map(|tx| tx.as_ref().effective_tip_per_gas(base_fee).unwrap_or(0))
                    .unwrap_or(0);

                (gas_used, effective_reward)
            })
            .collect();

        // sort by effective reward asc
        transactions.sort_by_key(|(_, reward)| *reward);

        item.rewards = reward_percentiles(&transactions, gas_used);
    } else {
        item.rewards = vec![0; REWARD_PERCENTILES.len()];
    }
    (item, block_number)
}

// An endless future that listens for new blocks and updates the cache
impl<N: Network> Future for FeeHistoryService<N>
where
    N::ReceiptEnvelope: TxReceipt<Log = alloy_primitives::Log>,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();

        while let Poll::Ready(Some(notification)) = pin.new_blocks.poll_next_unpin(cx) {
            // add the imported block.
            if let Some(block) = notification.as_new_block() {
                pin.insert_cache_entry_for_block(block.hash, block.header.as_ref());
            }
        }

        Poll::Pending
    }
}

pub type FeeHistoryCache = Arc<Mutex<BTreeMap<u64, FeeHistoryCacheItem>>>;

/// A single item in the whole fee history cache
#[derive(Clone, Debug)]
pub struct FeeHistoryCacheItem {
    pub block_hash: B256,
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
    pub const fn zero() -> Self {
        Self {
            gas_price: Some(0),
            max_fee_per_gas: Some(0),
            max_priority_fee_per_gas: Some(0),
            max_fee_per_blob_gas: None,
        }
    }

    /// If neither `gas_price` nor `max_fee_per_gas` is `Some`, this will set both to `0`
    pub const fn or_zero_fees(self) -> Self {
        let Self { gas_price, max_fee_per_gas, max_priority_fee_per_gas, max_fee_per_blob_gas } =
            self;

        let no_fees = gas_price.is_none() && max_fee_per_gas.is_none();
        let gas_price = if no_fees { Some(0) } else { gas_price };
        let max_fee_per_gas = if no_fees { Some(0) } else { max_fee_per_gas };
        let max_fee_per_blob_gas = if no_fees { None } else { max_fee_per_blob_gas };

        Self { gas_price, max_fee_per_gas, max_priority_fee_per_gas, max_fee_per_blob_gas }
    }

    /// Turns this type into a tuple
    pub const fn split(self) -> (Option<u128>, Option<u128>, Option<u128>, Option<u128>) {
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
            (_, max_fee, max_priority, max_fee_per_blob_gas) => {
                // eip-1559
                // Ensure `max_priority_fee_per_gas` is less or equal to `max_fee_per_gas`.
                if let Some(max_priority) = max_priority {
                    let max_fee = max_fee.unwrap_or_default();
                    if max_priority > max_fee {
                        return Err(BlockchainError::InvalidFeeInput);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn reward_percentiles_reference(
        transactions: &[(u64, u128)],
        block_gas_used: f64,
    ) -> Vec<u128> {
        REWARD_PERCENTILES
            .iter()
            .filter_map(|&percentile| {
                let target_gas = (percentile * block_gas_used / 100f64) as u64;
                let mut cumulative_gas = 0;
                for (tx_gas_used, effective_reward) in transactions.iter().copied() {
                    cumulative_gas += tx_gas_used;
                    if target_gas <= cumulative_gas {
                        return Some(effective_reward);
                    }
                }
                None
            })
            .collect()
    }

    fn assert_reward_percentiles_match(transactions: &mut [(u64, u128)], gas_used: u64) {
        transactions.sort_by_key(|(_, reward)| *reward);
        assert_eq!(
            reward_percentiles(transactions, gas_used as f64),
            reward_percentiles_reference(transactions, gas_used as f64)
        );
    }

    fn fee_manager(spec_id: SpecId) -> FeeManager {
        FeeManager::new(
            spec_id,
            INITIAL_BASE_FEE,
            true,
            INITIAL_GAS_PRICE,
            BlobExcessGasAndPrice::new_with_spec(0, SpecId::CANCUN),
            BlobParams::cancun(),
            BaseFeeParams::ethereum(),
            None,
        )
    }

    #[test]
    fn raw_next_base_fee_respects_london_activation() {
        let berlin = fee_manager(SpecId::BERLIN);
        assert_eq!(
            berlin.calculate_next_block_base_fee_per_gas(30_000_000, 30_000_000, INITIAL_BASE_FEE),
            0
        );

        let london = fee_manager(SpecId::LONDON);
        assert_ne!(
            london.calculate_next_block_base_fee_per_gas(30_000_000, 30_000_000, INITIAL_BASE_FEE),
            0
        );
    }

    #[cfg(feature = "optimism")]
    #[test]
    fn pre_london_parent_fees_preserve_optimism_metadata() {
        let fees = fee_manager(SpecId::BERLIN);
        let jovian = [1, 0, 0, 0, 250, 0, 0, 0, 2, 0, 0, 0, 0, 0, 76, 75, 64];
        fees.set_optimism_base_fee_rules(&jovian);
        let header = alloy_consensus::Header {
            extra_data: Bytes::copy_from_slice(&jovian),
            ..Default::default()
        };

        let parent_fees = fees.get_parent_header_fees(&header);
        assert_eq!(parent_fees.base_fee, 0);
        assert_eq!(parent_fees.extra_data.as_ref(), jovian);
        assert_eq!(parent_fees.optimism_jovian, Some(true));
    }

    #[test]
    fn reward_percentile_sweep_preserves_boundaries_and_empty_results() {
        assert_reward_percentiles_match(&mut [], 0);

        let mut transactions = [(5, 1), (0, 2), (5, 3)];
        assert_reward_percentiles_match(&mut transactions, 1_000);
        assert_eq!(reward_percentiles(&transactions, 1_000f64), [1, 1, 3]);

        let mut transactions = [(0, 10), (1, 20)];
        assert_reward_percentiles_match(&mut transactions, 1);
        let rewards = reward_percentiles(&transactions, 1f64);
        assert_eq!(&rewards[..200], &[10; 200]);
        assert_eq!(rewards[200], 20);
    }

    #[test]
    fn reward_percentile_sweep_matches_reference_for_randomized_inputs() {
        let mut state = 0x4d59_5df4_d0f3_3173u64;
        for _ in 0..2_000 {
            let len = (next_random(&mut state) % 129) as usize;
            let mut transactions = (0..len)
                .map(|_| {
                    let gas_used = next_random(&mut state) % 100;
                    let effective_reward = (next_random(&mut state) % 16) as u128;
                    (gas_used, effective_reward)
                })
                .collect::<Vec<_>>();
            let total_gas = transactions.iter().map(|(gas_used, _)| gas_used).sum::<u64>();
            let header_gas_used = match next_random(&mut state) % 4 {
                0 => total_gas,
                1 => next_random(&mut state) % (total_gas.saturating_add(1)),
                2 => total_gas.saturating_add(next_random(&mut state) % 1_000),
                _ => 0,
            };

            assert_reward_percentiles_match(&mut transactions, header_gas_used);
        }
    }

    fn next_random(state: &mut u64) -> u64 {
        *state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        *state
    }
}
