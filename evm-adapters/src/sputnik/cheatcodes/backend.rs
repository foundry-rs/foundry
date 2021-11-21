//! Cheatcode-enabled backend implementation
use super::Cheatcodes;
use ethers::types::{H160, H256, U256};
use sputnik::backend::{Backend, Basic};

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
    // TODO: Override the return values based on the values of `self.cheats`
    fn gas_price(&self) -> U256 {
        self.backend.gas_price()
    }

    fn origin(&self) -> H160 {
        self.backend.origin()
    }

    fn block_hash(&self, number: U256) -> H256 {
        self.backend.block_hash(number)
    }

    fn block_number(&self) -> U256 {
        self.cheats.block_number.unwrap_or_else(|| self.backend.block_number())
    }

    fn block_coinbase(&self) -> H160 {
        self.backend.block_coinbase()
    }

    fn block_timestamp(&self) -> U256 {
        self.cheats.block_timestamp.unwrap_or_else(|| self.backend.block_timestamp())
    }

    fn block_base_fee_per_gas(&self) -> U256 {
        self.cheats.block_base_fee_per_gas.unwrap_or_else(|| self.backend.block_base_fee_per_gas())
    }

    fn block_difficulty(&self) -> U256 {
        self.backend.block_difficulty()
    }

    fn block_gas_limit(&self) -> U256 {
        self.backend.block_gas_limit()
    }

    fn chain_id(&self) -> U256 {
        self.backend.chain_id()
    }

    fn exists(&self, address: H160) -> bool {
        self.backend.exists(address)
    }

    fn basic(&self, address: H160) -> Basic {
        self.backend.basic(address)
    }

    fn code(&self, address: H160) -> Vec<u8> {
        self.backend.code(address)
    }

    fn storage(&self, address: H160, index: H256) -> H256 {
        self.backend.storage(address, index)
    }

    fn original_storage(&self, address: H160, index: H256) -> Option<H256> {
        self.backend.original_storage(address, index)
    }
}
