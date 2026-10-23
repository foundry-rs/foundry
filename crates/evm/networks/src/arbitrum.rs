//! Minimal Arbitrum system contract compatibility helpers.

use alloy_chains::Chain;
use alloy_primitives::{Address, Bytes, U256, hex};

/// ArbSys system contract address.
pub const ARB_SYS_ADDRESS: Address = Address::with_last_byte(0x64);

/// `ArbSys.arbBlockNumber()` selector.
pub const ARB_BLOCK_NUMBER_SELECTOR: [u8; 4] = hex!("a3b1b31d");

/// Gas Nitro charges inside `ArbSys.arbBlockNumber()`: opening the ArbOS state reads its version
/// from storage (800, the EIP-2200 `SLOAD` cost), and copying the 32-byte result costs 3.
pub const ARB_BLOCK_NUMBER_GAS_COST: u64 = 803;

/// Returns whether `chain_id` is an Arbitrum chain.
pub fn is_arbitrum_chain(chain_id: u64) -> bool {
    Chain::from_id(chain_id).is_arbitrum()
}

/// Returns the ABI-encoded result for `ArbSys.arbBlockNumber()`.
pub fn arb_block_number_output(block_number: u64) -> Bytes {
    U256::from(block_number).to_be_bytes::<32>().into()
}

/// Returns the gas cost and ABI-encoded result for `ArbSys.arbBlockNumber()`.
pub fn arb_block_number_call(gas_limit: u64, block_number: u64) -> Option<(u64, Bytes)> {
    (gas_limit >= ARB_BLOCK_NUMBER_GAS_COST)
        .then(|| (ARB_BLOCK_NUMBER_GAS_COST, arb_block_number_output(block_number)))
}
