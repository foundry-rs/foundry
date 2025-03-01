//! Alloy genesis types

#![doc = include_str!(".././README.md")]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/alloy.jpg",
    html_favicon_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/favicon.ico"
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::{collections::BTreeMap, string::String};
use alloy_eips::eip7840::BlobParams;
use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use alloy_serde::{storage::deserialize_storage_map, ttd::deserialize_json_ttd_opt, OtherFields};
use alloy_trie::{TrieAccount, EMPTY_ROOT_HASH, KECCAK_EMPTY};
use core::str::FromStr;
use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize};

/// The genesis block specification.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Genesis {
    /// The fork configuration for this network.
    #[serde(default)]
    pub config: ChainConfig,
    /// The genesis header nonce.
    #[serde(with = "alloy_serde::quantity")]
    pub nonce: u64,
    /// The genesis header timestamp.
    #[serde(with = "alloy_serde::quantity")]
    pub timestamp: u64,
    /// The genesis header extra data.
    pub extra_data: Bytes,
    /// The genesis header gas limit.
    #[serde(with = "alloy_serde::quantity")]
    pub gas_limit: u64,
    /// The genesis header difficulty.
    pub difficulty: U256,
    /// The genesis header mix hash.
    pub mix_hash: B256,
    /// The genesis header coinbase address.
    pub coinbase: Address,
    /// The initial state of accounts in the genesis block.
    pub alloc: BTreeMap<Address, GenesisAccount>,
    // NOTE: the following fields:
    // * base_fee_per_gas
    // * excess_blob_gas
    // * blob_gas_used
    // * number
    // should NOT be set in a real genesis file, but are included here for compatibility with
    // consensus tests, which have genesis files with these fields populated.
    /// The genesis header base fee
    #[serde(default, skip_serializing_if = "Option::is_none", with = "alloy_serde::quantity::opt")]
    pub base_fee_per_gas: Option<u128>,
    /// The genesis header excess blob gas
    #[serde(default, skip_serializing_if = "Option::is_none", with = "alloy_serde::quantity::opt")]
    pub excess_blob_gas: Option<u64>,
    /// The genesis header blob gas used
    #[serde(default, skip_serializing_if = "Option::is_none", with = "alloy_serde::quantity::opt")]
    pub blob_gas_used: Option<u64>,
    /// The genesis block number
    #[serde(default, skip_serializing_if = "Option::is_none", with = "alloy_serde::quantity::opt")]
    pub number: Option<u64>,
}

impl Genesis {
    /// Creates a chain config for Clique using the given chain id and funds the given address with
    /// max coins.
    ///
    /// Enables all hard forks up to London at genesis.
    pub fn clique_genesis(chain_id: u64, signer_addr: Address) -> Self {
        // set up a clique config with an instant sealing period and short (8 block) epoch
        let clique_config = CliqueConfig { period: Some(0), epoch: Some(8) };

        let config = ChainConfig {
            chain_id,
            eip155_block: Some(0),
            eip150_block: Some(0),
            eip158_block: Some(0),

            homestead_block: Some(0),
            byzantium_block: Some(0),
            constantinople_block: Some(0),
            petersburg_block: Some(0),
            istanbul_block: Some(0),
            muir_glacier_block: Some(0),
            berlin_block: Some(0),
            london_block: Some(0),
            clique: Some(clique_config),
            ..Default::default()
        };

        // fund account
        let alloc = BTreeMap::from([(
            signer_addr,
            GenesisAccount { balance: U256::MAX, ..Default::default() },
        )]);

        // put signer address in the extra data, padded by the required amount of zeros
        // Clique issue: https://github.com/ethereum/EIPs/issues/225
        // Clique EIP: https://eips.ethereum.org/EIPS/eip-225
        //
        // The first 32 bytes are vanity data, so we will populate it with zeros
        // This is followed by the signer address, which is 20 bytes
        // There are 65 bytes of zeros after the signer address, which is usually populated with the
        // proposer signature. Because the genesis does not have a proposer signature, it will be
        // populated with zeros.
        let extra_data_bytes = [&[0u8; 32][..], signer_addr.as_slice(), &[0u8; 65][..]].concat();
        let extra_data = extra_data_bytes.into();

        Self {
            config,
            alloc,
            difficulty: U256::from(1),
            gas_limit: 5_000_000,
            extra_data,
            ..Default::default()
        }
    }

    /// Set the nonce.
    pub const fn with_nonce(mut self, nonce: u64) -> Self {
        self.nonce = nonce;
        self
    }

    /// Set the timestamp.
    pub const fn with_timestamp(mut self, timestamp: u64) -> Self {
        self.timestamp = timestamp;
        self
    }

    /// Set the extra data.
    pub fn with_extra_data(mut self, extra_data: Bytes) -> Self {
        self.extra_data = extra_data;
        self
    }

    /// Set the gas limit.
    pub const fn with_gas_limit(mut self, gas_limit: u64) -> Self {
        self.gas_limit = gas_limit;
        self
    }

    /// Set the difficulty.
    pub const fn with_difficulty(mut self, difficulty: U256) -> Self {
        self.difficulty = difficulty;
        self
    }

    /// Set the mix hash of the header.
    pub const fn with_mix_hash(mut self, mix_hash: B256) -> Self {
        self.mix_hash = mix_hash;
        self
    }

    /// Set the coinbase address.
    pub const fn with_coinbase(mut self, address: Address) -> Self {
        self.coinbase = address;
        self
    }

    /// Set the base fee.
    pub const fn with_base_fee(mut self, base_fee: Option<u128>) -> Self {
        self.base_fee_per_gas = base_fee;
        self
    }

    /// Set the excess blob gas.
    pub const fn with_excess_blob_gas(mut self, excess_blob_gas: Option<u64>) -> Self {
        self.excess_blob_gas = excess_blob_gas;
        self
    }

    /// Set the blob gas used.
    pub const fn with_blob_gas_used(mut self, blob_gas_used: Option<u64>) -> Self {
        self.blob_gas_used = blob_gas_used;
        self
    }

    /// Add accounts to the genesis block. If the address is already present,
    /// the account is updated.
    pub fn extend_accounts(
        mut self,
        accounts: impl IntoIterator<Item = (Address, GenesisAccount)>,
    ) -> Self {
        self.alloc.extend(accounts);
        self
    }
}

/// An account in the state of the genesis block.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenesisAccount {
    /// The nonce of the account at genesis.
    #[serde(skip_serializing_if = "Option::is_none", with = "alloy_serde::quantity::opt", default)]
    pub nonce: Option<u64>,
    /// The balance of the account at genesis.
    pub balance: U256,
    /// The account's bytecode at genesis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<Bytes>,
    /// The account's storage at genesis.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_storage_map"
    )]
    pub storage: Option<BTreeMap<B256, B256>>,
    /// The account's private key. Should only be used for testing.
    #[serde(
        rename = "secretKey",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_private_key"
    )]
    pub private_key: Option<B256>,
}

impl GenesisAccount {
    /// Set the nonce.
    pub const fn with_nonce(mut self, nonce: Option<u64>) -> Self {
        self.nonce = nonce;
        self
    }

    /// Set the balance.
    pub const fn with_balance(mut self, balance: U256) -> Self {
        self.balance = balance;
        self
    }

    /// Set the code.
    pub fn with_code(mut self, code: Option<Bytes>) -> Self {
        self.code = code;
        self
    }

    /// Set the storage.
    pub fn with_storage(mut self, storage: Option<BTreeMap<B256, B256>>) -> Self {
        self.storage = storage;
        self
    }

    /// Returns an iterator over the storage slots in (`B256`, `U256`) format.
    pub fn storage_slots(&self) -> impl Iterator<Item = (B256, U256)> + '_ {
        self.storage.as_ref().into_iter().flat_map(|storage| storage.iter()).map(|(key, value)| {
            let value = U256::from_be_bytes(value.0);
            (*key, value)
        })
    }

    /// Convert the genesis account into the [`TrieAccount`] format.
    pub fn into_trie_account(self) -> TrieAccount {
        self.into()
    }
}

impl From<GenesisAccount> for TrieAccount {
    fn from(account: GenesisAccount) -> Self {
        let storage_root = account
            .storage
            .map(|storage| {
                alloy_trie::root::storage_root_unhashed(
                    storage
                        .into_iter()
                        .filter(|(_, value)| !value.is_zero())
                        .map(|(slot, value)| (slot, U256::from_be_bytes(*value))),
                )
            })
            .unwrap_or(EMPTY_ROOT_HASH);

        Self {
            nonce: account.nonce.unwrap_or_default(),
            balance: account.balance,
            storage_root,
            code_hash: account.code.map_or(KECCAK_EMPTY, keccak256),
        }
    }
}

/// Custom deserialization function for the private key.
///
/// This function allows the private key to be deserialized from a string or a `null` value.
///
/// We need a custom function here especially to handle the case where the private key is `0x` and
/// should be deserialized as `None`.
fn deserialize_private_key<'de, D>(deserializer: D) -> Result<Option<B256>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt_str: Option<String> = Option::deserialize(deserializer)?;

    if let Some(ref s) = opt_str {
        if s == "0x" {
            return Ok(None);
        }
        B256::from_str(s).map(Some).map_err(D::Error::custom)
    } else {
        Ok(None)
    }
}

/// Defines core blockchain settings per block.
///
/// Tailors unique settings for each network based on its genesis block.
///
/// Governs crucial blockchain behavior and adaptability.
///
/// Encapsulates parameters shaping network evolution and behavior.
///
/// See [geth's `ChainConfig`
/// struct](https://github.com/ethereum/go-ethereum/blob/v1.14.0/params/config.go#L326)
/// for the source of each field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChainConfig {
    /// The network's chain ID.
    pub chain_id: u64,

    /// The homestead switch block (None = no fork, 0 = already homestead).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub homestead_block: Option<u64>,

    /// The DAO fork switch block (None = no fork).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub dao_fork_block: Option<u64>,

    /// Whether or not the node supports the DAO hard-fork.
    pub dao_fork_support: bool,

    /// The [EIP-150](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-150.md) hard fork block (None = no fork).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub eip150_block: Option<u64>,

    /// The [EIP-155](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-155.md) hard fork block.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub eip155_block: Option<u64>,

    /// The [EIP-158](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-158.md) hard fork block.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub eip158_block: Option<u64>,

    /// The Byzantium hard fork block (None = no fork, 0 = already on byzantium).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub byzantium_block: Option<u64>,

    /// The Constantinople hard fork block (None = no fork, 0 = already on constantinople).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub constantinople_block: Option<u64>,

    /// The Petersburg hard fork block (None = no fork, 0 = already on petersburg).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub petersburg_block: Option<u64>,

    /// The Istanbul hard fork block (None = no fork, 0 = already on istanbul).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub istanbul_block: Option<u64>,

    /// The Muir Glacier hard fork block (None = no fork, 0 = already on muir glacier).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub muir_glacier_block: Option<u64>,

    /// The Berlin hard fork block (None = no fork, 0 = already on berlin).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub berlin_block: Option<u64>,

    /// The London hard fork block (None = no fork, 0 = already on london).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub london_block: Option<u64>,

    /// The Arrow Glacier hard fork block (None = no fork, 0 = already on arrow glacier).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub arrow_glacier_block: Option<u64>,

    /// The Gray Glacier hard fork block (None = no fork, 0 = already on gray glacier).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub gray_glacier_block: Option<u64>,

    /// Virtual fork after the merge to use as a network splitter.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub merge_netsplit_block: Option<u64>,

    /// Shanghai switch time (None = no fork, 0 = already on shanghai).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub shanghai_time: Option<u64>,

    /// Cancun switch time (None = no fork, 0 = already on cancun).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub cancun_time: Option<u64>,

    /// Prague switch time (None = no fork, 0 = already on prague).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub prague_time: Option<u64>,

    /// Osaka switch time (None = no fork, 0 = already on osaka).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "alloy_serde::quantity::opt::deserialize"
    )]
    pub osaka_time: Option<u64>,

    /// Total difficulty reached that triggers the merge consensus upgrade.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_json_ttd_opt"
    )]
    pub terminal_total_difficulty: Option<U256>,

    /// A flag specifying that the network already passed the terminal total difficulty. Its
    /// purpose is to disable legacy sync without having seen the TTD locally.
    pub terminal_total_difficulty_passed: bool,

    /// Ethash parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ethash: Option<EthashConfig>,

    /// Clique parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clique: Option<CliqueConfig>,

    /// Parlia parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parlia: Option<ParliaConfig>,

    /// Additional fields specific to each chain.
    #[serde(flatten, default)]
    pub extra_fields: OtherFields,

    /// The deposit contract address
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deposit_contract_address: Option<Address>,

    /// The blob schedule for the chain, indexed by hardfork name.
    ///
    /// See [EIP-7840](https://github.com/ethereum/EIPs/tree/master/EIPS/eip-7840.md).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub blob_schedule: BTreeMap<String, BlobParams>,
}

impl ChainConfig {
    /// Checks if the blockchain is active at or after the Homestead fork block.
    pub fn is_homestead_active_at_block(&self, block: u64) -> bool {
        self.is_active_at_block(self.homestead_block, block)
    }

    /// Checks if the blockchain is active at or after the EIP150 fork block.
    pub fn is_eip150_active_at_block(&self, block: u64) -> bool {
        self.is_active_at_block(self.eip150_block, block)
    }

    /// Checks if the blockchain is active at or after the EIP155 fork block.
    pub fn is_eip155_active_at_block(&self, block: u64) -> bool {
        self.is_active_at_block(self.eip155_block, block)
    }

    /// Checks if the blockchain is active at or after the EIP158 fork block.
    pub fn is_eip158_active_at_block(&self, block: u64) -> bool {
        self.is_active_at_block(self.eip158_block, block)
    }

    /// Checks if the blockchain is active at or after the Byzantium fork block.
    pub fn is_byzantium_active_at_block(&self, block: u64) -> bool {
        self.is_active_at_block(self.byzantium_block, block)
    }

    /// Checks if the blockchain is active at or after the Constantinople fork block.
    pub fn is_constantinople_active_at_block(&self, block: u64) -> bool {
        self.is_active_at_block(self.constantinople_block, block)
    }

    /// Checks if the blockchain is active at or after the Muir Glacier (EIP-2384) fork block.
    pub fn is_muir_glacier_active_at_block(&self, block: u64) -> bool {
        self.is_active_at_block(self.muir_glacier_block, block)
    }

    /// Checks if the blockchain is active at or after the Petersburg fork block.
    pub fn is_petersburg_active_at_block(&self, block: u64) -> bool {
        self.is_active_at_block(self.petersburg_block, block)
    }

    /// Checks if the blockchain is active at or after the Istanbul fork block.
    pub fn is_istanbul_active_at_block(&self, block: u64) -> bool {
        self.is_active_at_block(self.istanbul_block, block)
    }

    /// Checks if the blockchain is active at or after the Berlin fork block.
    pub fn is_berlin_active_at_block(&self, block: u64) -> bool {
        self.is_active_at_block(self.berlin_block, block)
    }

    /// Checks if the blockchain is active at or after the London fork block.
    pub fn is_london_active_at_block(&self, block: u64) -> bool {
        self.is_active_at_block(self.london_block, block)
    }

    /// Checks if the blockchain is active at or after the Arrow Glacier (EIP-4345) fork block.
    pub fn is_arrow_glacier_active_at_block(&self, block: u64) -> bool {
        self.is_active_at_block(self.arrow_glacier_block, block)
    }

    /// Checks if the blockchain is active at or after the Gray Glacier (EIP-5133) fork block.
    pub fn is_gray_glacier_active_at_block(&self, block: u64) -> bool {
        self.is_active_at_block(self.gray_glacier_block, block)
    }

    /// Checks if the blockchain is active at or after the Shanghai fork block and the specified
    /// timestamp.
    pub fn is_shanghai_active_at_block_and_timestamp(&self, block: u64, timestamp: u64) -> bool {
        self.is_london_active_at_block(block)
            && self.is_active_at_timestamp(self.shanghai_time, timestamp)
    }

    /// Checks if the blockchain is active at or after the Cancun fork block and the specified
    /// timestamp.
    pub fn is_cancun_active_at_block_and_timestamp(&self, block: u64, timestamp: u64) -> bool {
        self.is_london_active_at_block(block)
            && self.is_active_at_timestamp(self.cancun_time, timestamp)
    }

    // Private function handling the comparison logic for block numbers
    fn is_active_at_block(&self, config_block: Option<u64>, block: u64) -> bool {
        config_block.is_some_and(|cb| cb <= block)
    }

    // Private function handling the comparison logic for timestamps
    fn is_active_at_timestamp(&self, config_timestamp: Option<u64>, timestamp: u64) -> bool {
        config_timestamp.is_some_and(|cb| cb <= timestamp)
    }
}

impl Default for ChainConfig {
    fn default() -> Self {
        Self {
            // mainnet
            chain_id: 1,
            homestead_block: None,
            dao_fork_block: None,
            dao_fork_support: false,
            eip150_block: None,
            eip155_block: None,
            eip158_block: None,
            byzantium_block: None,
            constantinople_block: None,
            petersburg_block: None,
            istanbul_block: None,
            muir_glacier_block: None,
            berlin_block: None,
            london_block: None,
            arrow_glacier_block: None,
            gray_glacier_block: None,
            merge_netsplit_block: None,
            shanghai_time: None,
            cancun_time: None,
            prague_time: None,
            osaka_time: None,
            terminal_total_difficulty: None,
            terminal_total_difficulty_passed: false,
            ethash: None,
            clique: None,
            parlia: None,
            extra_fields: Default::default(),
            deposit_contract_address: None,
            blob_schedule: Default::default(),
        }
    }
}

/// Empty consensus configuration for proof-of-work networks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthashConfig {}

/// Consensus configuration for Clique.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliqueConfig {
    /// Number of seconds between blocks to enforce.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<u64>,

    /// Epoch length to reset votes and checkpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
}

/// Consensus configuration for Parlia.
///
/// Parlia is the consensus engine for BNB Smart Chain.
/// For the general introduction: <https://docs.bnbchain.org/docs/learn/consensus/>
/// For the specification: <https://github.com/bnb-chain/bsc/blob/master/params/config.go#L558>
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParliaConfig {
    /// Number of seconds between blocks to enforce.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<u64>,

    /// Epoch length to update validator set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{collections::BTreeMap, vec};
    use alloy_primitives::{hex, Bytes};
    use alloy_trie::{root::storage_root_unhashed, TrieAccount};
    use core::str::FromStr;
    use serde_json::json;

    #[test]
    fn genesis_defaults_config() {
        let s = r#"{}"#;
        let genesis: Genesis = serde_json::from_str(s).unwrap();
        assert_eq!(genesis.config.chain_id, 1);
    }

    #[test]
    fn test_genesis() {
        let default_genesis = Genesis::default();

        let nonce = 999;
        let timestamp = 12345;
        let extra_data = Bytes::from(b"extra-data");
        let gas_limit = 333333;
        let difficulty = U256::from(9000);
        let mix_hash =
            hex!("74385b512f1e0e47100907efe2b00ac78df26acba6dd16b0772923068a5801a8").into();
        let coinbase = hex!("265873b6faf3258b3ab0827805386a2a20ed040e").into();
        // create dummy account
        let first_address: Address = hex!("7618a8c597b89e01c66a1f662078992c52a30c9a").into();
        let mut account = BTreeMap::default();
        account.insert(first_address, GenesisAccount::default());

        // check values updated
        let custom_genesis = Genesis::default()
            .with_nonce(nonce)
            .with_timestamp(timestamp)
            .with_extra_data(extra_data.clone())
            .with_gas_limit(gas_limit)
            .with_difficulty(difficulty)
            .with_mix_hash(mix_hash)
            .with_coinbase(coinbase)
            .extend_accounts(account.clone());

        assert_ne!(custom_genesis, default_genesis);
        // check every field
        assert_eq!(custom_genesis.nonce, nonce);
        assert_eq!(custom_genesis.timestamp, timestamp);
        assert_eq!(custom_genesis.extra_data, extra_data);
        assert_eq!(custom_genesis.gas_limit, gas_limit);
        assert_eq!(custom_genesis.difficulty, difficulty);
        assert_eq!(custom_genesis.mix_hash, mix_hash);
        assert_eq!(custom_genesis.coinbase, coinbase);
        assert_eq!(custom_genesis.alloc, account.clone());

        // update existing account
        assert_eq!(custom_genesis.alloc.len(), 1);
        let same_address = first_address;
        let new_alloc_account = GenesisAccount {
            nonce: Some(1),
            balance: U256::from(1),
            code: Some(b"code".into()),
            storage: Some(BTreeMap::default()),
            private_key: None,
        };
        let mut updated_account = BTreeMap::default();
        updated_account.insert(same_address, new_alloc_account);
        let custom_genesis = custom_genesis.extend_accounts(updated_account.clone());
        assert_ne!(account, updated_account);
        assert_eq!(custom_genesis.alloc.len(), 1);

        // add second account
        let different_address = hex!("94e0681e3073dd71cec54b53afe988f39078fd1a").into();
        let more_accounts = BTreeMap::from([(different_address, GenesisAccount::default())]);
        let custom_genesis = custom_genesis.extend_accounts(more_accounts);
        assert_eq!(custom_genesis.alloc.len(), 2);

        // ensure accounts are different
        let first_account = custom_genesis.alloc.get(&first_address);
        let second_account = custom_genesis.alloc.get(&different_address);
        assert!(first_account.is_some());
        assert!(second_account.is_some());
        assert_ne!(first_account, second_account);
    }

    #[test]
    fn test_genesis_account() {
        let default_account = GenesisAccount::default();

        let nonce = Some(1);
        let balance = U256::from(33);
        let code = Some(b"code".into());
        let root = hex!("9474ddfcea39c5a690d2744103e39d1ff1b03d18db10fc147d970ad24699395a").into();
        let value = hex!("58eb8294d9bb16832a9dabfcb270fff99ab8ee1d8764e4f3d9fdf59ec1dee469").into();
        let mut map = BTreeMap::default();
        map.insert(root, value);
        let storage = Some(map);

        let genesis_account = GenesisAccount::default()
            .with_nonce(nonce)
            .with_balance(balance)
            .with_code(code.clone())
            .with_storage(storage.clone());

        assert_ne!(default_account, genesis_account);
        // check every field
        assert_eq!(genesis_account.nonce, nonce);
        assert_eq!(genesis_account.balance, balance);
        assert_eq!(genesis_account.code, code);
        assert_eq!(genesis_account.storage, storage);
    }

    #[test]
    fn parse_hive_genesis() {
        let geth_genesis = r#"
    {
        "difficulty": "0x20000",
        "gasLimit": "0x1",
        "alloc": {},
        "config": {
          "ethash": {},
          "chainId": 1
        }
    }
    "#;

        let _genesis: Genesis = serde_json::from_str(geth_genesis).unwrap();
    }

    #[test]
    fn parse_hive_clique_smoke_genesis() {
        let geth_genesis = r#"
    {
      "difficulty": "0x1",
      "gasLimit": "0x400000",
      "extraData":
    "0x0000000000000000000000000000000000000000000000000000000000000000658bdf435d810c91414ec09147daa6db624063790000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
    ,   "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
      "nonce": "0x0",
      "timestamp": "0x5c51a607",
      "alloc": {}
    }
    "#;

        let _genesis: Genesis = serde_json::from_str(geth_genesis).unwrap();
    }

    #[test]
    fn parse_non_hex_prefixed_balance() {
        // tests that we can parse balance / difficulty fields that are either hex or decimal
        let example_balance_json = r#"
    {
        "nonce": "0x0000000000000042",
        "difficulty": "34747478",
        "mixHash": "0x123456789abcdef123456789abcdef123456789abcdef123456789abcdef1234",
        "coinbase": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "timestamp": "0x123456",
        "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "extraData": "0xfafbfcfd",
        "gasLimit": "0x2fefd8",
        "alloc": {
            "0x3E951C9f69a06Bc3AD71fF7358DbC56bEd94b9F2": {
              "balance": "1000000000000000000000000000"
            },
            "0xe228C30d4e5245f967ac21726d5412dA27aD071C": {
              "balance": "1000000000000000000000000000"
            },
            "0xD59Ce7Ccc6454a2D2C2e06bbcf71D0Beb33480eD": {
              "balance": "1000000000000000000000000000"
            },
            "0x1CF4D54414eF51b41f9B2238c57102ab2e61D1F2": {
              "balance": "1000000000000000000000000000"
            },
            "0x249bE3fDEd872338C733cF3975af9736bdCb9D4D": {
              "balance": "1000000000000000000000000000"
            },
            "0x3fCd1bff94513712f8cD63d1eD66776A67D5F78e": {
              "balance": "1000000000000000000000000000"
            }
        },
        "config": {
            "ethash": {},
            "chainId": 10,
            "homesteadBlock": 0,
            "eip150Block": 0,
            "eip155Block": 0,
            "eip158Block": 0,
            "byzantiumBlock": 0,
            "constantinopleBlock": 0,
            "petersburgBlock": 0,
            "istanbulBlock": 0
        }
    }
    "#;

        let genesis: Genesis = serde_json::from_str(example_balance_json).unwrap();

        // check difficulty against hex ground truth
        let expected_difficulty = U256::from_str("0x2123456").unwrap();
        assert_eq!(expected_difficulty, genesis.difficulty);

        // check all alloc balances
        let dec_balance = U256::from_str("1000000000000000000000000000").unwrap();
        for alloc in &genesis.alloc {
            assert_eq!(alloc.1.balance, dec_balance);
        }
    }

    #[test]
    fn parse_hive_rpc_genesis() {
        let geth_genesis = r#"
    {
      "config": {
        "chainId": 7,
        "homesteadBlock": 0,
        "eip150Block": 0,
        "eip150Hash": "0x5de1ee4135274003348e80b788e5afa4b18b18d320a5622218d5c493fedf5689",
        "eip155Block": 0,
        "eip158Block": 0
      },
      "coinbase": "0x0000000000000000000000000000000000000000",
      "difficulty": "0x20000",
      "extraData":
    "0x0000000000000000000000000000000000000000000000000000000000000000658bdf435d810c91414ec09147daa6db624063790000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
    ,   "gasLimit": "0x2fefd8",
      "nonce": "0x0000000000000000",
      "timestamp": "0x1234",
      "alloc": {
        "cf49fda3be353c69b41ed96333cd24302da4556f": {
          "balance": "0x123450000000000000000"
        },
        "0161e041aad467a890839d5b08b138c1e6373072": {
          "balance": "0x123450000000000000000"
        },
        "87da6a8c6e9eff15d703fc2773e32f6af8dbe301": {
          "balance": "0x123450000000000000000"
        },
        "b97de4b8c857e4f6bc354f226dc3249aaee49209": {
          "balance": "0x123450000000000000000"
        },
        "c5065c9eeebe6df2c2284d046bfc906501846c51": {
          "balance": "0x123450000000000000000"
        },
        "0000000000000000000000000000000000000314": {
          "balance": "0x0",
          "code":
    "0x60606040526000357c0100000000000000000000000000000000000000000000000000000000900463ffffffff168063a223e05d1461006a578063abd1a0cf1461008d578063abfced1d146100d4578063e05c914a14610110578063e6768b451461014c575b610000565b346100005761007761019d565b6040518082815260200191505060405180910390f35b34610000576100be600480803573ffffffffffffffffffffffffffffffffffffffff169060200190919050506101a3565b6040518082815260200191505060405180910390f35b346100005761010e600480803573ffffffffffffffffffffffffffffffffffffffff169060200190919080359060200190919050506101ed565b005b346100005761014a600480803590602001909190803573ffffffffffffffffffffffffffffffffffffffff16906020019091905050610236565b005b346100005761017960048080359060200190919080359060200190919080359060200190919050506103c4565b60405180848152602001838152602001828152602001935050505060405180910390f35b60005481565b6000600160008373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020016000205490505b919050565b80600160008473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff168152602001908152602001600020819055505b5050565b7f6031a8d62d7c95988fa262657cd92107d90ed96e08d8f867d32f26edfe85502260405180905060405180910390a17f47e2689743f14e97f7dcfa5eec10ba1dff02f83b3d1d4b9c07b206cbbda66450826040518082815260200191505060405180910390a1817fa48a6b249a5084126c3da369fbc9b16827ead8cb5cdc094b717d3f1dcd995e2960405180905060405180910390a27f7890603b316f3509577afd111710f9ebeefa15e12f72347d9dffd0d65ae3bade81604051808273ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200191505060405180910390a18073ffffffffffffffffffffffffffffffffffffffff167f7efef9ea3f60ddc038e50cccec621f86a0195894dc0520482abf8b5c6b659e4160405180905060405180910390a28181604051808381526020018273ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019250505060405180910390a05b5050565b6000600060008585859250925092505b935093509390505600a165627a7a72305820aaf842d0d0c35c45622c5263cbb54813d2974d3999c8c38551d7c613ea2bc1170029"
    ,       "storage": {
            "0x0000000000000000000000000000000000000000000000000000000000000000": "0x1234",
            "0x6661e9d6d8b923d5bbaab1b96e1dd51ff6ea2a93520fdc9eb75d059238b8c5e9": "0x01"
          }
        },
        "0000000000000000000000000000000000000315": {
          "balance": "0x9999999999999999999999999999999",
          "code":
    "0x60606040526000357c0100000000000000000000000000000000000000000000000000000000900463ffffffff168063ef2769ca1461003e575b610000565b3461000057610078600480803573ffffffffffffffffffffffffffffffffffffffff1690602001909190803590602001909190505061007a565b005b8173ffffffffffffffffffffffffffffffffffffffff166108fc829081150290604051809050600060405180830381858888f1935050505015610106578173ffffffffffffffffffffffffffffffffffffffff167f30a3c50752f2552dcc2b93f5b96866280816a986c0c0408cb6778b9fa198288f826040518082815260200191505060405180910390a25b5b50505600a165627a7a72305820637991fabcc8abad4294bf2bb615db78fbec4edff1635a2647d3894e2daf6a610029"
        }
      }
    }
    "#;

        let _genesis: Genesis = serde_json::from_str(geth_genesis).unwrap();
    }

    #[test]
    fn parse_hive_graphql_genesis() {
        let geth_genesis = r#"
    {
        "config"     : {},
        "coinbase"   : "0x8888f1f195afa192cfee860698584c030f4c9db1",
        "difficulty" : "0x020000",
        "extraData"  : "0x42",
        "gasLimit"   : "0x2fefd8",
        "mixHash"    : "0x2c85bcbce56429100b2108254bb56906257582aeafcbd682bc9af67a9f5aee46",
        "nonce"      : "0x78cc16f7b4f65485",
        "parentHash" : "0x0000000000000000000000000000000000000000000000000000000000000000",
        "timestamp"  : "0x54c98c81",
        "alloc"      : {
            "a94f5374fce5edbc8e2a8697c15331677e6ebf0b": {
                "balance" : "0x09184e72a000"
            }
        }
    }
    "#;

        let _genesis: Genesis = serde_json::from_str(geth_genesis).unwrap();
    }

    #[test]
    fn parse_hive_engine_genesis() {
        let geth_genesis = r#"
    {
      "config": {
        "chainId": 7,
        "homesteadBlock": 0,
        "eip150Block": 0,
        "eip150Hash": "0x5de1ee4135274003348e80b788e5afa4b18b18d320a5622218d5c493fedf5689",
        "eip155Block": 0,
        "eip158Block": 0,
        "byzantiumBlock": 0,
        "constantinopleBlock": 0,
        "petersburgBlock": 0,
        "istanbulBlock": 0,
        "muirGlacierBlock": 0,
        "berlinBlock": 0,
        "yolov2Block": 0,
        "yolov3Block": 0,
        "londonBlock": 0
      },
      "coinbase": "0x0000000000000000000000000000000000000000",
      "difficulty": "0x30000",
      "extraData":
    "0x0000000000000000000000000000000000000000000000000000000000000000658bdf435d810c91414ec09147daa6db624063790000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
    ,   "gasLimit": "0x2fefd8",
      "nonce": "0x0000000000000000",
      "timestamp": "0x1234",
      "alloc": {
        "cf49fda3be353c69b41ed96333cd24302da4556f": {
          "balance": "0x123450000000000000000"
        },
        "0161e041aad467a890839d5b08b138c1e6373072": {
          "balance": "0x123450000000000000000"
        },
        "87da6a8c6e9eff15d703fc2773e32f6af8dbe301": {
          "balance": "0x123450000000000000000"
        },
        "b97de4b8c857e4f6bc354f226dc3249aaee49209": {
          "balance": "0x123450000000000000000"
        },
        "c5065c9eeebe6df2c2284d046bfc906501846c51": {
          "balance": "0x123450000000000000000"
        },
        "0000000000000000000000000000000000000314": {
          "balance": "0x0",
          "code":
    "0x60606040526000357c0100000000000000000000000000000000000000000000000000000000900463ffffffff168063a223e05d1461006a578063abd1a0cf1461008d578063abfced1d146100d4578063e05c914a14610110578063e6768b451461014c575b610000565b346100005761007761019d565b6040518082815260200191505060405180910390f35b34610000576100be600480803573ffffffffffffffffffffffffffffffffffffffff169060200190919050506101a3565b6040518082815260200191505060405180910390f35b346100005761010e600480803573ffffffffffffffffffffffffffffffffffffffff169060200190919080359060200190919050506101ed565b005b346100005761014a600480803590602001909190803573ffffffffffffffffffffffffffffffffffffffff16906020019091905050610236565b005b346100005761017960048080359060200190919080359060200190919080359060200190919050506103c4565b60405180848152602001838152602001828152602001935050505060405180910390f35b60005481565b6000600160008373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020016000205490505b919050565b80600160008473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff168152602001908152602001600020819055505b5050565b7f6031a8d62d7c95988fa262657cd92107d90ed96e08d8f867d32f26edfe85502260405180905060405180910390a17f47e2689743f14e97f7dcfa5eec10ba1dff02f83b3d1d4b9c07b206cbbda66450826040518082815260200191505060405180910390a1817fa48a6b249a5084126c3da369fbc9b16827ead8cb5cdc094b717d3f1dcd995e2960405180905060405180910390a27f7890603b316f3509577afd111710f9ebeefa15e12f72347d9dffd0d65ae3bade81604051808273ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200191505060405180910390a18073ffffffffffffffffffffffffffffffffffffffff167f7efef9ea3f60ddc038e50cccec621f86a0195894dc0520482abf8b5c6b659e4160405180905060405180910390a28181604051808381526020018273ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019250505060405180910390a05b5050565b6000600060008585859250925092505b935093509390505600a165627a7a72305820aaf842d0d0c35c45622c5263cbb54813d2974d3999c8c38551d7c613ea2bc1170029"
    ,       "storage": {
            "0x0000000000000000000000000000000000000000000000000000000000000000": "0x1234",
            "0x6661e9d6d8b923d5bbaab1b96e1dd51ff6ea2a93520fdc9eb75d059238b8c5e9": "0x01"
          }
        },
        "0000000000000000000000000000000000000315": {
          "balance": "0x9999999999999999999999999999999",
          "code":
    "0x60606040526000357c0100000000000000000000000000000000000000000000000000000000900463ffffffff168063ef2769ca1461003e575b610000565b3461000057610078600480803573ffffffffffffffffffffffffffffffffffffffff1690602001909190803590602001909190505061007a565b005b8173ffffffffffffffffffffffffffffffffffffffff166108fc829081150290604051809050600060405180830381858888f1935050505015610106578173ffffffffffffffffffffffffffffffffffffffff167f30a3c50752f2552dcc2b93f5b96866280816a986c0c0408cb6778b9fa198288f826040518082815260200191505060405180910390a25b5b50505600a165627a7a72305820637991fabcc8abad4294bf2bb615db78fbec4edff1635a2647d3894e2daf6a610029"
        },
        "0000000000000000000000000000000000000316": {
          "balance": "0x0",
          "code": "0x444355"
        },
        "0000000000000000000000000000000000000317": {
          "balance": "0x0",
          "code": "0x600160003555"
        }
      }
    }
    "#;

        let _genesis: Genesis = serde_json::from_str(geth_genesis).unwrap();
    }

    #[test]
    fn parse_hive_devp2p_genesis() {
        let geth_genesis = r#"
    {
        "config": {
            "chainId": 19763,
            "homesteadBlock": 0,
            "eip150Block": 0,
            "eip155Block": 0,
            "eip158Block": 0,
            "byzantiumBlock": 0,
            "ethash": {}
        },
        "nonce": "0xdeadbeefdeadbeef",
        "timestamp": "0x0",
        "extraData": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "gasLimit": "0x80000000",
        "difficulty": "0x20000",
        "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "coinbase": "0x0000000000000000000000000000000000000000",
        "alloc": {
            "71562b71999873db5b286df957af199ec94617f7": {
                "balance": "0xffffffffffffffffffffffffff"
            }
        },
        "number": "0x0",
        "gasUsed": "0x0",
        "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000"
    }
    "#;

        let _genesis: Genesis = serde_json::from_str(geth_genesis).unwrap();
    }

    #[test]
    fn parse_deposit_contract_address() {
        let genesis = r#"
    {
      "config": {
        "chainId": 1337,
        "homesteadBlock": 0,
        "eip150Block": 0,
        "eip155Block": 0,
        "eip158Block": 0,
        "byzantiumBlock": 0,
        "constantinopleBlock": 0,
        "petersburgBlock": 0,
        "istanbulBlock": 0,
        "muirGlacierBlock": 0,
        "berlinBlock": 0,
        "londonBlock": 0,
        "arrowGlacierBlock": 0,
        "grayGlacierBlock": 0,
        "shanghaiTime": 0,
        "cancunTime": 0,
        "pragueTime": 1,
        "osakaTime": 2,
        "terminalTotalDifficulty": 0,
        "depositContractAddress": "0x0000000000000000000000000000000000000000",
        "terminalTotalDifficultyPassed": true
      },
      "nonce": "0x0",
      "timestamp": "0x0",
      "extraData": "0x",
      "gasLimit": "0x4c4b40",
      "difficulty": "0x1",
      "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
      "coinbase": "0x0000000000000000000000000000000000000000"
    }
    "#;

        let got_genesis: Genesis = serde_json::from_str(genesis).unwrap();
        let expected_genesis = Genesis {
            config: ChainConfig {
                chain_id: 1337,
                homestead_block: Some(0),
                eip150_block: Some(0),
                eip155_block: Some(0),
                eip158_block: Some(0),
                byzantium_block: Some(0),
                constantinople_block: Some(0),
                petersburg_block: Some(0),
                istanbul_block: Some(0),
                muir_glacier_block: Some(0),
                berlin_block: Some(0),
                london_block: Some(0),
                arrow_glacier_block: Some(0),
                gray_glacier_block: Some(0),
                dao_fork_block: None,
                dao_fork_support: false,
                shanghai_time: Some(0),
                cancun_time: Some(0),
                prague_time: Some(1),
                osaka_time: Some(2),
                terminal_total_difficulty: Some(U256::ZERO),
                terminal_total_difficulty_passed: true,
                deposit_contract_address: Some(Address::ZERO),
                ..Default::default()
            },
            nonce: 0,
            timestamp: 0,
            extra_data: Bytes::new(),
            gas_limit: 0x4c4b40,
            difficulty: U256::from(1),
            ..Default::default()
        };

        assert_eq!(expected_genesis, got_genesis);
    }

    #[test]
    fn parse_prague_time() {
        let genesis = r#"
    {
      "config": {
        "chainId": 1337,
        "homesteadBlock": 0,
        "eip150Block": 0,
        "eip155Block": 0,
        "eip158Block": 0,
        "byzantiumBlock": 0,
        "constantinopleBlock": 0,
        "petersburgBlock": 0,
        "istanbulBlock": 0,
        "muirGlacierBlock": 0,
        "berlinBlock": 0,
        "londonBlock": 0,
        "arrowGlacierBlock": 0,
        "grayGlacierBlock": 0,
        "shanghaiTime": 0,
        "cancunTime": 0,
        "pragueTime": 1,
        "terminalTotalDifficulty": 0,
        "terminalTotalDifficultyPassed": true
      },
      "nonce": "0x0",
      "timestamp": "0x0",
      "extraData": "0x",
      "gasLimit": "0x4c4b40",
      "difficulty": "0x1",
      "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
      "coinbase": "0x0000000000000000000000000000000000000000"
    }
    "#;

        let got_genesis: Genesis = serde_json::from_str(genesis).unwrap();
        let expected_genesis = Genesis {
            config: ChainConfig {
                chain_id: 1337,
                homestead_block: Some(0),
                eip150_block: Some(0),
                eip155_block: Some(0),
                eip158_block: Some(0),
                byzantium_block: Some(0),
                constantinople_block: Some(0),
                petersburg_block: Some(0),
                istanbul_block: Some(0),
                muir_glacier_block: Some(0),
                berlin_block: Some(0),
                london_block: Some(0),
                arrow_glacier_block: Some(0),
                gray_glacier_block: Some(0),
                dao_fork_block: None,
                dao_fork_support: false,
                shanghai_time: Some(0),
                cancun_time: Some(0),
                prague_time: Some(1),
                terminal_total_difficulty: Some(U256::ZERO),
                terminal_total_difficulty_passed: true,
                ..Default::default()
            },
            nonce: 0,
            timestamp: 0,
            extra_data: Bytes::new(),
            gas_limit: 0x4c4b40,
            difficulty: U256::from(1),
            ..Default::default()
        };

        assert_eq!(expected_genesis, got_genesis);
    }

    #[test]
    fn parse_execution_apis_genesis() {
        let geth_genesis = r#"
    {
      "config": {
        "chainId": 1337,
        "homesteadBlock": 0,
        "eip150Block": 0,
        "eip150Hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "eip155Block": 0,
        "eip158Block": 0,
        "byzantiumBlock": 0,
        "constantinopleBlock": 0,
        "petersburgBlock": 0,
        "istanbulBlock": 0,
        "muirGlacierBlock": 0,
        "berlinBlock": 0,
        "londonBlock": 0,
        "arrowGlacierBlock": 0,
        "grayGlacierBlock": 0,
        "shanghaiTime": 0,
        "terminalTotalDifficulty": 0,
        "terminalTotalDifficultyPassed": true,
        "ethash": {}
      },
      "nonce": "0x0",
      "timestamp": "0x0",
      "extraData": "0x",
      "gasLimit": "0x4c4b40",
      "difficulty": "0x1",
      "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
      "coinbase": "0x0000000000000000000000000000000000000000",
      "alloc": {
        "658bdf435d810c91414ec09147daa6db62406379": {
          "balance": "0x487a9a304539440000"
        },
        "aa00000000000000000000000000000000000000": {
          "code": "0x6042",
          "storage": {
            "0x0000000000000000000000000000000000000000000000000000000000000000":
    "0x0000000000000000000000000000000000000000000000000000000000000000",
            "0x0100000000000000000000000000000000000000000000000000000000000000":
    "0x0100000000000000000000000000000000000000000000000000000000000000",
            "0x0200000000000000000000000000000000000000000000000000000000000000":
    "0x0200000000000000000000000000000000000000000000000000000000000000",
            "0x0300000000000000000000000000000000000000000000000000000000000000":
    "0x0000000000000000000000000000000000000000000000000000000000000303"       },
          "balance": "0x1",
          "nonce": "0x1"
        },
        "bb00000000000000000000000000000000000000": {
          "code": "0x600154600354",
          "storage": {
            "0x0000000000000000000000000000000000000000000000000000000000000000":
    "0x0000000000000000000000000000000000000000000000000000000000000000",
            "0x0100000000000000000000000000000000000000000000000000000000000000":
    "0x0100000000000000000000000000000000000000000000000000000000000000",
            "0x0200000000000000000000000000000000000000000000000000000000000000":
    "0x0200000000000000000000000000000000000000000000000000000000000000",
            "0x0300000000000000000000000000000000000000000000000000000000000000":
    "0x0000000000000000000000000000000000000000000000000000000000000303"       },
          "balance": "0x2",
          "nonce": "0x1"
        }
      }
    }
    "#;

        let _genesis: Genesis = serde_json::from_str(geth_genesis).unwrap();
    }

    #[test]
    fn parse_hive_rpc_genesis_full() {
        let geth_genesis = r#"
    {
      "config": {
        "clique": {
          "period": 1
        },
        "chainId": 7,
        "homesteadBlock": 0,
        "eip150Block": 0,
        "eip155Block": 0,
        "eip158Block": 0
      },
      "coinbase": "0x0000000000000000000000000000000000000000",
      "difficulty": "0x020000",
      "extraData":
    "0x0000000000000000000000000000000000000000000000000000000000000000658bdf435d810c91414ec09147daa6db624063790000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
    ,   "gasLimit": "0x2fefd8",
      "nonce": "0x0000000000000000",
      "timestamp": "0x1234",
      "alloc": {
        "cf49fda3be353c69b41ed96333cd24302da4556f": {
          "balance": "0x123450000000000000000"
        },
        "0161e041aad467a890839d5b08b138c1e6373072": {
          "balance": "0x123450000000000000000"
        },
        "87da6a8c6e9eff15d703fc2773e32f6af8dbe301": {
          "balance": "0x123450000000000000000"
        },
        "b97de4b8c857e4f6bc354f226dc3249aaee49209": {
          "balance": "0x123450000000000000000"
        },
        "c5065c9eeebe6df2c2284d046bfc906501846c51": {
          "balance": "0x123450000000000000000"
        },
        "0000000000000000000000000000000000000314": {
          "balance": "0x0",
          "code":
    "0x60606040526000357c0100000000000000000000000000000000000000000000000000000000900463ffffffff168063a223e05d1461006a578063abd1a0cf1461008d578063abfced1d146100d4578063e05c914a14610110578063e6768b451461014c575b610000565b346100005761007761019d565b6040518082815260200191505060405180910390f35b34610000576100be600480803573ffffffffffffffffffffffffffffffffffffffff169060200190919050506101a3565b6040518082815260200191505060405180910390f35b346100005761010e600480803573ffffffffffffffffffffffffffffffffffffffff169060200190919080359060200190919050506101ed565b005b346100005761014a600480803590602001909190803573ffffffffffffffffffffffffffffffffffffffff16906020019091905050610236565b005b346100005761017960048080359060200190919080359060200190919080359060200190919050506103c4565b60405180848152602001838152602001828152602001935050505060405180910390f35b60005481565b6000600160008373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020016000205490505b919050565b80600160008473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff168152602001908152602001600020819055505b5050565b7f6031a8d62d7c95988fa262657cd92107d90ed96e08d8f867d32f26edfe85502260405180905060405180910390a17f47e2689743f14e97f7dcfa5eec10ba1dff02f83b3d1d4b9c07b206cbbda66450826040518082815260200191505060405180910390a1817fa48a6b249a5084126c3da369fbc9b16827ead8cb5cdc094b717d3f1dcd995e2960405180905060405180910390a27f7890603b316f3509577afd111710f9ebeefa15e12f72347d9dffd0d65ae3bade81604051808273ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200191505060405180910390a18073ffffffffffffffffffffffffffffffffffffffff167f7efef9ea3f60ddc038e50cccec621f86a0195894dc0520482abf8b5c6b659e4160405180905060405180910390a28181604051808381526020018273ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019250505060405180910390a05b5050565b6000600060008585859250925092505b935093509390505600a165627a7a72305820aaf842d0d0c35c45622c5263cbb54813d2974d3999c8c38551d7c613ea2bc1170029"
    ,       "storage": {
            "0x0000000000000000000000000000000000000000000000000000000000000000": "0x1234",
            "0x6661e9d6d8b923d5bbaab1b96e1dd51ff6ea2a93520fdc9eb75d059238b8c5e9": "0x01"
          }
        },
        "0000000000000000000000000000000000000315": {
          "balance": "0x9999999999999999999999999999999",
          "code":
    "0x60606040526000357c0100000000000000000000000000000000000000000000000000000000900463ffffffff168063ef2769ca1461003e575b610000565b3461000057610078600480803573ffffffffffffffffffffffffffffffffffffffff1690602001909190803590602001909190505061007a565b005b8173ffffffffffffffffffffffffffffffffffffffff166108fc829081150290604051809050600060405180830381858888f1935050505015610106578173ffffffffffffffffffffffffffffffffffffffff167f30a3c50752f2552dcc2b93f5b96866280816a986c0c0408cb6778b9fa198288f826040518082815260200191505060405180910390a25b5b50505600a165627a7a72305820637991fabcc8abad4294bf2bb615db78fbec4edff1635a2647d3894e2daf6a610029"
        }
      },
      "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
      "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000"
    }
    "#;

        let genesis: Genesis = serde_json::from_str(geth_genesis).unwrap();
        let alloc_entry = genesis
            .alloc
            .get(&Address::from_str("0000000000000000000000000000000000000314").unwrap())
            .expect("missing account for parsed genesis");
        let storage = alloc_entry.storage.as_ref().expect("missing storage for parsed genesis");
        let expected_storage = BTreeMap::from_iter(vec![
            (
                B256::from_str(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                )
                .unwrap(),
                B256::from_str(
                    "0x0000000000000000000000000000000000000000000000000000000000001234",
                )
                .unwrap(),
            ),
            (
                B256::from_str(
                    "0x6661e9d6d8b923d5bbaab1b96e1dd51ff6ea2a93520fdc9eb75d059238b8c5e9",
                )
                .unwrap(),
                B256::from_str(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                )
                .unwrap(),
            ),
        ]);
        assert_eq!(storage, &expected_storage);

        let expected_code =
    Bytes::from_str("0x60606040526000357c0100000000000000000000000000000000000000000000000000000000900463ffffffff168063a223e05d1461006a578063abd1a0cf1461008d578063abfced1d146100d4578063e05c914a14610110578063e6768b451461014c575b610000565b346100005761007761019d565b6040518082815260200191505060405180910390f35b34610000576100be600480803573ffffffffffffffffffffffffffffffffffffffff169060200190919050506101a3565b6040518082815260200191505060405180910390f35b346100005761010e600480803573ffffffffffffffffffffffffffffffffffffffff169060200190919080359060200190919050506101ed565b005b346100005761014a600480803590602001909190803573ffffffffffffffffffffffffffffffffffffffff16906020019091905050610236565b005b346100005761017960048080359060200190919080359060200190919080359060200190919050506103c4565b60405180848152602001838152602001828152602001935050505060405180910390f35b60005481565b6000600160008373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020016000205490505b919050565b80600160008473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff168152602001908152602001600020819055505b5050565b7f6031a8d62d7c95988fa262657cd92107d90ed96e08d8f867d32f26edfe85502260405180905060405180910390a17f47e2689743f14e97f7dcfa5eec10ba1dff02f83b3d1d4b9c07b206cbbda66450826040518082815260200191505060405180910390a1817fa48a6b249a5084126c3da369fbc9b16827ead8cb5cdc094b717d3f1dcd995e2960405180905060405180910390a27f7890603b316f3509577afd111710f9ebeefa15e12f72347d9dffd0d65ae3bade81604051808273ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16815260200191505060405180910390a18073ffffffffffffffffffffffffffffffffffffffff167f7efef9ea3f60ddc038e50cccec621f86a0195894dc0520482abf8b5c6b659e4160405180905060405180910390a28181604051808381526020018273ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019250505060405180910390a05b5050565b6000600060008585859250925092505b935093509390505600a165627a7a72305820aaf842d0d0c35c45622c5263cbb54813d2974d3999c8c38551d7c613ea2bc1170029"
    ).unwrap();
        let code = alloc_entry.code.as_ref().expect(
            "missing code for parsed
    genesis",
        );
        assert_eq!(code, &expected_code);
    }

    #[test]
    fn test_hive_smoke_alloc_deserialize() {
        let hive_genesis = r#"
    {
        "nonce": "0x0000000000000042",
        "difficulty": "0x2123456",
        "mixHash": "0x123456789abcdef123456789abcdef123456789abcdef123456789abcdef1234",
        "coinbase": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "timestamp": "0x123456",
        "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "extraData": "0xfafbfcfd",
        "gasLimit": "0x2fefd8",
        "alloc": {
            "dbdbdb2cbd23b783741e8d7fcf51e459b497e4a6": {
                "balance": "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
            },
            "e6716f9544a56c530d868e4bfbacb172315bdead": {
                "balance": "0x11",
                "code": "0x12"
            },
            "b9c015918bdaba24b4ff057a92a3873d6eb201be": {
                "balance": "0x21",
                "storage": {
                    "0x0000000000000000000000000000000000000000000000000000000000000001": "0x22"
                }
            },
            "1a26338f0d905e295fccb71fa9ea849ffa12aaf4": {
                "balance": "0x31",
                "nonce": "0x32"
            },
            "0000000000000000000000000000000000000001": {
                "balance": "0x41"
            },
            "0000000000000000000000000000000000000002": {
                "balance": "0x51"
            },
            "0000000000000000000000000000000000000003": {
                "balance": "0x61"
            },
            "0000000000000000000000000000000000000004": {
                "balance": "0x71"
            }
        },
        "config": {
            "ethash": {},
            "chainId": 10,
            "homesteadBlock": 0,
            "eip150Block": 0,
            "eip155Block": 0,
            "eip158Block": 0,
            "byzantiumBlock": 0,
            "constantinopleBlock": 0,
            "petersburgBlock": 0,
            "istanbulBlock": 0
        }
    }
    "#;

        let expected_genesis =
            Genesis {
                nonce: 0x0000000000000042,
                difficulty: U256::from(0x2123456),
                mix_hash: B256::from_str(
                    "0x123456789abcdef123456789abcdef123456789abcdef123456789abcdef1234",
                )
                .unwrap(),
                coinbase: Address::from_str("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
                timestamp: 0x123456,
                extra_data: Bytes::from_str("0xfafbfcfd").unwrap(),
                gas_limit: 0x2fefd8,
                base_fee_per_gas: None,
                excess_blob_gas: None,
                blob_gas_used: None,
                number: None,
                alloc: BTreeMap::from_iter(vec![
                (
                    Address::from_str("0xdbdbdb2cbd23b783741e8d7fcf51e459b497e4a6").unwrap(),
                    GenesisAccount {
                        balance:
    U256::from_str("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff").
    unwrap(),                     nonce: None,
                        code: None,
                        storage: None,
                        private_key: None,
                    },
                ),
                (
                    Address::from_str("0xe6716f9544a56c530d868e4bfbacb172315bdead").unwrap(),
                    GenesisAccount {
                        balance: U256::from_str("0x11").unwrap(),
                        nonce: None,
                        code: Some(Bytes::from_str("0x12").unwrap()),
                        storage: None,
                        private_key: None,
                    },
                ),
                (
                    Address::from_str("0xb9c015918bdaba24b4ff057a92a3873d6eb201be").unwrap(),
                    GenesisAccount {
                        balance: U256::from_str("0x21").unwrap(),
                        nonce: None,
                        code: None,
                        storage: Some(BTreeMap::from_iter(vec![
                            (

    B256::from_str("0x0000000000000000000000000000000000000000000000000000000000000001").
    unwrap(),
    B256::from_str("0x0000000000000000000000000000000000000000000000000000000000000022").
    unwrap(),                         ),
                        ])),
                        private_key: None,
                    },
                ),
                (
                    Address::from_str("0x1a26338f0d905e295fccb71fa9ea849ffa12aaf4").unwrap(),
                    GenesisAccount {
                        balance: U256::from_str("0x31").unwrap(),
                        nonce: Some(0x32u64),
                        code: None,
                        storage: None,
                        private_key: None,
                    },
                ),
                (
                    Address::from_str("0x0000000000000000000000000000000000000001").unwrap(),
                    GenesisAccount {
                        balance: U256::from_str("0x41").unwrap(),
                        nonce: None,
                        code: None,
                        storage: None,
                        private_key: None,
                    },
                ),
                (
                    Address::from_str("0x0000000000000000000000000000000000000002").unwrap(),
                    GenesisAccount {
                        balance: U256::from_str("0x51").unwrap(),
                        nonce: None,
                        code: None,
                        storage: None,
                        private_key: None,
                    },
                ),
                (
                    Address::from_str("0x0000000000000000000000000000000000000003").unwrap(),
                    GenesisAccount {
                        balance: U256::from_str("0x61").unwrap(),
                        nonce: None,
                        code: None,
                        storage: None,
                        private_key: None,
                    },
                ),
                (
                    Address::from_str("0x0000000000000000000000000000000000000004").unwrap(),
                    GenesisAccount {
                        balance: U256::from_str("0x71").unwrap(),
                        nonce: None,
                        code: None,
                        storage: None,
                        private_key: None,
                    },
                ),
            ]),
                config: ChainConfig {
                    ethash: Some(EthashConfig {}),
                    chain_id: 10,
                    homestead_block: Some(0),
                    eip150_block: Some(0),
                    eip155_block: Some(0),
                    eip158_block: Some(0),
                    byzantium_block: Some(0),
                    constantinople_block: Some(0),
                    petersburg_block: Some(0),
                    istanbul_block: Some(0),
                    deposit_contract_address: None,
                    ..Default::default()
                },
            };

        let deserialized_genesis: Genesis = serde_json::from_str(hive_genesis).unwrap();
        assert_eq!(
            deserialized_genesis, expected_genesis,
            "deserialized genesis
    {deserialized_genesis:#?} does not match expected {expected_genesis:#?}"
        );
    }

    #[test]
    fn parse_dump_genesis_mainnet() {
        let mainnet = include_str!("../dumpgenesis/mainnet.json");
        let gen = serde_json::from_str::<Genesis>(mainnet).unwrap();
        let s = serde_json::to_string_pretty(&gen).unwrap();
        let gen2 = serde_json::from_str::<Genesis>(&s).unwrap();
        assert_eq!(gen, gen2);
    }

    #[test]
    fn parse_dump_genesis_sepolia() {
        let sepolia = include_str!("../dumpgenesis/sepolia.json");
        let gen = serde_json::from_str::<Genesis>(sepolia).unwrap();
        let s = serde_json::to_string_pretty(&gen).unwrap();
        let gen2 = serde_json::from_str::<Genesis>(&s).unwrap();
        assert_eq!(gen, gen2);
    }

    #[test]
    fn parse_dump_genesis_holesky() {
        let holesky = include_str!("../dumpgenesis/holesky.json");
        let gen = serde_json::from_str::<Genesis>(holesky).unwrap();
        let s = serde_json::to_string_pretty(&gen).unwrap();
        let gen2 = serde_json::from_str::<Genesis>(&s).unwrap();
        assert_eq!(gen, gen2);
    }

    #[test]
    fn parse_extra_fields() {
        let geth_genesis = r#"
    {
        "difficulty": "0x20000",
        "gasLimit": "0x1",
        "alloc": {},
        "config": {
          "ethash": {},
          "chainId": 1,
          "string_field": "string_value",
          "numeric_field": 7,
          "object_field": {
            "sub_field": "sub_value"
          }
        }
    }
    "#;
        let genesis: Genesis = serde_json::from_str(geth_genesis).unwrap();
        let actual_string_value = genesis.config.extra_fields.get("string_field").unwrap();
        assert_eq!(actual_string_value, "string_value");
        let actual_numeric_value = genesis.config.extra_fields.get("numeric_field").unwrap();
        assert_eq!(actual_numeric_value, 7);
        let actual_object_value = genesis.config.extra_fields.get("object_field").unwrap();
        assert_eq!(actual_object_value, &serde_json::json!({"sub_field": "sub_value"}));
    }

    #[test]
    fn deserialize_private_key_as_none_when_0x() {
        // Test case where "secretKey" is "0x", expecting None
        let json_data = json!({
            "balance": "0x0",
            "secretKey": "0x"
        });

        let account: GenesisAccount = serde_json::from_value(json_data).unwrap();
        assert_eq!(account.private_key, None);
    }

    #[test]
    fn deserialize_private_key_with_valid_hex() {
        // Test case where "secretKey" is a valid hex string
        let json_data = json!({
            "balance": "0x0",
            "secretKey": "0x123456789abcdef123456789abcdef123456789abcdef123456789abcdef1234"
        });

        let account: GenesisAccount = serde_json::from_value(json_data).unwrap();
        let expected_key =
            B256::from_str("123456789abcdef123456789abcdef123456789abcdef123456789abcdef1234")
                .unwrap();
        assert_eq!(account.private_key, Some(expected_key));
    }

    #[test]
    fn deserialize_private_key_as_none_when_null() {
        // Test case where "secretKey" is null, expecting None
        let json_data = json!({
            "balance": "0x0",
            "secretKey": null
        });

        let account: GenesisAccount = serde_json::from_value(json_data).unwrap();
        assert_eq!(account.private_key, None);
    }

    #[test]
    fn deserialize_private_key_with_invalid_hex_fails() {
        // Test case where "secretKey" is an invalid hex string, expecting an error
        let json_data = json!({
            "balance": "0x0",
            "secretKey": "0xINVALIDHEX"
        });

        let result: Result<GenesisAccount, _> = serde_json::from_value(json_data);
        assert!(result.is_err()); // The deserialization should fail due to invalid hex
    }

    #[test]
    fn deserialize_private_key_with_empty_string_fails() {
        // Test case where "secretKey" is an empty string, expecting an error
        let json_data = json!({
            "secretKey": ""
        });

        let result: Result<GenesisAccount, _> = serde_json::from_value(json_data);
        assert!(result.is_err()); // The deserialization should fail due to an empty string
    }

    #[test]
    fn test_from_genesis_account_with_default_values() {
        let genesis_account = GenesisAccount::default();

        // Convert the GenesisAccount to a TrieAccount
        let trie_account: TrieAccount = genesis_account.into();

        // Check the fields are properly set.
        assert_eq!(trie_account.nonce, 0);
        assert_eq!(trie_account.balance, U256::default());
        assert_eq!(trie_account.storage_root, EMPTY_ROOT_HASH);
        assert_eq!(trie_account.code_hash, KECCAK_EMPTY);

        // Check that the default Account converts to the same TrieAccount
        assert_eq!(TrieAccount::default(), trie_account);
    }

    #[test]
    fn test_from_genesis_account_with_values() {
        // Create a GenesisAccount with specific values
        let mut storage = BTreeMap::new();
        storage.insert(B256::from([0x01; 32]), B256::from([0x02; 32]));

        let genesis_account = GenesisAccount {
            nonce: Some(10),
            balance: U256::from(1000),
            code: Some(Bytes::from(vec![0x60, 0x61])),
            storage: Some(storage),
            private_key: None,
        };

        // Convert the GenesisAccount to a TrieAccount
        let trie_account: TrieAccount = genesis_account.into();

        let expected_storage_root = storage_root_unhashed(BTreeMap::from([(
            B256::from([0x01; 32]),
            U256::from_be_bytes(*B256::from([0x02; 32])),
        )]));

        // Check that the fields are properly set.
        assert_eq!(trie_account.nonce, 10);
        assert_eq!(trie_account.balance, U256::from(1000));
        assert_eq!(trie_account.storage_root, expected_storage_root);
        assert_eq!(trie_account.code_hash, keccak256([0x60, 0x61]));
    }

    #[test]
    fn test_from_genesis_account_with_zeroed_storage_values() {
        // Create a GenesisAccount with storage containing zero values
        let storage = BTreeMap::from([(B256::from([0x01; 32]), B256::from([0x00; 32]))]);

        let genesis_account = GenesisAccount {
            nonce: Some(3),
            balance: U256::from(300),
            code: None,
            storage: Some(storage),
            private_key: None,
        };

        // Convert the GenesisAccount to a TrieAccount
        let trie_account: TrieAccount = genesis_account.into();

        // Check the fields are properly set.
        assert_eq!(trie_account.nonce, 3);
        assert_eq!(trie_account.balance, U256::from(300));
        // Zero values in storage should result in EMPTY_ROOT_HASH
        assert_eq!(trie_account.storage_root, EMPTY_ROOT_HASH);
        // No code provided, so code hash should be KECCAK_EMPTY
        assert_eq!(trie_account.code_hash, KECCAK_EMPTY);
    }
}
