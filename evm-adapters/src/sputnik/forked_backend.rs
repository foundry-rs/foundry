use crate::BlockingProvider;

use sputnik::backend::{Backend, Basic, MemoryAccount, MemoryVicinity};

use ethers::{
    providers::Middleware,
    types::{H160, H256, U256},
};
use std::collections::BTreeMap;

/// Memory backend with ability to fork another chain from an HTTP provider, storing all state
/// values in a `BTreeMap` in memory.
#[derive(Clone, Debug)]
// TODO: Add option to easily 1. impersonate accounts, 2. roll back to pinned block
pub struct ForkMemoryBackend<M> {
    /// ethers middleware for querying on-chain data
    pub provider: BlockingProvider<M>,
    /// the global context of the chain
    pub vicinity: MemoryVicinity,
    /// state cache
    // TODO: This should probably be abstracted away into something that efficiently
    // also caches at disk etc.
    pub state: BTreeMap<H160, MemoryAccount>,
}

impl<M: Middleware> ForkMemoryBackend<M> {
    /// Create a new memory backend given a provider, an optional block to pin state
    /// against and a state tree
    pub fn new(provider: M, pin_block: Option<u64>, state: BTreeMap<H160, MemoryAccount>) -> Self {
        let provider = BlockingProvider::new(provider);
        let vicinity = provider
            .vicinity(pin_block)
            .expect("could not instantiate vicinity corresponding to upstream");
        Self { provider, vicinity, state }
    }
}

impl<M: Middleware> Backend for ForkMemoryBackend<M> {
    fn gas_price(&self) -> U256 {
        self.vicinity.gas_price
    }

    fn origin(&self) -> H160 {
        self.vicinity.origin
    }

    fn block_hash(&self, number: U256) -> H256 {
        if number >= self.vicinity.block_number ||
            self.vicinity.block_number - number - U256::one() >=
                U256::from(self.vicinity.block_hashes.len())
        {
            H256::default()
        } else {
            let index = (self.vicinity.block_number - number - U256::one()).as_usize();
            self.vicinity.block_hashes[index]
        }
    }

    fn block_number(&self) -> U256 {
        self.vicinity.block_number
    }

    fn block_coinbase(&self) -> H160 {
        self.vicinity.block_coinbase
    }

    fn block_timestamp(&self) -> U256 {
        self.vicinity.block_timestamp
    }

    fn block_difficulty(&self) -> U256 {
        self.vicinity.block_difficulty
    }

    fn block_gas_limit(&self) -> U256 {
        self.vicinity.block_gas_limit
    }

    fn chain_id(&self) -> U256 {
        self.vicinity.chain_id
    }

    fn exists(&self, address: H160) -> bool {
        let mut exists = self.state.contains_key(&address);

        // check non-zero balance
        if !exists {
            let balance = self
                .provider
                .get_balance(address, Some(self.vicinity.block_number.as_u64().into()))
                .unwrap_or_default();
            exists = balance != U256::zero();
        }

        // check non-zero nonce
        if !exists {
            let nonce = self
                .provider
                .get_transaction_count(address, Some(self.vicinity.block_number.as_u64().into()))
                .unwrap_or_default();
            exists = nonce != U256::zero();
        }

        // check non-empty code
        if !exists {
            let code = self
                .provider
                .get_code(address, Some(self.vicinity.block_number.as_u64().into()))
                .unwrap_or_default();
            exists = !code.0.is_empty();
        }

        exists
    }

    fn basic(&self, address: H160) -> Basic {
        self.state
            .get(&address)
            .map(|a| Basic { balance: a.balance, nonce: a.nonce })
            .unwrap_or_else(|| Basic {
                balance: self
                    .provider
                    .get_balance(address, Some(self.vicinity.block_number.as_u64().into()))
                    .unwrap_or_default(),
                nonce: self
                    .provider
                    .get_transaction_count(
                        address,
                        Some(self.vicinity.block_number.as_u64().into()),
                    )
                    .unwrap_or_default(),
            })
    }

    fn code(&self, address: H160) -> Vec<u8> {
        self.state.get(&address).map(|v| v.code.clone()).unwrap_or_else(|| {
            self.provider
                .get_code(address, Some(self.vicinity.block_number.as_u64().into()))
                .unwrap_or_default()
                .to_vec()
        })
    }

    fn storage(&self, address: H160, index: H256) -> H256 {
        if let Some(acct) = self.state.get(&address) {
            if let Some(store_data) = acct.storage.get(&index) {
                *store_data
            } else {
                self.provider
                    .get_storage_at(
                        address,
                        index,
                        Some(self.vicinity.block_number.as_u64().into()),
                    )
                    .unwrap_or_default()
            }
        } else {
            self.provider
                .get_storage_at(address, index, Some(self.vicinity.block_number.as_u64().into()))
                .unwrap_or_default()
        }
    }

    fn original_storage(&self, address: H160, index: H256) -> Option<H256> {
        Some(self.storage(address, index))
    }
}

#[cfg(test)]
mod tests {
    use crate::{sputnik::Executor, test_helpers::COMPILED, Evm};
    use ethers::{
        providers::{Http, Provider},
        types::Address,
    };
    use sputnik::Config;
    use std::convert::TryFrom;

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
        let backend = ForkMemoryBackend::new(provider, Some(13292465), Default::default());
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
