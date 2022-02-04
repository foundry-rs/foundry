//! Cheatcode-enabled backend implementation
use ethers::types::{H160, H256, U256};
use sputnik::backend::{Backend, Basic};

use crate::sputnik::macros::forward_backend_methods;

use super::Cheatcodes;

#[derive(Debug, Clone)]
/// A cheatcode backend is a wrapper around the inner backend which returns the
/// cheatcode value if it's already been set, else it falls back to the default value
/// inside the backend.
///
/// The cheatcode backend can be composed with other enhanced backends, e.g. the forking
/// backend. You should always put the cheatcode backend on the highest layer of your
/// stack of backend middlewares, so that it is always hit first.
pub struct CheatcodeBackend<B> {
    /// The inner backend type.
    pub backend: B,
    /// The enabled cheatcodes
    pub cheats: Cheatcodes,
}

impl<B: Backend> Backend for CheatcodeBackend<B> {
    fn origin(&self) -> H160 {
        self.cheats.origin.unwrap_or_else(|| self.backend.origin())
    }

    fn block_number(&self) -> U256 {
        self.cheats.block_number.unwrap_or_else(|| self.backend.block_number())
    }

    fn block_timestamp(&self) -> U256 {
        self.cheats.block_timestamp.unwrap_or_else(|| self.backend.block_timestamp())
    }

    fn block_base_fee_per_gas(&self) -> U256 {
        self.cheats.block_base_fee_per_gas.unwrap_or_else(|| self.backend.block_base_fee_per_gas())
    }

    forward_backend_methods! {
        gas_price() -> U256,
        block_hash(number: U256) -> H256,
        block_coinbase() -> H160,
        block_difficulty() -> U256,
        block_gas_limit() -> U256,
        chain_id() -> U256,
        exists(address: H160) -> bool,
        basic(address: H160) -> Basic,
        code(address: H160) -> Vec<u8>,
        storage(address: H160, index: H256) -> H256,
        original_storage(address: H160, index: H256) -> Option<H256>
    }
}
