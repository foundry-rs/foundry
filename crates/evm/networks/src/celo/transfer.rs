//! Celo precompile implementation for token transfers.
//!
//! This module implements the Celo transfer precompile that enables native token transfers from an
//! EVM contract. The precompile is part of Celo's token duality system, allowing transfer of
//! native tokens via ERC20.
//!
//! For more details, see: <https://specs.celo.org/token_duality.html#the-transfer-precompile>
//!
//! The transfer precompile is deployed at address 0xfd and accepts 96 bytes of input:
//! - from address (32 bytes, left-padded)
//! - to address (32 bytes, left-padded)
//! - value (32 bytes, big-endian U256)

use std::borrow::Cow;

use alloy_evm::precompiles::{DynPrecompile, PrecompileInput};
use alloy_primitives::{Address, U256, address};
use revm::precompile::{PrecompileError, PrecompileId, PrecompileOutput, PrecompileResult};

/// Label of the Celo transfer precompile to display in traces.
pub const CELO_TRANSFER_LABEL: &str = "CELO_TRANSFER_PRECOMPILE";

/// Address of the Celo transfer precompile.
pub const CELO_TRANSFER_ADDRESS: Address = address!("0x00000000000000000000000000000000000000fd");

/// ID for the [Celo transfer precompile](CELO_TRANSFER_ADDRESS).
pub static PRECOMPILE_ID_CELO_TRANSFER: PrecompileId =
    PrecompileId::Custom(Cow::Borrowed("celo transfer"));

/// Gas cost for Celo transfer precompile.
const CELO_TRANSFER_GAS_COST: u64 = 9000;

/// Returns the Celo native transfer.
pub fn precompile() -> DynPrecompile {
    DynPrecompile::new_stateful(PRECOMPILE_ID_CELO_TRANSFER.clone(), celo_transfer_precompile)
}

/// Celo transfer precompile implementation.
///
/// Uses load_account to modify balances directly, making it compatible with PrecompilesMap.
pub fn celo_transfer_precompile(mut input: PrecompileInput<'_>) -> PrecompileResult {
    // Check minimum gas requirement
    if input.gas < CELO_TRANSFER_GAS_COST {
        return Err(PrecompileError::OutOfGas);
    }

    // Validate input length (must be exactly 96 bytes: 32 + 32 + 32)
    if input.data.len() != 96 {
        return Err(PrecompileError::Other(
            format!(
                "Invalid input length for Celo transfer precompile: expected 96 bytes, got {}",
                input.data.len()
            )
            .into(),
        ));
    }

    // Parse input: from (bytes 12-32), to (bytes 44-64), value (bytes 64-96)
    let from_bytes = &input.data[12..32];
    let to_bytes = &input.data[44..64];
    let value_bytes = &input.data[64..96];

    let from_address = Address::from_slice(from_bytes);
    let to_address = Address::from_slice(to_bytes);
    let value = U256::from_be_slice(value_bytes);

    // Perform the transfer using load_account to modify balances directly
    let internals = input.internals_mut();

    // Load and check the from account balance first

    let from_account = match internals.load_account(from_address) {
        Ok(account) => account,
        Err(e) => {
            return Err(PrecompileError::Other(
                format!("Failed to load from account: {e:?}").into(),
            ));
        }
    };

    // Check if from account has sufficient balance
    if from_account.data.info.balance < value {
        return Err(PrecompileError::Other("Insufficient balance".into()));
    }

    let to_account = match internals.load_account(to_address) {
        Ok(account) => account,
        Err(e) => {
            return Err(PrecompileError::Other(format!("Failed to load to account: {e:?}").into()));
        }
    };

    // Check for overflow in to account
    if to_account.data.info.balance.checked_add(value).is_none() {
        return Err(PrecompileError::Other("Balance overflow in to account".into()));
    }

    // Transfer the value between accounts
    internals
        .transfer(from_address, to_address, value)
        .map_err(|e| PrecompileError::Other(format!("Failed to perform transfer: {e:?}").into()))?;

    // No output data for successful transfer
    Ok(PrecompileOutput::new(CELO_TRANSFER_GAS_COST, alloy_primitives::Bytes::new()))
}
