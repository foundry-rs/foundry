use ethers::{
    prelude::BlockNumber,
    providers::Middleware,
    types::{Address, Block, BlockId, Bytes, TxHash, H256, U256, U64},
};
use foundry_utils::RuntimeOrHandle;

#[derive(Debug)]
/// Blocking wrapper around an Ethers middleware, for use in synchronous contexts
/// (powered by a tokio runtime)
pub struct BlockingProvider<M> {
    provider: M,
    runtime: RuntimeOrHandle,
}

impl<M: Clone> Clone for BlockingProvider<M> {
    fn clone(&self) -> Self {
        Self { provider: self.provider.clone(), runtime: RuntimeOrHandle::new() }
    }
}

impl<M: Middleware> BlockingProvider<M>
where
    M::Error: 'static,
{
    /// Constructs the provider. If no tokio runtime exists, it instantiates one as well.
    pub fn new(provider: M) -> Self {
        Self { provider, runtime: RuntimeOrHandle::new() }
    }

    /// Receives a future and runs it to completion.
    fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        self.runtime.block_on(f)
    }

    /// Gets the specified block as well as the chain id concurrently.
    pub fn block_and_chainid(
        &self,
        block_id: Option<impl Into<BlockId>>,
    ) -> eyre::Result<(Block<TxHash>, U256)> {
        let block_id = block_id.map(Into::into).unwrap_or(BlockId::Number(BlockNumber::Latest));
        let f = async {
            let block = self.provider.get_block(block_id);
            let chain_id = self.provider.get_chainid();
            tokio::try_join!(block, chain_id)
        };
        let (block, chain_id) = self.block_on(f)?;
        Ok((block.ok_or_else(|| eyre::eyre!("block {:?} not found", block_id))?, chain_id))
    }

    /// Gets the nonce, balance and code associated with an account.
    pub fn get_account(
        &self,
        address: Address,
        block_id: Option<BlockId>,
    ) -> eyre::Result<(U256, U256, Bytes)> {
        let f = async {
            let balance = self.provider.get_balance(address, block_id);
            let nonce = self.provider.get_transaction_count(address, block_id);
            let code = self.provider.get_code(address, block_id);
            tokio::try_join!(balance, nonce, code)
        };
        let (balance, nonce, code) = self.block_on(f)?;

        Ok((nonce, balance, code))
    }

    /// Gets the current block number.
    pub fn get_block_number(&self) -> Result<U64, M::Error> {
        self.block_on(self.provider.get_block_number())
    }

    /// Gets the account's balance at the specified block.
    pub fn get_balance(&self, address: Address, block: Option<BlockId>) -> Result<U256, M::Error> {
        self.block_on(self.provider.get_balance(address, block))
    }

    /// Gets the account's nonce at the specified block.
    pub fn get_transaction_count(
        &self,
        address: Address,
        block: Option<BlockId>,
    ) -> Result<U256, M::Error> {
        self.block_on(self.provider.get_transaction_count(address, block))
    }

    /// Gets the account's code at the specified block.
    pub fn get_code(&self, address: Address, block: Option<BlockId>) -> Result<Bytes, M::Error> {
        self.block_on(self.provider.get_code(address, block))
    }

    /// Gets the value at the specified storage slot & block.
    pub fn get_storage_at(
        &self,
        address: Address,
        slot: H256,
        block: Option<BlockId>,
    ) -> Result<H256, M::Error> {
        self.block_on(self.provider.get_storage_at(address, slot, block))
    }
}
