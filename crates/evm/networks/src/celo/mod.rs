//! Celo-specific execution support.

/// Celo dynamic fee transaction type introduced by CIP-64.
pub const CELO_DYNAMIC_FEE_TX_TYPE: u8 = 0x7b;

pub mod transfer;
