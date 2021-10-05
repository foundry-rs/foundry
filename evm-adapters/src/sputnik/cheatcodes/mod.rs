pub mod memory_stackstate_owned;

pub mod cheatcode_handler;
use std::collections::HashMap;

pub use cheatcode_handler::CheatcodeHandler;

mod backend;

use ethers::{
    abi::parse_abi,
    prelude::{BaseContract, Lazy},
    types::{Address, H256, U256},
};
use sputnik::backend::{Backend, MemoryAccount, MemoryBackend};

#[derive(Clone, Debug, Default)]
/// Cheatcodes can be used to control the EVM context during setup or runtime,
/// which can be useful for simulations or specialized unti tests
pub struct Cheatcodes {
    pub block_number: Option<U256>,
    pub block_timestamp: Option<U256>,
    pub accounts: HashMap<Address, MemoryAccount>,
}

pub trait BackendExt: Backend {
    fn set_storage(&mut self, address: Address, slot: H256, value: H256);
}

impl<'a> BackendExt for MemoryBackend<'a> {
    fn set_storage(&mut self, address: Address, slot: H256, value: H256) {
        let account = self.state_mut().entry(address).or_insert_with(Default::default);
        let slot = account.storage.entry(slot).or_insert_with(Default::default);
        *slot = value;
    }
}

// TODO: Add more cheatcodes.
pub static HEVM: Lazy<BaseContract> = Lazy::new(|| {
    BaseContract::from(
        parse_abi(&[
            // sets the block number to x
            "roll(uint256)",
            // sets the block timestamp to x
            "warp(uint256)",
            // sets account at `address`'s storage `slot` to `value`
            "store(address,bytes32,bytes32)",
            // returns the `value` of the storage `slot` at `address`
            "load(address,bytes32)(bytes32)",
        ])
        .expect("could not parse hevm cheatcode abi"),
    )
});
