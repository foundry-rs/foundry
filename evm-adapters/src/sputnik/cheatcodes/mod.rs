pub mod memory_stackstate_owned;

mod backend;

use ethers::types::U256;

#[derive(Clone, Debug, Default)]
/// Cheatcodes can be used to control the EVM context during setup or runtime,
/// which can be useful for simulations or specialized unti tests
pub struct Cheatcodes {
    pub block_number: Option<U256>,
    pub block_timestamp: Option<U256>,
}

// TODO: Add Lazy ethabi instance of the cheatcode function signatures
