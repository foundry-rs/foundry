use ethers::{
    providers::{Middleware, Provider},
    solc::utils::RuntimeOrHandle,
    types::{Address, Chain, U256},
};
use revm::{BlockEnv, CfgEnv, SpecId, TxEnv};
use serde::{Deserialize, Deserializer, Serialize};

use foundry_common;

use super::fork::environment;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvmOpts {
    #[serde(flatten)]
    pub env: Env,

    /// Fetch state over a remote instead of starting from empty state
    #[serde(rename = "eth_rpc_url")]
    pub fork_url: Option<String>,

    /// pins the block number for the state fork
    pub fork_block_number: Option<u64>,

    /// Disables storage caching entirely.
    pub no_storage_caching: bool,

    /// the initial balance of each deployed test contract
    pub initial_balance: U256,

    /// the address which will be executing all tests
    pub sender: Address,

    /// enables the FFI cheatcode
    pub ffi: bool,

    /// Verbosity mode of EVM output as number of occurrences
    pub verbosity: u8,

    /// The memory limit of the EVM in bytes.
    pub memory_limit: u64,
}

impl EvmOpts {
    pub async fn evm_env(&self) -> revm::Env {
        if let Some(ref fork_url) = self.fork_url {
            let provider =
                Provider::try_from(fork_url.as_str()).expect("could not instantiated provider");
            environment(
                &provider,
                self.memory_limit,
                self.env.gas_price,
                self.env.chain_id,
                self.fork_block_number,
                self.sender,
            )
            .await
            .expect("could not instantiate forked environment")
        } else {
            revm::Env {
                block: BlockEnv {
                    number: self.env.block_number.into(),
                    coinbase: self.env.block_coinbase,
                    timestamp: self.env.block_timestamp.into(),
                    difficulty: self.env.block_difficulty.into(),
                    basefee: self.env.block_base_fee_per_gas.into(),
                    gas_limit: self.gas_limit(),
                },
                cfg: CfgEnv {
                    chain_id: self.env.chain_id.unwrap_or(foundry_common::DEV_CHAIN_ID).into(),
                    spec_id: SpecId::LONDON,
                    perf_all_precompiles_have_balance: false,
                    memory_limit: self.memory_limit,
                },
                tx: TxEnv {
                    gas_price: self.env.gas_price.unwrap_or_default().into(),
                    gas_limit: self.gas_limit().as_u64(),
                    caller: self.sender,
                    ..Default::default()
                },
            }
        }
    }

    /// Returns the gas limit to use
    pub fn gas_limit(&self) -> U256 {
        self.env.block_gas_limit.unwrap_or(self.env.gas_limit).into()
    }

    /// Returns the configured chain id, which will be
    ///   - the value of `chain_id` if set
    ///   - mainnet if `fork_url` contains "mainnet"
    ///   - the chain if `fork_url` is set and the endpoints returned its chain id successfully
    ///   - mainnet otherwise
    pub fn get_chain_id(&self) -> u64 {
        if let Some(id) = self.env.chain_id {
            return id
        }
        self.get_remote_chain_id().map_or(Chain::Mainnet as u64, |id| id as u64)
    }

    /// Returns the chain ID from the RPC, if any.
    pub fn get_remote_chain_id(&self) -> Option<Chain> {
        if let Some(ref url) = self.fork_url {
            if url.contains("mainnet") {
                tracing::trace!("auto detected mainnet chain from url {url}");
                return Some(Chain::Mainnet)
            }
            let provider = Provider::try_from(url.as_str())
                .unwrap_or_else(|_| panic!("Failed to establish provider to {url}"));

            if let Ok(id) = RuntimeOrHandle::new().block_on(provider.get_chainid()) {
                return Chain::try_from(id.as_u64()).ok()
            }
        }

        None
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Env {
    /// the block gas limit
    #[serde(deserialize_with = "string_or_number")]
    pub gas_limit: u64,

    /// the chainid opcode value
    pub chain_id: Option<u64>,

    /// the tx.gasprice value during EVM execution
    ///
    /// This is an Option, so we can determine in fork mode whether to use the config's gas price
    /// (if set by user) or the remote client's gas price.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas_price: Option<u64>,

    /// the base fee in a block
    pub block_base_fee_per_gas: u64,

    /// the tx.origin value during EVM execution
    pub tx_origin: Address,

    /// the block.coinbase value during EVM execution
    pub block_coinbase: Address,

    /// the block.timestamp value during EVM execution
    pub block_timestamp: u64,

    /// the block.number value during EVM execution"
    pub block_number: u64,

    /// the block.difficulty value during EVM execution
    pub block_difficulty: u64,

    /// the block.gaslimit value during EVM execution
    #[serde(deserialize_with = "string_or_number_opt")]
    pub block_gas_limit: Option<u64>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Gas {
    Number(u64),
    Text(String),
}

fn string_or_number<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    match Gas::deserialize(deserializer)? {
        Gas::Number(num) => Ok(num),
        Gas::Text(s) => s.parse().map_err(D::Error::custom),
    }
}

fn string_or_number_opt<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;

    match Option::<Gas>::deserialize(deserializer)? {
        Some(gas) => match gas {
            Gas::Number(num) => Ok(Some(num)),
            Gas::Text(s) => s.parse().map(Some).map_err(D::Error::custom),
        },
        _ => Ok(None),
    }
}
