//! Simple in-memory cache backend for use with forking providers
use std::{cell::RefCell, collections::BTreeMap};

use ethers::{
    providers::Middleware,
    types::{Block, BlockId, BlockNumber, TxHash, H160, H256, U256},
};
use sputnik::backend::{Backend, Basic, MemoryAccount};

use crate::BlockingProvider;

/// Memory backend with ability to fork another chain from an HTTP provider, storing all cache
/// values in a `BTreeMap` in memory.
// TODO: Add option to easily 1. impersonate accounts, 2. roll back to pinned block
// TODO: In order to improve speed, does it make sense to add a job which pre-fetches
// accounts speculatively? Or maybe do it for smart contract code which is typically the
// biggest issue?
// TODO: In order to improve speed, can we instead write a custom blocking provider which
// does not block_on in-line, but has a background thread that polls everything in parallel
// and just returns the results synchronously via some channel?
pub struct ForkMemoryBackend<B, M> {
    /// ethers middleware for querying on-chain data
    pub provider: BlockingProvider<M>,
    /// The internal backend
    pub backend: B,
    /// cache state
    // TODO: Actually cache values in memory.
    // TODO: This should probably be abstracted away into something that efficiently
    // also caches at disk etc.
    pub cache: RefCell<BTreeMap<H160, MemoryAccount>>,
    /// The block to fetch data from.
    // This is an `Option` so that we can have less code churn in the functions below
    pin_block: Option<BlockId>,
    /// The block at which we forked off
    pin_block_meta: Block<TxHash>,
    /// The chain id of the forked chain
    chain_id: U256,
}

impl<B: Backend, M: Middleware> ForkMemoryBackend<B, M>
where
    M::Error: 'static,
{
    pub fn new(
        provider: M,
        backend: B,
        pin_block: Option<u64>,
        init_cache: BTreeMap<H160, MemoryAccount>,
    ) -> Self {
        let provider = BlockingProvider::new(provider);

        // get the remaining block metadata
        let (block, chain_id) =
            provider.block_and_chainid(pin_block).expect("could not get block meta and chain id");

        Self {
            provider,
            backend,
            cache: RefCell::new(init_cache),
            pin_block: pin_block.map(Into::into),
            pin_block_meta: block,
            chain_id,
        }
    }
}

impl<B: Backend, M: Middleware> Backend for ForkMemoryBackend<B, M>
where
    M::Error: 'static,
{
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
        self.pin_block
            .and_then(|block| match block {
                BlockId::Number(num) => match num {
                    BlockNumber::Number(num) => Some(num.as_u64().into()),
                    _ => None,
                },
                BlockId::Hash(_) => None,
            })
            .unwrap_or_else(|| self.backend.block_number())
    }

    fn block_coinbase(&self) -> H160 {
        self.pin_block_meta.author
    }

    fn block_timestamp(&self) -> U256 {
        self.pin_block_meta.timestamp
    }

    fn block_difficulty(&self) -> U256 {
        self.pin_block_meta.difficulty
    }

    fn block_gas_limit(&self) -> U256 {
        self.pin_block_meta.gas_limit
    }

    fn block_base_fee_per_gas(&self) -> U256 {
        self.pin_block_meta.base_fee_per_gas.unwrap_or_default()
    }

    fn chain_id(&self) -> U256 {
        self.chain_id
    }

    fn exists(&self, address: H160) -> bool {
        let mut exists = self.cache.borrow().contains_key(&address);

        // check non-zero balance
        if !exists {
            let mut cache = self.cache.borrow_mut();
            let account = cache.entry(address).or_insert_with(|| {
                let res = self.provider.get_account(address, self.pin_block).unwrap_or_default();
                MemoryAccount {
                    nonce: res.0,
                    balance: res.1,
                    code: res.2.to_vec(),
                    storage: Default::default(),
                }
            });
            exists = account.balance != U256::zero() ||
                account.nonce != U256::zero() ||
                !account.code.is_empty();
        }

        exists
    }

    fn basic(&self, address: H160) -> Basic {
        let mut cache = self.cache.borrow_mut();
        let account = cache.entry(address).or_insert_with(|| {
            let res = self.provider.get_account(address, self.pin_block).unwrap_or_default();
            MemoryAccount {
                nonce: res.0,
                balance: res.1,
                code: res.2.to_vec(),
                storage: Default::default(),
            }
        });
        Basic { balance: account.balance, nonce: account.nonce }
    }

    fn code(&self, address: H160) -> Vec<u8> {
        let mut cache = self.cache.borrow_mut();
        let account = cache.entry(address).or_insert_with(|| {
            // println!("didnt have account code {:?}", address);
            let res = self.provider.get_account(address, self.pin_block).unwrap_or_default();
            MemoryAccount {
                nonce: res.0,
                balance: res.1,
                code: res.2.to_vec(),
                storage: Default::default(),
            }
        });
        account.code.clone()
    }

    fn storage(&self, address: H160, index: H256) -> H256 {
        let mut cache = self.cache.borrow_mut();
        let account = cache.entry(address).or_insert_with(|| {
            let res = self.provider.get_account(address, self.pin_block).unwrap_or_default();
            MemoryAccount {
                nonce: res.0,
                balance: res.1,
                code: res.2.to_vec(),
                storage: Default::default(),
            }
        });
        if let Some(val) = account.storage.get(&index) {
            *val
        } else {
            let ret =
                self.provider.get_storage_at(address, index, self.pin_block).unwrap_or_default();
            account.storage.insert(index, ret);
            ret
        }
    }

    fn original_storage(&self, address: H160, index: H256) -> Option<H256> {
        Some(self.storage(address, index))
    }
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;

    use ethers::{
        providers::{Http, Provider},
        types::Address,
    };
    use sputnik::Config;
    use tokio::runtime::Runtime;

    use crate::{
        sputnik::{helpers::new_backend, vicinity, Executor, PRECOMPILES_MAP},
        test_helpers::COMPILED,
        Evm,
    };

    use super::*;

    #[test]
    fn forked_backend() {
        let cfg = Config::istanbul();
        let compiled = COMPILED.find("Greeter").expect("could not find contract");

        let provider = Provider::<Http>::try_from(
            "https://mainnet.infura.io/v3/c60b0bb42f8a4c6481ecd229eddaca27",
        )
        .unwrap();
        let rt = Runtime::new().unwrap();
        let blk = Some(13292465);
        let vicinity = rt.block_on(vicinity(&provider, blk)).unwrap();
        let backend = new_backend(&vicinity, Default::default());
        let backend = ForkMemoryBackend::new(provider, backend, blk, Default::default());

        let precompiles = PRECOMPILES_MAP.clone();
        let mut evm = Executor::new(12_000_000, &cfg, &backend, &precompiles);

        let (addr, _, _, _) =
            evm.deploy(Address::zero(), compiled.bin.unwrap().clone(), 0.into()).unwrap();

        let (res, _, _, _) =
            evm.call::<U256, _, _>(Address::zero(), addr, "time()(uint256)", (), 0.into()).unwrap();

        // https://etherscan.io/block/13292465
        assert_eq!(res.as_u64(), 1632539668);
    }
}
