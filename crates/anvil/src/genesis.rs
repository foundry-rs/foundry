//! Bindings for geth's `genesis.json` format
use crate::revm::primitives::AccountInfo;
use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_signer::LocalWallet;
use foundry_common::errors::FsPathError;
use foundry_evm::revm::primitives::{Bytecode, Env, KECCAK_EMPTY, U256 as rU256};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

/// Genesis specifies the header fields, state of a genesis block. It also defines hard fork
/// switch-over blocks through the chain configuration See also: <https://github.com/ethereum/go-ethereum/blob/0ce494b60cd00d70f1f9f2dd0b9bfbd76204168a/core/genesis.go#L47-L66>
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Genesis {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Config>,
    #[serde(
        default,
        deserialize_with = "anvil_core::eth::serde_helpers::numeric::deserialize_stringified_u64_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub nonce: Option<u64>,
    #[serde(
        default,
        deserialize_with = "anvil_core::eth::serde_helpers::numeric::deserialize_stringified_u64_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub timestamp: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_data: Option<Bytes>,
    #[serde(
        deserialize_with = "anvil_core::eth::serde_helpers::numeric::deserialize_stringified_u64"
    )]
    pub gas_limit: u64,
    #[serde(
        deserialize_with = "anvil_core::eth::serde_helpers::numeric::deserialize_stringified_u64"
    )]
    pub difficulty: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mix_hash: Option<B256>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coinbase: Option<Address>,
    #[serde(default)]
    pub alloc: Alloc,
    #[serde(
        default,
        deserialize_with = "anvil_core::eth::serde_helpers::numeric::deserialize_stringified_u64_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub number: Option<u64>,
    #[serde(
        default,
        deserialize_with = "anvil_core::eth::serde_helpers::numeric::deserialize_stringified_u64_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub gas_used: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_hash: Option<B256>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_fee_per_gas: Option<U256>,
}

impl Genesis {
    /// Loads the `Genesis` object from the given json file path
    pub fn load(path: impl AsRef<Path>) -> Result<Self, FsPathError> {
        foundry_common::fs::read_json_file(path.as_ref())
    }

    /// The clap `value_parser` function
    pub(crate) fn parse(path: &str) -> Result<Self, String> {
        Self::load(path).map_err(|err| err.to_string())
    }

    pub fn chain_id(&self) -> Option<u64> {
        self.config.as_ref().and_then(|c| c.chain_id)
    }

    /// Applies all settings to the given `env`
    pub fn apply(&self, env: &mut Env) {
        if let Some(chain_id) = self.chain_id() {
            env.cfg.chain_id = chain_id;
        }
        if let Some(timestamp) = self.timestamp {
            env.block.timestamp = U256::from(timestamp);
        }
        if let Some(base_fee) = self.base_fee_per_gas {
            env.block.basefee = base_fee;
        }
        if let Some(number) = self.number {
            env.block.number = rU256::from(number);
        }
        if let Some(coinbase) = self.coinbase {
            env.block.coinbase = coinbase;
        }
        env.block.difficulty = U256::from(self.difficulty);
        env.block.gas_limit = U256::from(self.gas_limit);
    }

    /// Returns all private keys from the genesis accounts, if they exist
    pub fn private_keys(&self) -> Vec<LocalWallet> {
        self.alloc.accounts.values().filter_map(|acc| acc.private_key.clone()).collect()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Alloc {
    pub accounts: BTreeMap<Address, GenesisAccount>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenesisAccount {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<Bytes>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub storage: HashMap<B256, B256>,
    pub balance: U256,
    #[serde(
        default,
        deserialize_with = "anvil_core::eth::serde_helpers::numeric::deserialize_stringified_u64_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub nonce: Option<u64>,
    #[serde(
        rename = "secretKey",
        default,
        skip_serializing_if = "Option::is_none",
        with = "secret_key"
    )]
    pub private_key: Option<LocalWallet>,
}

impl From<GenesisAccount> for AccountInfo {
    fn from(acc: GenesisAccount) -> Self {
        let GenesisAccount { code, balance, nonce, .. } = acc;
        let code = code.map(|code| Bytecode::new_raw(code.to_vec().into()));
        AccountInfo {
            balance,
            nonce: nonce.unwrap_or_default(),
            code_hash: code.as_ref().map(|code| code.hash_slow()).unwrap_or(KECCAK_EMPTY),
            code,
        }
    }
}

/// ChainConfig is the core config which determines the blockchain settings.
///
/// ChainConfig is stored in the database on a per block basis. This means
/// that any network, identified by its genesis block, can have its own
/// set of configuration options.
/// <(https://github.com/ethereum/go-ethereum/blob/0ce494b60cd00d70f1f9f2dd0b9bfbd76204168a/params/config.go#L342-L387>
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homestead_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dao_fork_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dao_fork_support: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eip150_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eip150_hash: Option<B256>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eip155_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eip158_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byzantium_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constantinople_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub petersburg_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub istanbul_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muir_glacier_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub berlin_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub london_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrow_glacier_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gray_glacier_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_netsplit_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shanghai_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancun_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_total_difficulty: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_total_difficulty_passed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ethash: Option<EthashConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clique: Option<CliqueConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthashConfig {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliqueConfig {
    pub period: u64,
    pub epoch: u64,
}

/// serde support for `secretKey` in genesis

pub mod secret_key {
    use alloy_primitives::Bytes;
    use alloy_signer::LocalWallet;
    use k256::{ecdsa::SigningKey, SecretKey};
    use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(value: &Option<LocalWallet>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if let Some(wallet) = value {
            let signer: SigningKey = wallet.signer().clone();
            let signer_bytes = signer.to_bytes();
            let signer_bytes2: [u8; 32] = *signer_bytes.as_ref();
            Bytes::from(signer_bytes2).serialize(serializer)
        } else {
            serializer.serialize_none()
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<LocalWallet>, D::Error>
    where
        D: Deserializer<'de>,
    {
        if let Some(s) = Option::<Bytes>::deserialize(deserializer)? {
            if s.is_empty() {
                return Ok(None)
            }
            SecretKey::from_bytes(s.as_ref().into())
                .map_err(de::Error::custom)
                .map(Into::into)
                .map(Some)
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_parse_genesis_json() {
        let s = r#"{
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
            "balance": "0xffffffffffffffffffffffffff",
            "secretkey": "0x305b526d493844b63466be6d48a424ab83f5216011eef860acc6db4c1821adc9"
        }
    },
    "number": "0x0",
    "gasUsed": "0x0",
    "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000"
}
"#;

        let gen: Genesis = serde_json::from_str(s).unwrap();
        assert_eq!(gen.nonce, Some(16045690984833335023));
        assert_eq!(gen.gas_limit, 2147483648);
        assert_eq!(gen.difficulty, 131072);
        assert_eq!(gen.alloc.accounts.len(), 1);
        let config = gen.config.unwrap();
        assert_eq!(config.chain_id, Some(19763));
    }
}
