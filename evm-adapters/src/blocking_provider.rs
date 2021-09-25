use ethers::{
    providers::Middleware,
    types::{Address, BlockId, Bytes, H256, U256},
};
use tokio::runtime::Runtime;

#[derive(Debug)]
/// Blocking wrapper around an Ethers middleware, for use in synchronous contexts
/// (powered by a tokio runtime)
pub struct BlockingProvider<M> {
    provider: M,
    runtime: Runtime,
}

#[cfg(feature = "sputnik")]
use sputnik::backend::MemoryVicinity;

impl<M: Middleware> BlockingProvider<M> {
    pub fn new(provider: M) -> Self {
        Self { provider, runtime: Runtime::new().unwrap() }
    }

    #[cfg(feature = "sputnik")]
    pub fn vicinity(&self, pin_block: Option<u64>) -> Result<MemoryVicinity, M::Error> {
        let block_number = if let Some(pin_block) = pin_block {
            pin_block
        } else {
            self.block_on(self.provider.get_block_number())?.as_u64()
        };

        let gas_price = self.block_on(self.provider.get_gas_price())?;
        let chain_id = self.block_on(self.provider.get_chainid())?;
        let block = self.block_on(self.provider.get_block(block_number))?.expect("block not found");

        Ok(MemoryVicinity {
            origin: Address::default(),
            chain_id,
            block_hashes: Vec::new(),
            block_number: block.number.expect("block number not found").as_u64().into(),
            block_coinbase: block.author,
            block_difficulty: block.difficulty,
            block_gas_limit: block.gas_limit,
            block_timestamp: block.timestamp,
            gas_price,
        })
    }

    fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        self.runtime.block_on(f)
    }

    pub fn get_balance(&self, address: Address, block: Option<BlockId>) -> Result<U256, M::Error> {
        self.block_on(self.provider.get_balance(address, block))
    }

    pub fn get_transaction_count(
        &self,
        address: Address,
        block: Option<BlockId>,
    ) -> Result<U256, M::Error> {
        self.block_on(self.provider.get_transaction_count(address, block))
    }

    pub fn get_code(&self, address: Address, block: Option<BlockId>) -> Result<Bytes, M::Error> {
        self.block_on(self.provider.get_code(address, block))
    }

    pub fn get_storage_at(
        &self,
        address: Address,
        slot: H256,
        block: Option<BlockId>,
    ) -> Result<H256, M::Error> {
        self.block_on(self.provider.get_storage_at(address, slot, block))
    }
}
