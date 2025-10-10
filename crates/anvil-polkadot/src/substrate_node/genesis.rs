//! Genesis settings

use crate::config::AnvilNodeConfig;
use alloy_genesis::GenesisAccount;
use alloy_primitives::Address;
use codec::Encode;
use polkadot_sdk::{
    sc_chain_spec::{BuildGenesisBlock, resolve_state_version_from_wasm},
    sc_client_api::{BlockImportOperation, backend::Backend},
    sc_executor::RuntimeVersionOf,
    sp_blockchain,
    sp_core::storage::Storage,
    sp_runtime::{
        BuildStorage,
        traits::{Block as BlockT, Hash as HashT, HashingFor, Header as HeaderT},
    },
};
use std::{collections::BTreeMap, marker::PhantomData, sync::Arc};

// Hex-encode key: 0x9527366927478e710d3f7fb77c6d1f89
pub const CHAIN_ID_KEY: [u8; 16] = [
    149u8, 39u8, 54u8, 105u8, 39u8, 71u8, 142u8, 113u8, 13u8, 63u8, 127u8, 183u8, 124u8, 109u8,
    31u8, 137u8,
];

// Hex-encode key: 0xf0c365c3cf59d671eb72da0e7a4113c49f1f0515f462cdcf84e0f1d6045dfcbb
// twox_128(b"Timestamp") ++ twox_128(b"Now")
// corresponds to `Timestamp::Now` storage item in pallet-timestamp
pub const TIMESTAMP_KEY: [u8; 32] = [
    240u8, 195u8, 101u8, 195u8, 207u8, 89u8, 214u8, 113u8, 235u8, 114u8, 218u8, 14u8, 122u8, 65u8,
    19u8, 196u8, 159u8, 31u8, 5u8, 21u8, 244u8, 98u8, 205u8, 207u8, 132u8, 224u8, 241u8, 214u8,
    4u8, 93u8, 252u8, 187u8,
];

// Hex-encode key: 0x26aa394eea5630e07c48ae0c9558cef702a5c1b19ab7a04f536c519aca4983ac
// twox_128(b"System") ++ twox_128(b"Number")
// corresponds to `System::Number` storage item in pallet-system
pub const BLOCK_NUMBER_KEY: [u8; 32] = [
    38u8, 170u8, 57u8, 78u8, 234u8, 86u8, 48u8, 224u8, 124u8, 72u8, 174u8, 12u8, 149u8, 88u8,
    206u8, 247u8, 2u8, 165u8, 193u8, 177u8, 154u8, 183u8, 160u8, 79u8, 83u8, 108u8, 81u8, 154u8,
    202u8, 73u8, 131u8, 172u8,
];

/// Genesis settings
#[derive(Clone, Debug, Default)]
pub struct GenesisConfig {
    /// The chain id of the Substrate chain.
    pub chain_id: u64,
    /// The initial timestamp for the genesis block in milliseconds
    pub timestamp: u64,
    /// All accounts that should be initialised at genesis with their info.
    pub alloc: Option<BTreeMap<Address, GenesisAccount>>,
    /// The initial number for the genesis block
    pub number: u32,
    /// The genesis header base fee
    pub base_fee_per_gas: u64,
    /// The genesis header gas limit.
    pub gas_limit: Option<u128>,
}

impl<'a> From<&'a AnvilNodeConfig> for GenesisConfig {
    fn from(anvil_config: &'a AnvilNodeConfig) -> Self {
        Self {
            chain_id: anvil_config.get_chain_id(),
            // Anvil genesis timestamp is in seconds, while Substrate timestamp is in milliseconds.
            timestamp: anvil_config
                .get_genesis_timestamp()
                .checked_mul(1000)
                .expect("Genesis timestamp overflow"),
            alloc: anvil_config.genesis.as_ref().map(|g| g.alloc.clone()),
            number: anvil_config
                .get_genesis_number()
                .try_into()
                .expect("Genesis block number overflow"),
            base_fee_per_gas: anvil_config.get_base_fee(),
            gas_limit: anvil_config.gas_limit,
        }
    }
}

impl GenesisConfig {
    pub fn as_storage_key_value(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        let storage = vec![
            (CHAIN_ID_KEY.to_vec(), self.chain_id.encode()),
            (TIMESTAMP_KEY.to_vec(), self.timestamp.encode()),
            (BLOCK_NUMBER_KEY.to_vec(), self.number.encode()),
        ];
        // TODO: add other fields
        storage
    }
}

pub struct DevelopmentGenesisBlockBuilder<Block: BlockT, B, E> {
    genesis_number: u32,
    genesis_storage: Storage,
    commit_genesis_state: bool,
    backend: Arc<B>,
    executor: E,
    _phantom: PhantomData<Block>,
}

impl<Block: BlockT, B: Backend<Block>, E: RuntimeVersionOf>
    DevelopmentGenesisBlockBuilder<Block, B, E>
{
    pub fn new(
        genesis_number: u64,
        build_genesis_storage: &dyn BuildStorage,
        commit_genesis_state: bool,
        backend: Arc<B>,
        executor: E,
    ) -> sp_blockchain::Result<Self> {
        let genesis_storage =
            build_genesis_storage.build_storage().map_err(sp_blockchain::Error::Storage)?;
        Self::new_with_storage(
            genesis_number,
            genesis_storage,
            commit_genesis_state,
            backend,
            executor,
        )
    }

    pub fn new_with_storage(
        genesis_number: u64,
        genesis_storage: Storage,
        commit_genesis_state: bool,
        backend: Arc<B>,
        executor: E,
    ) -> sp_blockchain::Result<Self> {
        Ok(Self {
            genesis_number: genesis_number.try_into().map_err(|_| {
                sp_blockchain::Error::Application(
                    format!(
                        "Genesis number {} is too large for u32 (max: {})",
                        genesis_number,
                        u32::MAX
                    )
                    .into(),
                )
            })?,
            genesis_storage,
            commit_genesis_state,
            backend,
            executor,
            _phantom: PhantomData::<Block>,
        })
    }
}

impl<Block: BlockT, B: Backend<Block>, E: RuntimeVersionOf> BuildGenesisBlock<Block>
    for DevelopmentGenesisBlockBuilder<Block, B, E>
{
    type BlockImportOperation = <B as Backend<Block>>::BlockImportOperation;

    fn build_genesis_block(self) -> sp_blockchain::Result<(Block, Self::BlockImportOperation)> {
        let Self {
            genesis_number,
            genesis_storage,
            commit_genesis_state,
            backend,
            executor,
            _phantom,
        } = self;

        let genesis_state_version =
            resolve_state_version_from_wasm::<_, HashingFor<Block>>(&genesis_storage, &executor)?;
        let mut op = backend.begin_operation()?;
        let state_root =
            op.set_genesis_state(genesis_storage, commit_genesis_state, genesis_state_version)?;
        let extrinsics_root = <<<Block as BlockT>::Header as HeaderT>::Hashing as HashT>::trie_root(
            Vec::new(),
            genesis_state_version,
        );
        let genesis_block = Block::new(
            <<Block as BlockT>::Header as HeaderT>::new(
                genesis_number.into(),
                extrinsics_root,
                state_root,
                Default::default(),
                Default::default(),
            ),
            Default::default(),
        );

        Ok((genesis_block, op))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_encoding() {
        let block_number: u32 = 5;
        let timestamp: u64 = 10;
        let chain_id: u64 = 42;
        let genesis_config =
            GenesisConfig { number: block_number, timestamp, chain_id, ..Default::default() };
        let genesis_storage = genesis_config.as_storage_key_value();
        assert!(
            genesis_storage.contains(&(BLOCK_NUMBER_KEY.to_vec(), block_number.encode())),
            "Block number not found in genesis key-value storage"
        );
        assert!(
            genesis_storage.contains(&(TIMESTAMP_KEY.to_vec(), timestamp.encode())),
            "Timestamp not found in genesis key-value storage"
        );
        assert!(
            genesis_storage.contains(&(CHAIN_ID_KEY.to_vec(), chain_id.encode())),
            "Chain id not found in genesis key-value storage"
        );
    }
}
