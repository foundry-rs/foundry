use crate::executor::fork::CreateFork;
use ethers::{
    providers::{Middleware, Provider},
    solc::utils::RuntimeOrHandle,
    types::{Address, Chain, H256, U256},
};
use eyre::WrapErr;
use foundry_common::{self, ProviderBuilder, RpcUrl, ALCHEMY_FREE_TIER_CUPS};
use foundry_config::Config;
use revm::{BlockEnv, CfgEnv, SpecId, TxEnv};
use serde::{Deserialize, Deserializer, Serialize};

use super::fork::environment;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvmOpts {
    #[serde(flatten)]
    pub env: Env,

    /// Fetch state over a remote instead of starting from empty state
    #[serde(rename = "eth_rpc_url")]
    pub fork_url: Option<RpcUrl>,

    /// pins the block number for the state fork
    pub fork_block_number: Option<u64>,

    /// initial retry backoff
    pub fork_retry_backoff: Option<u64>,

    /// The available compute units per second
    pub compute_units_per_second: Option<u64>,

    /// Disables rate limiting entirely.
    pub no_rpc_rate_limit: bool,

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
    /// Configures a new `revm::Env`
    ///
    /// If a `fork_url` is set, it gets configured with settings fetched from the endpoint (chain
    /// id, )
    pub async fn evm_env(&self) -> revm::Env {
        if let Some(ref fork_url) = self.fork_url {
            self.fork_evm_env(fork_url).await.expect("Could not instantiate forked environment")
        } else {
            self.local_evm_env()
        }
    }

    /// Convenience implementation to configure a `revm::Env` from non async code
    ///
    /// This only attaches are creates a temporary tokio runtime if `fork_url` is set
    ///
    /// Returns an error if a RPC request failed, or the fork url is not a valid url
    pub fn evm_env_blocking(&self) -> eyre::Result<revm::Env> {
        if let Some(ref fork_url) = self.fork_url {
            RuntimeOrHandle::new().block_on(async { self.fork_evm_env(fork_url).await })
        } else {
            Ok(self.local_evm_env())
        }
    }

    /// Returns the `revm::Env` configured with settings retrieved from the endpoints
    pub async fn fork_evm_env(&self, fork_url: impl AsRef<str>) -> eyre::Result<revm::Env> {
        let fork_url = fork_url.as_ref();
        let provider = ProviderBuilder::new(fork_url)
            .compute_units_per_second(self.get_compute_units_per_second())
            .build()?;
        environment(
            &provider,
            self.memory_limit,
            self.env.gas_price,
            self.env.chain_id,
            self.fork_block_number,
            self.sender,
        )
        .await
        .wrap_err_with(|| {
            format!("Could not instantiate forked environment with fork url: {fork_url}")
        })
    }

    /// Returns the `revm::Env` configured with only local settings
    pub fn local_evm_env(&self) -> revm::Env {
        revm::Env {
            block: BlockEnv {
                number: self.env.block_number.into(),
                coinbase: self.env.block_coinbase,
                timestamp: self.env.block_timestamp.into(),
                difficulty: self.env.block_difficulty.into(),
                prevrandao: Some(self.env.block_prevrandao),
                basefee: self.env.block_base_fee_per_gas.into(),
                gas_limit: self.gas_limit(),
            },
            cfg: CfgEnv {
                chain_id: self.env.chain_id.unwrap_or(foundry_common::DEV_CHAIN_ID).into(),
                spec_id: SpecId::MERGE,
                limit_contract_code_size: self.env.code_size_limit.or(Some(usize::MAX)),
                memory_limit: self.memory_limit,
                // EIP-3607 rejects transactions from senders with deployed code.
                // If EIP-3607 is enabled it can cause issues during fuzz/invariant tests if the
                // caller is a contract. So we disable the check by default.
                disable_eip3607: true,
                ..Default::default()
            },
            tx: TxEnv {
                gas_price: self.env.gas_price.unwrap_or_default().into(),
                gas_limit: self.gas_limit().as_u64(),
                caller: self.sender,
                ..Default::default()
            },
        }
    }

    /// Helper function that returns the [CreateFork] to use, if any.
    ///
    /// storage caching for the [CreateFork] will be enabled if
    ///   - `fork_url` is present
    ///   - `fork_block_number` is present
    ///   - [StorageCachingConfig] allows the `fork_url` +  chain id pair
    ///   - storage is allowed (`no_storage_caching = false`)
    ///
    /// If all these criteria are met, then storage caching is enabled and storage info will be
    /// written to [Config::foundry_cache_dir()]/<str(chainid)>/<block>/storage.json
    ///
    /// for `mainnet` and `--fork-block-number 14435000` on mac the corresponding storage cache will
    /// be at `~/.foundry/cache/mainnet/14435000/storage.json`
    pub fn get_fork(&self, config: &Config, env: revm::Env) -> Option<CreateFork> {
        let url = self.fork_url.clone()?;
        let enable_caching = config.enable_caching(&url, env.cfg.chain_id.as_u64());
        Some(CreateFork { url, enable_caching, env, evm_opts: self.clone() })
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

    /// Returns the available compute units per second, which will be
    /// - u64::MAX, if `no_rpc_rate_limit` if set (as rate limiting is disabled)
    /// - the assigned compute units, if `compute_units_per_second` is set
    /// - ALCHEMY_FREE_TIER_CUPS (330) otherwise
    pub fn get_compute_units_per_second(&self) -> u64 {
        if self.no_rpc_rate_limit {
            u64::MAX
        } else if let Some(cups) = self.compute_units_per_second {
            return cups
        } else {
            ALCHEMY_FREE_TIER_CUPS
        }
    }

    /// Returns the chain ID from the RPC, if any.
    pub fn get_remote_chain_id(&self) -> Option<Chain> {
        if let Some(ref url) = self.fork_url {
            if url.contains("mainnet") {
                tracing::trace!(?url, "auto detected mainnet chain");
                return Some(Chain::Mainnet)
            }
            tracing::trace!(?url, "retrieving chain via eth_chainId");
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

    /// Previous block beacon chain random value. Before merge this field is used for mix_hash
    pub block_prevrandao: H256,

    /// the block.gaslimit value during EVM execution
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "string_or_number_opt"
    )]
    pub block_gas_limit: Option<u64>,

    /// EIP-170: Contract code size limit in bytes. Useful to increase this because of tests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_size_limit: Option<usize>,
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
