use crate::BlockingProvider;

use sputnik::backend::{Backend, Basic, MemoryAccount};

use ethers::{
    providers::Middleware,
    types::{BlockId, H160, H256, U256},
};
use std::collections::BTreeMap;

/// Memory backend with ability to fork another chain from an HTTP provider, storing all cache
/// values in a `BTreeMap` in memory.
#[derive(Clone, Debug)]
// TODO: Add option to easily 1. impersonate accounts, 2. roll back to pinned block
pub struct ForkMemoryBackend<B, M> {
    /// ethers middleware for querying on-chain data
    pub provider: BlockingProvider<M>,
    /// The internal backend
    pub backend: B,
    /// cache state
    // TODO: Actually cache values in memory.
    // TODO: This should probably be abstracted away into something that efficiently
    // also caches at disk etc.
    pub cache: BTreeMap<H160, MemoryAccount>,
    /// The block to fetch data from.
    // This is an `Option` so that we can have less code churn in the functions below
    pin_block: Option<BlockId>,
}

impl<B: Backend, M: Middleware> ForkMemoryBackend<B, M> {
    pub fn new(provider: M, backend: B) -> Self {
        let provider = BlockingProvider::new(provider);
        let pin_block = Some(backend.block_number().as_u64().into());
        Self { provider, backend, cache: Default::default(), pin_block }
    }
}

impl<B: Backend, M: Middleware> Backend for ForkMemoryBackend<B, M> {
    fn exists(&self, address: H160) -> bool {
        let mut exists = self.cache.contains_key(&address);

        // check non-zero balance
        if !exists {
            let balance = self.provider.get_balance(address, self.pin_block).unwrap_or_default();
            exists = balance != U256::zero();
        }

        // check non-zero nonce
        if !exists {
            let nonce =
                self.provider.get_transaction_count(address, self.pin_block).unwrap_or_default();
            exists = nonce != U256::zero();
        }

        // check non-empty code
        if !exists {
            let code = self.provider.get_code(address, self.pin_block).unwrap_or_default();
            exists = !code.0.is_empty();
        }

        exists
    }

    fn basic(&self, address: H160) -> Basic {
        self.cache
            .get(&address)
            .map(|a| Basic { balance: a.balance, nonce: a.nonce })
            .unwrap_or_else(|| Basic {
                balance: self.provider.get_balance(address, self.pin_block).unwrap_or_default(),
                nonce: self
                    .provider
                    .get_transaction_count(address, self.pin_block)
                    .unwrap_or_default(),
            })
    }

    fn code(&self, address: H160) -> Vec<u8> {
        self.cache.get(&address).map(|v| v.code.clone()).unwrap_or_else(|| {
            self.provider.get_code(address, self.pin_block).unwrap_or_default().to_vec()
        })
    }

    fn storage(&self, address: H160, index: H256) -> H256 {
        if let Some(acct) = self.cache.get(&address) {
            if let Some(store_data) = acct.storage.get(&index) {
                *store_data
            } else {
                self.provider.get_storage_at(address, index, self.pin_block).unwrap_or_default()
            }
        } else {
            self.provider.get_storage_at(address, index, self.pin_block).unwrap_or_default()
        }
    }

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
        self.backend.block_number()
    }

    fn block_coinbase(&self) -> H160 {
        self.backend.block_coinbase()
    }

    fn block_timestamp(&self) -> U256 {
        self.backend.block_timestamp()
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

    fn original_storage(&self, address: H160, index: H256) -> Option<H256> {
        Some(self.storage(address, index))
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        sputnik::{helpers::new_backend, vicinity, Executor},
        test_helpers::COMPILED,
        Evm,
    };
    use ethers::{
        providers::{Http, Provider},
        types::Address,
    };
    use sputnik::Config;
    use std::convert::TryFrom;
    use tokio::runtime::Runtime;

    use super::*;

    #[test]
    fn forked_backend() {
        let cfg = Config::istanbul();
        let compiled = COMPILED.get("Greeter").expect("could not find contract");
        let addr = "0x1000000000000000000000000000000000000000".parse().unwrap();

        let provider = Provider::<Http>::try_from(
            "https://mainnet.infura.io/v3/c60b0bb42f8a4c6481ecd229eddaca27",
        )
        .unwrap();
        let rt = Runtime::new().unwrap();
        let vicinity = rt.block_on(vicinity(&provider, Some(13292465))).unwrap();
        let backend = new_backend(&vicinity, Default::default());
        let backend = ForkMemoryBackend::new(provider, backend);

        let mut evm = Executor::new(12_000_000, &cfg, &backend);
        evm.initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        // call the setup function to deploy the contracts inside the test

        let (res, _, _) = evm
            .call::<U256, _>(
                Address::zero(),
                addr,
                &dapp_utils::get_func("function time() public view returns (uint256)").unwrap(),
                (),
                0.into(),
            )
            .unwrap();

        // https://etherscan.io/block/13292465
        assert_eq!(res.as_u64(), 1632539668);
    }
}
