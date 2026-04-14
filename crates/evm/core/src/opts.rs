use crate::{
    EvmEnv, FoundryBlock, FoundryTransaction,
    constants::DEFAULT_CREATE2_DEPLOYER,
    fork::CreateFork,
    utils::{apply_chain_and_block_specific_env_changes, block_env_from_header},
};
use alloy_chains::NamedChain;
use alloy_consensus::BlockHeader;
use alloy_network::{AnyNetwork, BlockResponse, Network};
use alloy_primitives::{Address, B256, BlockNumber, ChainId, U256};
use alloy_provider::{Provider, RootProvider};
use alloy_rpc_types::{BlockNumberOrTag, anvil::NodeInfo};
use eyre::WrapErr;
use foundry_common::{ALCHEMY_FREE_TIER_CUPS, NON_ARCHIVE_NODE_WARNING, provider::ProviderBuilder};
use foundry_config::{Chain, Config, GasLimit};
use foundry_evm_networks::NetworkConfigs;
use revm::{context::CfgEnv, primitives::hardfork::SpecId};
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
    /// Returns a `RootProvider` for the given fork URL configured with options in `self` and
    /// annotated `Network` type.
    pub fn fork_provider_with_url<N: Network>(
        &self,
        fork_url: &str,
    ) -> eyre::Result<RootProvider<N>> {
        ProviderBuilder::new(fork_url)
            .maybe_max_retry(self.fork_retries)
            .maybe_initial_backoff(self.fork_retry_backoff)
            .maybe_headers(self.fork_headers.clone())
            .compute_units_per_second(self.get_compute_units_per_second())
            .build()
    }

    /// Infers the network configuration from the fork chain ID if not already set.
    ///
    /// When a fork URL is configured and the network has not been explicitly set,
    /// this fetches the chain ID from the remote endpoint and calls
    /// [`NetworkConfigs::with_chain_id`] to auto-enable the correct network
    /// (e.g. Tempo, OP Stack) based on the chain ID.
    pub async fn infer_network_from_fork(&mut self) {
        if !self.networks.is_tempo()
            && !self.networks.is_optimism()
            && let Some(ref fork_url) = self.fork_url
            && let Ok(provider) = self.fork_provider_with_url::<AnyNetwork>(fork_url)
            && let Ok(chain_id) = provider.get_chain_id().await
        {
            // If Anvil's chain, request anvil_nodeInfo to determine if the network is Tempo.
            if chain_id == NamedChain::AnvilHardhat as u64 {
                if let Ok(node_info) =
                    provider.raw_request::<_, NodeInfo>("anvil_nodeInfo".into(), ()).await
                    && node_info.network.is_some_and(|network| network == "tempo")
                {
                    self.networks = NetworkConfigs::with_tempo();
                }
            } else {
                self.networks = self.networks.with_chain_id(chain_id);
            }
        }
    }

    /// Returns a tuple with [`EvmEnv`], `TxEnv`, and the actual fork block number.
    ///
    /// If a `fork_url` is set, creates a provider and passes it to both `EvmOpts::fork_evm_env`
    /// and `EvmOpts::fork_tx_env`. Falls back to local settings when no fork URL is configured.
    ///
    /// The fork block number is returned separately because on some L2s (e.g., Arbitrum) the
    /// `block_env.number` may be remapped (to the L1 block number) and therefore cannot be used
    /// to pin the fork.
    pub async fn env<
        SPEC: Into<SpecId> + Default + Copy,
        BLOCK: FoundryBlock + Default,
        TX: FoundryTransaction + Default,
    >(
        &self,
    ) -> eyre::Result<(EvmEnv<SPEC, BLOCK>, TX, Option<BlockNumber>)> {
        if let Some(ref fork_url) = self.fork_url {
            let provider = self.fork_provider_with_url::<AnyNetwork>(fork_url)?;
            let ((evm_env, block_number), tx) =
                tokio::try_join!(self.fork_evm_env(&provider), self.fork_tx_env(&provider))?;
            Ok((evm_env, tx, Some(block_number)))
        } else {
            Ok((self.local_evm_env(), self.local_tx_env(), None))
        }
    }

    /// Returns the [`EvmEnv`] (cfg + block) and [`BlockNumber`] fetched from the fork endpoint via
    /// provider
    pub async fn fork_evm_env<
        SPEC: Into<SpecId> + Default + Copy,
        BLOCK: FoundryBlock + Default,
        N: Network,
        P: Provider<N>,
    >(
        &self,
        provider: &P,
    ) -> eyre::Result<(EvmEnv<SPEC, BLOCK>, BlockNumber)> {
        trace!(
            memory_limit = %self.memory_limit,
            override_chain_id = ?self.env.chain_id,
            pin_block = ?self.fork_block_number,
            origin = %self.sender,
            disable_block_gas_limit = %self.disable_block_gas_limit,
            enable_tx_gas_limit = %self.enable_tx_gas_limit,
            configs = ?self.networks,
            "creating fork environment"
        );

        let bn = match self.fork_block_number {
            Some(bn) => BlockNumberOrTag::Number(bn),
            None => BlockNumberOrTag::Latest,
        };

        let (chain_id, block) = tokio::try_join!(
            option_try_or_else(self.env.chain_id, async || provider.get_chain_id().await),
            provider.get_block_by_number(bn)
        )
        .wrap_err_with(|| {
            let mut msg = "could not instantiate forked environment".to_string();
            if let Some(fork_url) = self.fork_url.as_deref()
                && let Ok(url) = Url::parse(fork_url)
                && let Some(host) = url.host()
            {
                write!(msg, " with provider {host}").unwrap();
            }
            msg
        })?;

        let Some(block) = block else {
            let bn_msg = match bn {
                BlockNumberOrTag::Number(bn) => format!("block number: {bn}"),
                bn => format!("{bn} block"),
            };
            let latest_msg = if let Ok(latest_block) = provider.get_block_number().await {
                if let Some(block_number) = self.fork_block_number
                    && block_number <= latest_block
                {
                    error!("{NON_ARCHIVE_NODE_WARNING}");
                }
                format!("; latest block number: {latest_block}")
            } else {
                Default::default()
            };
            eyre::bail!("failed to get {bn_msg}{latest_msg}");
        };

        let block_number = block.header().number();
        let mut evm_env = EvmEnv {
            cfg_env: self.cfg_env(chain_id),
            block_env: block_env_from_header(block.header()),
        };

        apply_chain_and_block_specific_env_changes::<N, _, _>(&mut evm_env, &block, self.networks);

        Ok((evm_env, block_number))
    }

    /// Returns the [`EvmEnv`] configured with only local settings.
    fn local_evm_env<SPEC: Into<SpecId> + Default, BLOCK: FoundryBlock + Default>(
        &self,
    ) -> EvmEnv<SPEC, BLOCK> {
        let cfg_env = self.cfg_env(self.env.chain_id.unwrap_or(foundry_common::DEV_CHAIN_ID));
        let mut block_env = BLOCK::default();
        block_env.set_number(self.env.block_number);
        block_env.set_beneficiary(self.env.block_coinbase);
        block_env.set_timestamp(self.env.block_timestamp);
        block_env.set_difficulty(U256::from(self.env.block_difficulty));
        block_env.set_prevrandao(Some(self.env.block_prevrandao));
        block_env.set_basefee(self.env.block_base_fee_per_gas);
        block_env.set_gas_limit(self.gas_limit());
        EvmEnv::new(cfg_env, block_env)
    }

    /// Returns the `TxEnv` with gas price and chain id resolved from provider.
    async fn fork_tx_env<TX: FoundryTransaction + Default, N: Network, P: Provider<N>>(
        &self,
        provider: &P,
    ) -> eyre::Result<TX> {
        let (gas_price, chain_id) = tokio::try_join!(
            option_try_or_else(self.env.gas_price.map(|v| v as u128), async || {
                provider.get_gas_price().await
            }),
            option_try_or_else(self.env.chain_id, async || provider.get_chain_id().await),
        )?;
        let mut tx_env = TX::default();
        tx_env.set_caller(self.sender);
        tx_env.set_chain_id(Some(chain_id));
        tx_env.set_gas_price(gas_price);
        tx_env.set_gas_limit(self.gas_limit());
        Ok(tx_env)
    }

    /// Returns the `TxEnv` configured from local settings only.
    fn local_tx_env<TX: FoundryTransaction + Default>(&self) -> TX {
        let mut tx_env = TX::default();
        tx_env.set_caller(self.sender);
        tx_env.set_gas_price(self.env.gas_price.unwrap_or_default().into());
        tx_env.set_gas_limit(self.gas_limit());
        tx_env
    }

    /// Builds a [`CfgEnv`] from the options, using the provided [`ChainId`].
    fn cfg_env<SPEC: Into<SpecId> + Default>(&self, chain_id: ChainId) -> CfgEnv<SPEC> {
        let mut cfg = CfgEnv::default();
        cfg.chain_id = chain_id;
        cfg.memory_limit = self.memory_limit;
        cfg.limit_contract_code_size = self.env.code_size_limit.or(Some(usize::MAX));
        // EIP-3607 rejects transactions from senders with deployed code.
        // If EIP-3607 is enabled it can cause issues during fuzz/invariant tests if the caller
        // is a contract. So we disable the check by default.
        cfg.disable_eip3607 = true;
        cfg.disable_block_gas_limit = self.disable_block_gas_limit;
        cfg.disable_nonce_check = true;
        // By default do not enforce transaction gas limits imposed by Osaka (EIP-7825).
        // Users can opt-in to enable these limits by setting `enable_tx_gas_limit` to true.
        if !self.enable_tx_gas_limit {
            cfg.tx_gas_limit_cap = Some(u64::MAX);
        }
        cfg
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
    /// `fork_block_number` is the actual block number to pin the fork to. This must be the
    /// real chain block number, not a remapped value. On some L2s (e.g., Arbitrum)
    /// `block_env.number` is remapped to the L1 block number, so callers must pass the
    /// original block number returned by [`EvmOpts::env`] instead.
    pub fn get_fork(
        &self,
        config: &Config,
        chain_id: u64,
        fork_block_number: Option<BlockNumber>,
    ) -> Option<CreateFork> {
        let url = self.fork_url.clone()?;
        let enable_caching = config.enable_caching(&url, chain_id);

        // Pin fork_block_number to the block that was already fetched in env, so subsequent
        // fork operations use the same block. This prevents inconsistencies when forking at
        // "latest" where the chain could advance between calls.
        let mut evm_opts = self.clone();
        evm_opts.fork_block_number = evm_opts.fork_block_number.or(fork_block_number);

        Some(CreateFork { url, enable_caching, evm_opts })
    }

    /// Returns the gas limit to use
    pub fn gas_limit(&self) -> u64 {
        self.env.block_gas_limit.unwrap_or(self.env.gas_limit).0
    }

    /// Returns the available compute units per second, which will be
    /// - u64::MAX, if `no_rpc_rate_limit` if set (as rate limiting is disabled)
    /// - the assigned compute units, if `compute_units_per_second` is set
    /// - ALCHEMY_FREE_TIER_CUPS (330) otherwise
    const fn get_compute_units_per_second(&self) -> u64 {
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
            && let Ok(provider) = self.fork_provider_with_url::<AnyNetwork>(url)
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

async fn option_try_or_else<T, E>(
    option: Option<T>,
    f: impl AsyncFnOnce() -> Result<T, E>,
) -> Result<T, E> {
    if let Some(value) = option { Ok(value) } else { f().await }
}

#[cfg(test)]
mod tests {
    use revm::context::{BlockEnv, TxEnv};

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn infer_network_default_anvil_selects_ethereum() {
        let (_api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;

        let config = Config::figment();
        let mut evm_opts = config.extract::<EvmOpts>().unwrap();
        evm_opts.fork_url = Some(handle.http_endpoint());
        assert_eq!(evm_opts.networks, NetworkConfigs::default());

        evm_opts.infer_network_from_fork().await;

        // Plain anvil (chain id 31337) without tempo flag -> Ethereum (no network flags set).
        assert!(!evm_opts.networks.is_tempo());
        assert!(!evm_opts.networks.is_optimism());
        assert!(!evm_opts.networks.is_celo());
        assert_eq!(evm_opts.networks, NetworkConfigs::default());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn infer_network_tempo_anvil_via_node_info() {
        let (_api, handle) = anvil::spawn(anvil::NodeConfig::test_tempo()).await;

        let config = Config::figment();
        let mut evm_opts = config.extract::<EvmOpts>().unwrap();
        evm_opts.fork_url = Some(handle.http_endpoint());
        // Networks not set -> should query anvil_nodeInfo to discover tempo.
        assert_eq!(evm_opts.networks, NetworkConfigs::default());

        evm_opts.infer_network_from_fork().await;

        assert!(evm_opts.networks.is_tempo(), "should detect tempo via anvil_nodeInfo");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn infer_network_tempo_anvil_skips_rpc_when_already_set() {
        // Use a URL that would fail if any RPC call were attempted (connection refused).
        // This proves the early-return guard prevents all network requests.
        let config = Config::figment();
        let mut evm_opts = config.extract::<EvmOpts>().unwrap();
        evm_opts.fork_url = Some("http://127.0.0.1:1".to_string());
        // Explicitly set tempo before calling infer (simulates --tempo CLI flag).
        evm_opts.networks = NetworkConfigs::with_tempo();

        evm_opts.infer_network_from_fork().await;

        // Should still be tempo, the early-return guard skips the RPC call.
        assert!(evm_opts.networks.is_tempo());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn flaky_infer_network_tempo_moderato_rpc() {
        let config = Config::figment();
        let mut evm_opts = config.extract::<EvmOpts>().unwrap();
        evm_opts.fork_url = Some("https://rpc.moderato.tempo.xyz".to_string());
        assert_eq!(evm_opts.networks, NetworkConfigs::default());

        evm_opts.infer_network_from_fork().await;

        // Tempo Moderato has a known Tempo chain ID -> should be inferred via with_chain_id.
        assert!(evm_opts.networks.is_tempo(), "should detect tempo from Moderato chain ID");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_fork_pins_block_number_from_env() {
        let endpoint = foundry_test_utils::rpc::next_http_rpc_endpoint();

        let config = Config::figment();
        let mut evm_opts = config.extract::<EvmOpts>().unwrap();
        evm_opts.fork_url = Some(endpoint.clone());
        // Explicitly leave fork_block_number as None to simulate --fork-url without --block-number
        assert!(evm_opts.fork_block_number.is_none());

        // Fetch the environment (this resolves "latest" to an actual block number)
        let (evm_env, _, fork_block) = evm_opts.env::<SpecId, BlockEnv, TxEnv>().await.unwrap();
        assert!(fork_block.is_some(), "should have resolved a fork block number");
        let resolved_block = fork_block.unwrap();
        assert!(resolved_block > 0, "should have resolved to a real block number");

        // Create the fork - this should pin the block number
        let fork =
            evm_opts.get_fork(&Config::default(), evm_env.cfg_env.chain_id, fork_block).unwrap();

        // The fork's evm_opts should now have fork_block_number set to the resolved block
        assert_eq!(
            fork.evm_opts.fork_block_number,
            Some(resolved_block),
            "get_fork should pin fork_block_number to the block from env"
        );
    }

    // Regression test for https://github.com/foundry-rs/foundry/issues/13576
    // On Arbitrum, `block_env.number` is remapped to the L1 block number by
    // `apply_chain_and_block_specific_env_changes`. The fork block number returned
    // by `env()` must be the actual L2 block number, not the remapped L1 value.
    #[tokio::test(flavor = "multi_thread")]
    async fn flaky_get_fork_uses_l2_block_number_on_arbitrum() {
        let endpoint =
            foundry_test_utils::rpc::next_rpc_endpoint(foundry_config::NamedChain::Arbitrum);

        let config = Config::figment();
        let mut evm_opts = config.extract::<EvmOpts>().unwrap();
        evm_opts.fork_url = Some(endpoint.clone());
        assert!(evm_opts.fork_block_number.is_none());

        let (evm_env, _, fork_block) = evm_opts.env::<SpecId, BlockEnv, TxEnv>().await.unwrap();
        let fork_block = fork_block.expect("should have resolved a fork block number");

        // On Arbitrum, block_env.number is the L1 block number (much smaller).
        // The fork_block should be the actual L2 block number (much larger).
        let block_env_number: u64 = evm_env.block_env.number.to();
        assert!(
            fork_block > block_env_number,
            "fork_block ({fork_block}) should be the L2 block, which is larger than \
             block_env.number ({block_env_number}) which is the L1 block on Arbitrum"
        );

        // Verify get_fork pins to the correct L2 block number
        let fork = evm_opts
            .get_fork(&Config::default(), evm_env.cfg_env.chain_id, Some(fork_block))
            .unwrap();
        assert_eq!(
            fork.evm_opts.fork_block_number,
            Some(fork_block),
            "get_fork should pin to the L2 block number, not the L1 block number"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_fork_preserves_explicit_block_number() {
        let endpoint = foundry_test_utils::rpc::next_http_rpc_endpoint();

        let config = Config::figment();
        let mut evm_opts = config.extract::<EvmOpts>().unwrap();
        evm_opts.fork_url = Some(endpoint.clone());
        // Set an explicit block number
        evm_opts.fork_block_number = Some(12345678);

        let (evm_env, _, fork_block) = evm_opts.env::<SpecId, BlockEnv, TxEnv>().await.unwrap();

        let fork =
            evm_opts.get_fork(&Config::default(), evm_env.cfg_env.chain_id, fork_block).unwrap();

        // Should preserve the explicit block number, not override it
        assert_eq!(
            fork.evm_opts.fork_block_number,
            Some(12345678),
            "get_fork should preserve explicitly set fork_block_number"
        );
    }
}
