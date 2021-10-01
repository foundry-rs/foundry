pub mod memory_stackstate_owned;

pub mod cheatcode_handler;
pub use cheatcode_handler::CheatcodeHandler;

mod backend;

use ethers::{
    abi::parse_abi,
    prelude::{BaseContract, Lazy},
    types::U256,
};

#[derive(Clone, Debug, Default)]
/// Cheatcodes can be used to control the EVM context during setup or runtime,
/// which can be useful for simulations or specialized unti tests
pub struct Cheatcodes {
    pub block_number: Option<U256>,
    pub block_timestamp: Option<U256>,
}

// TODO: Add more cheatcodes.
pub static HEVM: Lazy<BaseContract> = Lazy::new(|| {
    BaseContract::from(
        parse_abi(&[
            // sets the block number to x
            "roll(uint256)",
            // sets the block timestamp to x
            "warp(uint256)",
        ])
        .expect("could not parse hevm cheatcode abi"),
    )
});
