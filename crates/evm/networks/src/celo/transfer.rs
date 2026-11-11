//! Celo transfer precompile metadata.
//!
//! The Celo transfer precompile enables native token transfers from an EVM contract. It is part of
//! Celo's token duality system, allowing transfer of native tokens via ERC20.
//!
//! For more details, see: <https://specs.celo.org/token_duality.html#the-transfer-precompile>

use alloy_primitives::Address;

/// Label of the Celo transfer precompile to display in traces.
pub const CELO_TRANSFER_LABEL: &str = "CELO_TRANSFER_PRECOMPILE";

/// Address of the Celo transfer precompile.
pub const CELO_TRANSFER_ADDRESS: Address = Address::with_last_byte(0xfd);

/// Name of the [Celo transfer precompile](CELO_TRANSFER_ADDRESS).
pub const CELO_TRANSFER_PRECOMPILE_NAME: &str = "celo transfer";
