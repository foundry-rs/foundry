//! Optimism-specific extensions for [`NetworkConfigs`] and related helpers.

use crate::{NetworkConfigs, NetworkVariant};
use alloy_eips::eip1559::BaseFeeParams;
use alloy_op_hardforks::{OpChainHardforks, OpHardforks};

impl NetworkConfigs {
    pub fn with_optimism() -> Self {
        Self { network: Some(NetworkVariant::Optimism), optimism: true, ..Default::default() }
    }

    pub const fn is_optimism(&self) -> bool {
        matches!(self.resolved_network(), Some(NetworkVariant::Optimism))
    }

    /// Optimism-specific base fee parameters, picking Canyon vs pre-Canyon based on `timestamp`.
    pub(crate) fn op_base_fee_params(&self, timestamp: u64) -> BaseFeeParams {
        let op_hardforks = OpChainHardforks::op_mainnet();
        if op_hardforks.is_canyon_active_at_timestamp(timestamp) {
            BaseFeeParams::optimism_canyon()
        } else {
            BaseFeeParams::optimism()
        }
    }
}
