use super::fork::environment;
use crate::{
    EvmEnv,
    constants::DEFAULT_CREATE2_DEPLOYER,
    fork::{CreateFork, configure_env},
};
use alloy_network::Network;
use alloy_primitives::{Address, B256, U256};
use alloy_provider::{Provider, network::AnyRpcBlock};
use eyre::WrapErr;
use foundry_common::{
    ALCHEMY_FREE_TIER_CUPS,
    provider::{ProviderBuilder, RetryProvider},
};
use foundry_config::{Chain, Config, GasLimit};
use foundry_evm_networks::NetworkConfigs;
use revm::context::{BlockEnv, TxEnv};
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use url::Url;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvmOpts {
    /// The EVM environment configuration.
    #[serde(flatten)]
    pub env: Env,

    /// Fetch state over a remote instead of starting from empty state.
    #[serde(rename = "eth_rpc_url")]
    pub fork_url: Option<String>,

    /// Pins the block number for the state fork.
    pub fork_block_number: Option<u64>,

    /// The number of retries.
    pub fork_retries: Option<u32>,

    /// Initial retry backoff.
    pub fork_retry_backoff: Option<u64>,

    /// Headers to use with `fork_url`
    pub fork_headers: Option<Vec<String>>,

    /// The available compute units per second.
    ///
    /// See also <https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second>
    pub compute_units_per_second: Option<u64>,

    /// Disables RPC rate limiting entirely.
    pub no_rpc_rate_limit: bool,

    /// Disables storage caching entirely.
    pub no_storage_caching: bool,

    /// The initial balance of each deployed test contract.
    pub initial_balance: U256,

    /// The address which will be executing all tests.
    pub sender: Address,

    /// Enables the FFI cheatcode.
    pub ffi: bool,

    /// Use the create 2 factory in all cases including tests and non-broadcasting scripts.
    pub always_use_create_2_factory: bool,

    /// Verbosity mode of EVM output as number of occurrences.
    pub verbosity: u8,

    /// The memory limit per EVM execution in bytes.
    /// If this limit is exceeded, a `MemoryLimitOOG` result is thrown.
    pub memory_limit: u64,

    /// Whether to enable isolation of calls.
    pub isolate: bool,

    /// Whether to disable block gas limit checks.
    pub disable_block_gas_limit: bool,

    /// Whether to enable tx gas limit checks as imposed by Osaka (EIP-7825).
    pub enable_tx_gas_limit: bool,

    #[serde(flatten)]
    /// Networks with enabled features.
    pub networks: NetworkConfigs,

    /// The CREATE2 deployer's address.
    pub create2_deployer: Address,
}

impl Default for EvmOpts {
    fn default() -> Self {
        Self {
            env: Env::default(),
            fork_url: None,
            fork_block_number: None,
            fork_retries: None,
            fork_retry_backoff: None,
            fork_headers: None,
            compute_units_per_second: None,
            no_rpc_rate_limit: false,
            no_storage_caching: false,
            initial_balance: U256::default(),
            sender: Address::default(),
            ffi: false,
            always_use_create_2_factory: false,
            verbosity: 0,
            memory_limit: 0,
            isolate: false,
            disable_block_gas_limit: false,
            enable_tx_gas_limit: false,
            networks: NetworkConfigs::default(),
            create2_deployer: DEFAULT_CREATE2_DEPLOYER,
        }
    }
}

impl EvmOpts {
    /// Returns a `RetryProvider` for the given fork URL configured with options in `self`.
    pub fn fork_provider_with_url(&self, fork_url: &str) -> eyre::Result<RetryProvider> {
        ProviderBuilder::new(fork_url)
            .maybe_max_retry(self.fork_retries)
            .maybe_initial_backoff(self.fork_retry_backoff)
            .maybe_headers(self.fork_headers.clone())
            .compute_units_per_second(self.get_compute_units_per_second())
            .build()
    }

    /// Configures a new `revm::Env`
    ///
    /// If a `fork_url` is set, it gets configured with settings fetched from the endpoint (chain
    /// id, )
    pub async fn evm_env(&self) -> eyre::Result<crate::Env> {
        if let Some(ref fork_url) = self.fork_url {
            Ok(self.fork_evm_env(fork_url).await?.0)
        } else {
            Ok(self.local_evm_env())
        }
    }

    /// Returns the `revm::Env` that is configured with settings retrieved from the endpoint,
    /// and the block that was used to configure the environment.
    pub async fn fork_evm_env(&self, fork_url: &str) -> eyre::Result<(crate::Env, AnyRpcBlock)> {
        let provider = self.fork_provider_with_url(fork_url)?;
        self.fork_evm_env_with_provider(fork_url, &provider).await
    }

    /// Returns the `revm::Env` that is configured with settings retrieved from the provider,
    /// and the block that was used to configure the environment.
    pub async fn fork_evm_env_with_provider<P: Provider<N>, N: Network>(
        &self,
        fork_url: &str,
        provider: &P,
    ) -> eyre::Result<(crate::Env, N::BlockResponse)> {
        environment(
            provider,
            self.memory_limit,
            self.env.gas_price.map(|v| v as u128),
            self.env.chain_id,
            self.fork_block_number,
            self.sender,
            self.disable_block_gas_limit,
            self.enable_tx_gas_limit,
            self.networks,
        )
        .await
        .wrap_err_with(|| {
            let mut msg = "could not instantiate forked environment".to_string();
            if let Ok(url) = Url::parse(fork_url)
                && let Some(provider) = url.host()
            {
                write!(msg, " with provider {provider}").unwrap();
            }
            msg
        })
    }

    /// Returns the `revm::Env` configured with only local settings
    fn local_evm_env(&self) -> crate::Env {
        let cfg = configure_env(
            self.env.chain_id.unwrap_or(foundry_common::DEV_CHAIN_ID),
            self.memory_limit,
            self.disable_block_gas_limit,
            self.enable_tx_gas_limit,
        );

        crate::Env {
            evm_env: EvmEnv {
                cfg_env: cfg,
                block_env: BlockEnv {
                    number: self.env.block_number,
                    beneficiary: self.env.block_coinbase,
                    timestamp: self.env.block_timestamp,
                    difficulty: U256::from(self.env.block_difficulty),
                    prevrandao: Some(self.env.block_prevrandao),
                    basefee: self.env.block_base_fee_per_gas,
                    gas_limit: self.gas_limit(),
                    ..Default::default()
                },
            },
            tx: TxEnv {
                gas_price: self.env.gas_price.unwrap_or_default().into(),
                gas_limit: self.gas_limit(),
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
    ///   - `StorageCachingConfig` allows the `fork_url` + chain ID pair
    ///   - storage is allowed (`no_storage_caching = false`)
    ///
    /// If all these criteria are met, then storage caching is enabled and storage info will be
    /// written to `<Config::foundry_cache_dir()>/<str(chainid)>/<block>/storage.json`.
    ///
    /// for `mainnet` and `--fork-block-number 14435000` on mac the corresponding storage cache will
    /// be at `~/.foundry/cache/mainnet/14435000/storage.json`.
    pub fn get_fork(&self, config: &Config, env: crate::Env) -> Option<CreateFork> {
        let url = self.fork_url.clone()?;
        let enable_caching = config.enable_caching(&url, env.evm_env.cfg_env.chain_id);
        Some(CreateFork { url, enable_caching, env, evm_opts: self.clone() })
    }

    /// Returns the gas limit to use
    pub fn gas_limit(&self) -> u64 {
        self.env.block_gas_limit.unwrap_or(self.env.gas_limit).0
    }

    /// Returns the available compute units per second, which will be
    /// - u64::MAX, if `no_rpc_rate_limit` if set (as rate limiting is disabled)
    /// - the assigned compute units, if `compute_units_per_second` is set
    /// - ALCHEMY_FREE_TIER_CUPS (330) otherwise
    fn get_compute_units_per_second(&self) -> u64 {
        if self.no_rpc_rate_limit {
            u64::MAX
        } else if let Some(cups) = self.compute_units_per_second {
            cups
        } else {
            ALCHEMY_FREE_TIER_CUPS
        }
    }

    /// Returns the chain ID from the RPC, if any.
    pub async fn get_remote_chain_id(&self) -> Option<Chain> {
        if let Some(url) = &self.fork_url
            && let Ok(provider) = self.fork_provider_with_url(url)
        {
            trace!(?url, "retrieving chain via eth_chainId");

            if let Ok(id) = provider.get_chain_id().await {
                return Some(Chain::from(id));
            }

            // Provider URLs could be of the format `{CHAIN_IDENTIFIER}-mainnet`
            // (e.g. Alchemy `opt-mainnet`, `arb-mainnet`), fallback to this method only
            // if we're not able to retrieve chain id from `RetryProvider`.
            if url.contains("mainnet") {
                trace!(?url, "auto detected mainnet chain");
                return Some(Chain::mainnet());
            }
        }

        None
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Env {
    /// The block gas limit.
    pub gas_limit: GasLimit,

    /// The `CHAINID` opcode value.
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
    #[serde(
        deserialize_with = "foundry_config::deserialize_u64_to_u256",
        serialize_with = "foundry_config::serialize_u64_or_u256"
    )]
    pub block_timestamp: U256,

    /// the block.number value during EVM execution"
    #[serde(
        deserialize_with = "foundry_config::deserialize_u64_to_u256",
        serialize_with = "foundry_config::serialize_u64_or_u256"
    )]
    pub block_number: U256,

    /// the block.difficulty value during EVM execution
    pub block_difficulty: u64,

    /// Previous block beacon chain random value. Before merge this field is used for mix_hash
    pub block_prevrandao: B256,

    /// the block.gaslimit value during EVM execution
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_gas_limit: Option<GasLimit>,

    /// EIP-170: Contract code size limit in bytes. Useful to increase this because of tests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_size_limit: Option<usize>,
}
