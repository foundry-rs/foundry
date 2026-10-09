use alloy_primitives::{Address, B256};
use parking_lot::RwLock;
use std::sync::Arc;

/// Shared block environment overrides for the block gas limit, the coinbase, the transaction
/// count limit, the gas price before London, and the next block base fee.
///
/// The gas limit, the coinbase, the transaction count limit, and the gas price persist and apply
/// to every following block until changed again. The next base fee is consumed once, when the
/// next block is built.
#[derive(Clone, Debug, Default)]
pub struct BlockEnvOverrides {
    gas_limit: Arc<RwLock<Option<u64>>>,
    coinbase: Arc<RwLock<Option<Address>>>,
    max_transactions: Arc<RwLock<Option<usize>>>,
    gas_price: Arc<RwLock<Option<u128>>>,
    next_base_fee: Arc<RwLock<Option<u64>>>,
    next_prev_randao: Arc<RwLock<Option<B256>>>,
    next_parent_beacon_block_root: Arc<RwLock<Option<B256>>>,
    /// The one-shot overrides of the block under construction.
    building: Arc<RwLock<Option<BuildingOverrides>>>,
}

impl BlockEnvOverrides {
    /// Sets a persistent gas limit override for all future blocks.
    pub fn set_gas_limit(&self, limit: u64) {
        *self.gas_limit.write() = Some(limit);
    }

    /// Returns the gas limit override, if set.
    pub fn gas_limit(&self) -> Option<u64> {
        *self.gas_limit.read()
    }

    /// Sets a persistent coinbase override for all future blocks.
    pub fn set_coinbase(&self, address: Address) {
        *self.coinbase.write() = Some(address);
    }

    /// Returns the coinbase override, if set.
    pub fn coinbase(&self) -> Option<Address> {
        *self.coinbase.read()
    }

    /// Sets the number of transactions a block may hold for all future blocks. `None` lifts the
    /// limit.
    pub fn set_max_transactions(&self, limit: Option<usize>) {
        *self.max_transactions.write() = limit;
    }

    /// Returns the number of transactions a block may hold, if limited.
    pub fn max_transactions(&self) -> Option<usize> {
        *self.max_transactions.read()
    }

    /// Sets the gas price the node suggests before London.
    pub fn set_gas_price(&self, gas_price: u128) {
        *self.gas_price.write() = Some(gas_price);
    }

    /// Returns the gas price the node suggests before London, if set.
    pub fn gas_price(&self) -> Option<u128> {
        *self.gas_price.read()
    }

    /// Sets the base fee for the next block only.
    pub fn set_next_base_fee(&self, fee: u64) {
        *self.next_base_fee.write() = Some(fee);
    }

    /// Returns the next block base fee override.
    pub fn next_base_fee(&self) -> Option<u64> {
        *self.next_base_fee.read()
    }

    /// Sets the prevrandao of the next block only.
    pub fn set_next_prev_randao(&self, prev_randao: B256) {
        *self.next_prev_randao.write() = Some(prev_randao);
    }

    /// Returns the next block prevrandao override.
    pub fn next_prev_randao(&self) -> Option<B256> {
        *self.next_prev_randao.read()
    }

    /// Sets the parent beacon block root of the next block only.
    pub fn set_next_parent_beacon_block_root(&self, root: B256) {
        *self.next_parent_beacon_block_root.write() = Some(root);
    }

    /// Returns the next block parent beacon block root override.
    pub fn next_parent_beacon_block_root(&self) -> Option<B256> {
        *self.next_parent_beacon_block_root.read()
    }

    /// Moves the one-shot overrides into the block under construction. [`Self::end_block`]
    /// drops them when the block is mined, and puts them back when it is not, as anvil keeps
    /// them for the next attempt.
    pub fn begin_block(&self) {
        let building = BuildingOverrides {
            base_fee: self.next_base_fee.write().take(),
            prev_randao: self.next_prev_randao.write().take(),
            parent_beacon_block_root: self.next_parent_beacon_block_root.write().take(),
        };
        *self.building.write() = Some(building);
    }

    /// Finishes the block under construction.
    pub fn end_block(&self, mined: bool) {
        let Some(building) = self.building.write().take() else { return };
        if mined {
            return;
        }
        // An override set during the attempt wins over the one put back.
        let mut base_fee = self.next_base_fee.write();
        *base_fee = base_fee.or(building.base_fee);
        let mut prev_randao = self.next_prev_randao.write();
        *prev_randao = prev_randao.or(building.prev_randao);
        let mut root = self.next_parent_beacon_block_root.write();
        *root = root.or(building.parent_beacon_block_root);
    }

    /// Returns the base fee of the block under construction, if overridden.
    pub fn building_base_fee(&self) -> Option<u64> {
        self.building.read().and_then(|building| building.base_fee)
    }

    /// Returns the prevrandao of the block under construction, if overridden.
    pub fn building_prev_randao(&self) -> Option<B256> {
        self.building.read().and_then(|building| building.prev_randao)
    }

    /// Returns the parent beacon block root of the block under construction, if overridden.
    pub fn building_parent_beacon_block_root(&self) -> Option<B256> {
        self.building.read().and_then(|building| building.parent_beacon_block_root)
    }

    /// Captures the current overrides.
    pub fn snapshot(&self) -> BlockEnvSnapshot {
        BlockEnvSnapshot {
            gas_limit: *self.gas_limit.read(),
            coinbase: *self.coinbase.read(),
            max_transactions: *self.max_transactions.read(),
            gas_price: *self.gas_price.read(),
            next_base_fee: *self.next_base_fee.read(),
            next_prev_randao: *self.next_prev_randao.read(),
            next_parent_beacon_block_root: *self.next_parent_beacon_block_root.read(),
        }
    }

    /// Restores the given overrides.
    pub fn restore(&self, snapshot: BlockEnvSnapshot) {
        *self.gas_limit.write() = snapshot.gas_limit;
        *self.coinbase.write() = snapshot.coinbase;
        *self.max_transactions.write() = snapshot.max_transactions;
        *self.gas_price.write() = snapshot.gas_price;
        *self.next_base_fee.write() = snapshot.next_base_fee;
        *self.next_prev_randao.write() = snapshot.next_prev_randao;
        *self.next_parent_beacon_block_root.write() = snapshot.next_parent_beacon_block_root;
    }
}

/// The one-shot overrides of the block under construction.
#[derive(Clone, Copy, Debug, Default)]
struct BuildingOverrides {
    base_fee: Option<u64>,
    prev_randao: Option<B256>,
    parent_beacon_block_root: Option<B256>,
}

/// A copy of the block environment overrides.
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockEnvSnapshot {
    gas_limit: Option<u64>,
    coinbase: Option<Address>,
    max_transactions: Option<usize>,
    gas_price: Option<u128>,
    next_base_fee: Option<u64>,
    next_prev_randao: Option<B256>,
    next_parent_beacon_block_root: Option<B256>,
}
