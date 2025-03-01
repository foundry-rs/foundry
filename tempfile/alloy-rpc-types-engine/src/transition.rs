use alloy_primitives::{B256, U256};

/// This structure contains configurable settings of the transition process.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[doc(alias = "TxConfiguration")]
pub struct TransitionConfiguration {
    /// Maps on the TERMINAL_TOTAL_DIFFICULTY parameter of EIP-3675
    pub terminal_total_difficulty: U256,
    /// Maps on TERMINAL_BLOCK_HASH parameter of EIP-3675
    pub terminal_block_hash: B256,
    /// Maps on TERMINAL_BLOCK_NUMBER parameter of EIP-3675
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity"))]
    pub terminal_block_number: u64,
}
