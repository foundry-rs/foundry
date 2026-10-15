//! Execution environments and network extension state.

use super::FoundryTxEnv;
use alloy_eips::eip4844::{BLOB_TX_MIN_BLOB_GASPRICE, fake_exponential};
use alloy_primitives::U256;
use evm2::{EvmFeatures, EvmTypesHost, Version, env::BlockEnv};
use std::fmt::Debug;

/// Successor of [`crate::EvmEnv`]: the spec, runtime version and block an EVM is built with.
///
/// [`Version`] replaces REVM's `CfgEnv`. Foundry's execution toggles map onto [`EvmFeatures`]:
/// `disable_nonce_check` is `NONCE_CHECK`, `disable_balance_check` is `BALANCE_CHECK`,
/// `disable_base_fee` is `BASE_FEE_CHECK`, `disable_block_gas_limit` is `BLOCK_GAS_LIMIT_CHECK`,
/// `disable_eip3607` is `EIP3607`, `disable_fee_charge` is `FEE_CHARGE`,
/// `disable_priority_fee_check` is `PRIORITY_FEE_CHECK` and `tx_chain_id_check` is
/// `TX_CHAIN_ID_CHECK`. Size and memory limits are plain [`Version`] fields.
#[derive(Debug)]
pub struct EvmEnv<T: EvmTypesHost> {
    /// Active network spec.
    pub spec: T::SpecId,
    /// Runtime version: chain ID, features, gas parameters and limits.
    pub version: Version,
    /// Block environment.
    pub block: BlockEnv<T>,
}

impl<T: EvmTypesHost> Clone for EvmEnv<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: EvmTypesHost> Copy for EvmEnv<T> {}

impl<T: EvmTypesHost> EvmEnv<T> {
    /// Creates an environment.
    pub const fn new(spec: T::SpecId, version: Version, block: BlockEnv<T>) -> Self {
        Self { spec, version, block }
    }

    /// Returns the chain ID.
    pub const fn chain_id(&self) -> u64 {
        self.version.chain_id
    }

    /// Sets the chain ID used by `CHAINID` and transaction validation.
    pub const fn set_chain_id(&mut self, chain_id: u64) {
        self.version.chain_id = chain_id;
    }

    /// Enables or disables `feature`.
    pub const fn set_feature(&mut self, feature: EvmFeatures, enabled: bool) {
        self.version.features.set(feature, enabled);
    }

    /// Sets the blob base fee from the excess blob gas, replacing REVM's
    /// `BlockEnv::set_blob_excess_gas_and_price`.
    pub fn set_blob_excess_gas(&mut self, excess_blob_gas: u64, base_fee_update_fraction: u64) {
        self.block.blob_basefee = U256::from(fake_exponential(
            BLOB_TX_MIN_BLOB_GASPRICE,
            excess_blob_gas as u128,
            base_fee_update_fraction as u128,
        ));
    }
}

/// Features Foundry toggles independently of the fork.
pub const FOUNDRY_POLICY_FEATURES: EvmFeatures = {
    let mut features = EvmFeatures::TX_CHAIN_ID_CHECK;
    features.insert(EvmFeatures::NONCE_CHECK);
    features.insert(EvmFeatures::BALANCE_CHECK);
    features.insert(EvmFeatures::BALANCE_TOP_UP);
    features.insert(EvmFeatures::BLOCK_GAS_LIMIT_CHECK);
    features.insert(EvmFeatures::EIP3607);
    features.insert(EvmFeatures::PRIORITY_FEE_CHECK);
    features.insert(EvmFeatures::FEE_CHARGE);
    features.insert(EvmFeatures::BASE_FEE_CHECK);
    features.insert(EvmFeatures::CODE_SIZE_CHECK);
    features
};

/// Returns `next` with Foundry's execution policy carried over from `current`.
///
/// `current_default` is the network's default version for the current spec. A policy feature or
/// limit that differs from it was set by Foundry and survives the spec change; everything else,
/// including gas parameters and fork features, comes from `next`.
pub fn carry_execution_policy(
    current_default: &Version,
    current: &Version,
    mut next: Version,
) -> Version {
    let disabled = current_default.features & FOUNDRY_POLICY_FEATURES & !current.features;
    let enabled = current.features & FOUNDRY_POLICY_FEATURES & !current_default.features;
    next.features = (next.features & !disabled) | enabled;
    next.chain_id = current.chain_id;
    macro_rules! carry_overridden {
        ($($field:ident),*) => {$(
            if current.$field != current_default.$field {
                next.$field = current.$field;
            }
        )*};
    }
    carry_overridden!(tx_gas_limit_cap, memory_limit, max_code_size, max_initcode_size);
    next
}

/// Successor of the network-specific half of [`crate::FoundryBlock`].
///
/// Common block fields are public on [`BlockEnv`]; only extension fields need accessors.
pub trait FoundryBlockExt: Copy + Debug + Default + Send + Sync + 'static {
    /// Returns the milliseconds portion of the block timestamp.
    fn timestamp_millis_part(&self) -> u64 {
        0
    }

    /// Sets the milliseconds portion of the block timestamp.
    fn set_timestamp_millis_part(&mut self, _millis: u64) {}
}

impl FoundryBlockExt for () {}

/// Successor of [`crate::FoundryChain`]: per-EVM network state such as OP L1 block info or Monad's
/// reserve-balance tracker.
pub trait FoundryEvmExt<TxExt>: Clone + Debug + Default + 'static {
    /// Prepares the state for a standalone synthetic transaction.
    fn for_transaction(&mut self, _tx: &FoundryTxEnv<TxExt>) {}

    /// Clears cached per-transaction protocol fees after a synthetic transaction restores state.
    fn clear_transaction_fee_cache(&mut self) {}
}

impl<TxExt> FoundryEvmExt<TxExt> for () {}

/// Successor of `IntoInstructionResult` for halt reasons.
///
/// The shared `TxResult::stop` carries the outcome; this exposes network-only outcome flags.
pub trait FoundryTxResultExt: Clone + Debug + Default + 'static {
    /// Returns whether an OP-stack deposit failed and kept only its mint and nonce bump.
    fn is_failed_deposit(&self) -> bool {
        false
    }
}

impl FoundryTxResultExt for () {}
