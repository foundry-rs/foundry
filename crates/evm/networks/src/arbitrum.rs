//! Minimal Arbitrum system contract compatibility helpers.

use std::borrow::Cow;

use alloy_chains::Chain;
use alloy_evm::precompiles::{DynPrecompile, PrecompileInput};
use alloy_primitives::{Address, Bytes, U256, address, hex};
use revm::precompile::{PrecompileHalt, PrecompileId, PrecompileOutput, PrecompileResult};

/// ArbSys system contract address.
pub const ARB_SYS_ADDRESS: Address = address!("0000000000000000000000000000000000000064");

/// Robinhood Chain mainnet chain ID.
pub const ROBINHOOD_MAINNET_CHAIN_ID: u64 = 4663;

/// Robinhood Chain testnet chain ID.
pub const ROBINHOOD_TESTNET_CHAIN_ID: u64 = 46630;

/// `ArbSys.arbBlockNumber()` selector.
pub const ARB_BLOCK_NUMBER_SELECTOR: [u8; 4] = hex!("a3b1b31d");

/// Gas charged by Nitro for returning the 32-byte `arbBlockNumber()` result.
pub const ARB_BLOCK_NUMBER_GAS_COST: u64 = 3;

/// ID for the ArbSys precompile.
pub static PRECOMPILE_ID_ARB_SYS: PrecompileId = PrecompileId::Custom(Cow::Borrowed("ArbSys"));

/// Returns whether `chain_id` is an Arbitrum chain.
pub fn is_arbitrum_chain(chain_id: u64) -> bool {
    Chain::from_id(chain_id).is_arbitrum()
        || matches!(chain_id, ROBINHOOD_MAINNET_CHAIN_ID | ROBINHOOD_TESTNET_CHAIN_ID)
}

/// Returns the ABI-encoded result for `ArbSys.arbBlockNumber()`.
pub fn arb_block_number_output(block_number: u64) -> Bytes {
    Bytes::copy_from_slice(&U256::from(block_number).to_be_bytes::<32>())
}

/// Returns the gas cost and ABI-encoded result for `ArbSys.arbBlockNumber()`.
pub fn arb_block_number_call(gas_limit: u64, block_number: u64) -> Option<(u64, Bytes)> {
    (gas_limit >= ARB_BLOCK_NUMBER_GAS_COST)
        .then(|| (ARB_BLOCK_NUMBER_GAS_COST, arb_block_number_output(block_number)))
}

/// Returns an ArbSys precompile for the provided L2 block number.
pub fn arb_sys_precompile(block_number: u64) -> DynPrecompile {
    DynPrecompile::new_stateful(PRECOMPILE_ID_ARB_SYS.clone(), move |input| {
        arb_sys_precompile_call(input, block_number)
    })
}

fn arb_sys_precompile_call(input: PrecompileInput<'_>, block_number: u64) -> PrecompileResult {
    if input.data.get(..4) != Some(&ARB_BLOCK_NUMBER_SELECTOR) {
        return Ok(PrecompileOutput::halt(
            PrecompileHalt::Other("unsupported ArbSys selector".into()),
            input.reservoir,
        ));
    }

    let Some((gas_cost, output)) = arb_block_number_call(input.gas, block_number) else {
        return Ok(PrecompileOutput::halt(PrecompileHalt::OutOfGas, input.reservoir));
    };

    Ok(PrecompileOutput::new(gas_cost, output, input.reservoir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_robinhood_as_arbitrum() {
        assert!(is_arbitrum_chain(ROBINHOOD_MAINNET_CHAIN_ID));
        assert!(is_arbitrum_chain(ROBINHOOD_TESTNET_CHAIN_ID));
    }
}
