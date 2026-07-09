//! Minimal Arbitrum system contract compatibility helpers.

use std::borrow::Cow;

use alloy_chains::Chain;
use alloy_evm::precompiles::{DynPrecompile, PrecompileInput};
use alloy_primitives::{Address, Bytes, U256, address, hex};
use revm::precompile::{PrecompileHalt, PrecompileId, PrecompileOutput, PrecompileResult};

/// ArbSys system contract address.
pub const ARB_SYS_ADDRESS: Address = address!("0000000000000000000000000000000000000064");

/// `ArbSys.arbBlockNumber()` selector.
pub const ARB_BLOCK_NUMBER_SELECTOR: [u8; 4] = hex!("a3b1b31d");

/// ID for the ArbSys precompile.
pub static PRECOMPILE_ID_ARB_SYS: PrecompileId = PrecompileId::Custom(Cow::Borrowed("ArbSys"));

/// Returns whether `chain_id` is an Arbitrum chain.
pub fn is_arbitrum_chain(chain_id: u64) -> bool {
    Chain::from_id(chain_id).is_arbitrum()
}

/// Returns the ABI-encoded result for `ArbSys.arbBlockNumber()`.
pub fn arb_block_number_output(block_number: u64) -> Bytes {
    Bytes::copy_from_slice(&U256::from(block_number).to_be_bytes::<32>())
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

    Ok(PrecompileOutput::new(0, arb_block_number_output(block_number), input.reservoir))
}
