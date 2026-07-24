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
use alloy_rpc_types::{BlockId, BlockNumberOrTag, anvil::NodeInfo};
use eyre::{OptionExt, WrapErr};
use foundry_common::{ALCHEMY_FREE_TIER_CUPS, NON_ARCHIVE_NODE_WARNING, provider::ProviderBuilder};
use foundry_config::{Chain, Config, ExecutionSpec, FoundryHardfork, GasLimit};
#[cfg(feature = "monad")]
use foundry_evm_hardforks::MonadHardfork;
use foundry_evm_hardforks::TempoHardfork;
use foundry_evm_networks::{NetworkConfigs, NetworkVariant};
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

/// Identity and block context of the remote chain backing a fork.
///
/// The source chain ID remains distinct from the configured `CHAINID` opcode override.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ForkContext {
    /// Chain ID reported by the fork endpoint.
    pub source_chain_id: ChainId,
    /// Actual block number fetched from the fork endpoint.
    pub block_number: BlockNumber,
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
    /// Resolves and pins an unpinned fork to the current latest block.
    pub async fn pin_fork_block(&mut self) -> eyre::Result<Option<BlockNumber>> {
        if self.fork_block_number.is_none()
            && let Some(fork_url) = &self.fork_url
        {
            self.fork_block_number = Some(
                self.fork_provider_with_url::<AnyNetwork>(fork_url)?.get_block_number().await?,
            );
        }
        Ok(self.fork_block_number)
    }

    /// Returns whether the configured CREATE2 deployer can be used for library linking.
    ///
    /// Locally Foundry can only install its canonical deployer. On forks, any deployer with code
    /// is usable because the call executes against the forked state.
    pub async fn can_use_create2_deployer(
        &self,
        fork_block: Option<BlockNumber>,
    ) -> eyre::Result<bool> {
        let Some(fork_url) = &self.fork_url else {
            return Ok(self.create2_deployer == DEFAULT_CREATE2_DEPLOYER);
        };
        let block = fork_block.ok_or_else(|| eyre::eyre!("fork block must be resolved"))?;
        let provider = self.fork_provider_with_url::<AnyNetwork>(fork_url)?;
        Ok(!provider
            .get_code_at(self.create2_deployer)
            .block_id(BlockId::number(block))
            .await?
            .is_empty())
    }

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
    /// this fetches the chain ID from the remote endpoint, caches it for subsequent fork setup,
    /// and calls [`NetworkConfigs::with_chain_id`] to auto-enable the correct network (e.g. Tempo,
    /// OP Stack) based on the chain ID.
    pub async fn infer_network_from_fork(&mut self) {
        #[cfg(feature = "optimism")]
        let already_op = self.networks.is_optimism();
        #[cfg(not(feature = "optimism"))]
        let already_op = false;
        if !self.networks.is_tempo()
            && !self.networks.is_monad()
            && !already_op
            && let Some(ref fork_url) = self.fork_url
            && let Ok(provider) = self.fork_provider_with_url::<AnyNetwork>(fork_url)
            && let Ok(chain_id) = provider.get_chain_id().await
        {
            self.env.chain_id.get_or_insert(chain_id);

            // If Anvil's chain, request anvil_nodeInfo to determine the enabled network.
            if chain_id == NamedChain::AnvilHardhat as u64 {
                if let Ok(node_info) =
                    provider.raw_request::<_, NodeInfo>("anvil_nodeInfo".into(), ()).await
                {
                    match node_info.network.as_deref() {
                        Some("tempo") => self.networks = NetworkConfigs::with_tempo(),
                        #[cfg(feature = "monad")]
                        Some("monad") => self.networks = NetworkConfigs::with_monad(),
                        _ => {}
                    }
                }
            } else {
                self.networks = self.networks.with_chain_id(chain_id);
            }
        }
    }

    /// Resolves the chain ID and network family exposed by the configured fork endpoint.
    ///
    /// Unlike [`Self::infer_network_from_fork`], this always inspects the endpoint and returns an
    /// error when the network cannot be determined. This is useful for callers that cannot change
    /// their concrete EVM implementation after startup.
    pub async fn fork_network(&self) -> eyre::Result<(ChainId, NetworkVariant)> {
        let fork_url = self.fork_url.as_deref().ok_or_eyre("fork URL is not configured")?;
        let provider = self.fork_provider_with_url::<AnyNetwork>(fork_url)?;
        let chain_id = provider
            .get_chain_id()
            .await
            .wrap_err("failed to retrieve chain ID from fork endpoint")?;

        let known_network = known_network_variant(chain_id)?;
        if chain_id == NamedChain::AnvilHardhat as u64 || known_network.is_none() {
            let network = match provider
                .raw_request::<_, NodeInfo>("anvil_nodeInfo".into(), ())
                .await
            {
                Ok(node_info) => network_variant_from_node_info(node_info.network.as_deref())?,
                // Hardhat uses the same default chain ID but does not expose Anvil's metadata RPC.
                Err(error)
                    if chain_id == NamedChain::AnvilHardhat as u64
                        && error
                            .as_error_resp()
                            .is_some_and(|response| response.code == -32601) =>
                {
                    NetworkVariant::Ethereum
                }
                Err(error)
                    if error.as_error_resp().is_some_and(|response| response.code == -32601) =>
                {
                    eyre::bail!(
                        "cannot determine network family for unknown chain ID {chain_id}: the fork \
                         endpoint does not expose `anvil_nodeInfo`"
                    )
                }
                Err(error) => {
                    return Err(error)
                        .wrap_err("failed to determine network family from fork endpoint");
                }
            };
            return Ok((chain_id, network));
        }

        Ok((chain_id, known_network.expect("checked above")))
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
        let (evm_env, tx_env, fork_context) = self.env_with_fork_context().await?;
        Ok((evm_env, tx_env, fork_context.map(|context| context.block_number)))
    }

    /// Returns the execution environment and the source identity of its remote fork, if any.
    ///
    /// The source chain ID is always fetched from the fork endpoint, even when the execution
    /// environment applies a configured `CHAINID` opcode override.
    pub async fn env_with_fork_context<
        SPEC: Into<SpecId> + Default + Copy,
        BLOCK: FoundryBlock + Default,
        TX: FoundryTransaction + Default,
    >(
        &self,
    ) -> eyre::Result<(EvmEnv<SPEC, BLOCK>, TX, Option<ForkContext>)> {
        if let Some(ref fork_url) = self.fork_url {
            let provider = self.fork_provider_with_url::<AnyNetwork>(fork_url)?;
            let ((evm_env, fork_context), tx) = tokio::try_join!(
                self.fork_evm_env_with_context(&provider),
                self.fork_tx_env(&provider)
            )?;
            Ok((evm_env, tx, Some(fork_context)))
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
        let (evm_env, context) = self.fork_evm_env_with_context(provider).await?;
        Ok((evm_env, context.block_number))
    }

    /// Returns the fork environment together with the remote chain identity used to build it.
    pub(crate) async fn fork_evm_env_with_context<
        SPEC: Into<SpecId> + Default + Copy,
        BLOCK: FoundryBlock + Default,
        N: Network,
        P: Provider<N>,
    >(
        &self,
        provider: &P,
    ) -> eyre::Result<(EvmEnv<SPEC, BLOCK>, ForkContext)> {
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

        let (source_chain_id, block) =
            tokio::try_join!(provider.get_chain_id(), provider.get_block_by_number(bn))
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
        let chain_id = self.env.chain_id.unwrap_or(source_chain_id);
        let mut evm_env = EvmEnv {
            cfg_env: self.cfg_env(chain_id),
            block_env: block_env_from_header(block.header()),
        };

        apply_chain_and_block_specific_env_changes::<N, _, _>(&mut evm_env, &block, self.networks);

        Ok((evm_env, ForkContext { source_chain_id, block_number }))
    }

    /// Returns the [`EvmEnv`] configured with only local settings.
    fn local_evm_env<SPEC: Into<SpecId> + Default + Clone, BLOCK: FoundryBlock + Default>(
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
    fn cfg_env<SPEC: Into<SpecId> + Default + Clone>(&self, chain_id: ChainId) -> CfgEnv<SPEC> {
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
    ///   - `StorageCachingConfig` allows the `fork_url` + source chain ID pair
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
        source_chain_id: u64,
        fork_block_number: Option<BlockNumber>,
    ) -> Option<CreateFork> {
        let url = self.fork_url.clone()?;
        let enable_caching = config.enable_caching(&url, source_chain_id);

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

fn known_network_variant(chain_id: ChainId) -> eyre::Result<Option<NetworkVariant>> {
    let chain = Chain::from_id(chain_id);
    if chain.is_tempo() {
        return Ok(Some(NetworkVariant::Tempo));
    }
    if matches!(chain.named(), Some(NamedChain::Monad | NamedChain::MonadTestnet)) {
        #[cfg(feature = "monad")]
        return Ok(Some(NetworkVariant::Monad));
        #[cfg(not(feature = "monad"))]
        eyre::bail!("network family `monad` is not enabled in this build");
    }
    if chain.is_optimism() {
        #[cfg(feature = "optimism")]
        return Ok(Some(NetworkVariant::Optimism));
        #[cfg(not(feature = "optimism"))]
        eyre::bail!("network family `optimism` is not enabled in this build");
    }
    Ok(chain.named().map(|_| NetworkVariant::Ethereum))
}

fn network_variant_from_node_info(network: Option<&str>) -> eyre::Result<NetworkVariant> {
    match network {
        None => Ok(NetworkVariant::Ethereum),
        #[cfg(feature = "optimism")]
        Some("optimism") => Ok(NetworkVariant::Optimism),
        #[cfg(not(feature = "optimism"))]
        Some("optimism") => eyre::bail!("network family `optimism` is not enabled in this build"),
        Some("tempo") => Ok(NetworkVariant::Tempo),
        #[cfg(feature = "monad")]
        Some("monad") => Ok(NetworkVariant::Monad),
        #[cfg(not(feature = "monad"))]
        Some("monad") => eyre::bail!("network family `monad` is not enabled in this build"),
        Some(network) => {
            eyre::bail!("unsupported network family `{network}` reported by fork endpoint")
        }
    }
}

/// Resolves and applies the execution spec for an EVM environment.
///
/// A direct caller override takes precedence over a configured namespaced hardfork, followed by
/// the hardfork active at the source chain and environment timestamp when `schedule_chain_id` is
/// provided. Networks without a known activation schedule and local environments fall back to
/// their configured EVM version.
///
/// Returns the exact namespaced hardfork, when applicable, so execution and trace decoding can use
/// the same hardfork.
pub fn resolve_execution_spec<SPEC, BLOCK>(
    config: &Config,
    networks: NetworkConfigs,
    evm_env: &mut EvmEnv<SPEC, BLOCK>,
    schedule_chain_id: Option<ChainId>,
    explicit_spec: Option<SPEC>,
    explicit_hardfork: Option<FoundryHardfork>,
) -> Option<FoundryHardfork>
where
    SPEC: ExecutionSpec + Into<SpecId> + Copy,
    BLOCK: FoundryBlock,
{
    let supports = |hardfork| SPEC::from_foundry_hardfork(hardfork).is_some();
    let configured_hardfork = config.hardfork.filter(|&hardfork| supports(hardfork));
    let timestamp_hardfork = schedule_chain_id
        .and_then(|chain_id| {
            FoundryHardfork::from_chain_and_timestamp(
                chain_id,
                evm_env.block_env.timestamp().saturating_to(),
            )
        })
        .filter(|&hardfork| supports(hardfork));
    let fallback_hardfork = if networks.is_tempo() {
        Some(FoundryHardfork::Tempo(config.evm_spec_id::<TempoHardfork>()))
    } else {
        #[cfg(feature = "monad")]
        let hardfork = networks
            .is_monad()
            .then(|| FoundryHardfork::Monad(config.evm_spec_id::<MonadHardfork>()));
        #[cfg(not(feature = "monad"))]
        let hardfork = None;
        hardfork
    };

    let resolved_hardfork = if explicit_spec.is_some() {
        explicit_hardfork
    } else {
        configured_hardfork.or(timestamp_hardfork).or(fallback_hardfork)
    };
    let spec = explicit_spec
        .or_else(|| resolved_hardfork.and_then(SPEC::from_foundry_hardfork))
        .unwrap_or_else(|| config.evm_spec_id());
    evm_env.cfg_env.set_spec_and_mainnet_gas_params(spec);

    resolved_hardfork
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

    #[cfg(feature = "monad")]
    fn monad_env(timestamp: u64) -> EvmEnv<MonadHardfork, BlockEnv> {
        let mut block = BlockEnv::default();
        block.set_timestamp(U256::from(timestamp));
        let mut cfg = CfgEnv::new_with_spec(MonadHardfork::default());
        cfg.chain_id = NamedChain::Monad as u64;
        EvmEnv::new(cfg, block)
    }

    #[test]
    #[cfg(feature = "monad")]
    fn resolve_execution_spec_uses_monad_activation_timestamp() {
        let config = Config::default();
        let networks = NetworkConfigs::with_monad();
        let activation = MonadHardfork::MonadNine.mainnet_activation_timestamp().unwrap();

        let mut before = monad_env(activation - 1);
        assert_eq!(
            resolve_execution_spec(
                &config,
                networks,
                &mut before,
                Some(NamedChain::Monad as u64),
                None,
                None,
            ),
            Some(FoundryHardfork::Monad(MonadHardfork::MonadEight))
        );
        assert_eq!(before.cfg_env.spec, MonadHardfork::MonadEight);

        let mut after = monad_env(activation);
        assert_eq!(
            resolve_execution_spec(
                &config,
                networks,
                &mut after,
                Some(NamedChain::Monad as u64),
                None,
                None,
            ),
            Some(FoundryHardfork::Monad(MonadHardfork::MonadNine))
        );
        assert_eq!(after.cfg_env.spec, MonadHardfork::MonadNine);
    }

    #[test]
    #[cfg(feature = "monad")]
    fn resolve_execution_spec_ignores_schedule_for_local_env() {
        let config = Config::default();
        let networks = NetworkConfigs::with_monad();
        let activation = MonadHardfork::MonadNine.mainnet_activation_timestamp().unwrap();
        let mut env = monad_env(activation - 1);

        assert_eq!(
            resolve_execution_spec(&config, networks, &mut env, None, None, None),
            Some(FoundryHardfork::Monad(MonadHardfork::MonadNine))
        );
        assert_eq!(env.cfg_env.spec, MonadHardfork::MonadNine);
    }

    #[test]
    #[cfg(feature = "monad")]
    fn resolve_execution_spec_honors_explicit_precedence() {
        let networks = NetworkConfigs::with_monad();
        let activation = MonadHardfork::MonadNine.mainnet_activation_timestamp().unwrap();
        let mut configured = Config {
            hardfork: Some(FoundryHardfork::Monad(MonadHardfork::MonadNine)),
            ..Default::default()
        };
        let mut env = monad_env(activation - 1);

        assert_eq!(
            resolve_execution_spec(
                &configured,
                networks,
                &mut env,
                Some(NamedChain::Monad as u64),
                None,
                None,
            ),
            configured.hardfork
        );
        assert_eq!(env.cfg_env.spec, MonadHardfork::MonadNine);

        configured.hardfork = None;
        assert_eq!(
            resolve_execution_spec(
                &configured,
                networks,
                &mut env,
                Some(NamedChain::Monad as u64),
                Some(MonadHardfork::MonadEight),
                Some(FoundryHardfork::Monad(MonadHardfork::MonadEight)),
            ),
            Some(FoundryHardfork::Monad(MonadHardfork::MonadEight))
        );
        assert_eq!(env.cfg_env.spec, MonadHardfork::MonadEight);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg(feature = "monad")]
    async fn fork_context_preserves_source_chain_with_execution_override() {
        let activation = MonadHardfork::MonadNine.mainnet_activation_timestamp().unwrap();
        let (_api, handle) = anvil::spawn(
            anvil::NodeConfig::test()
                .with_chain_id(Some(NamedChain::Monad as u64))
                .with_genesis_timestamp(Some(activation - 1)),
        )
        .await;
        let mut evm_opts = EvmOpts { fork_url: Some(handle.http_endpoint()), ..Default::default() };
        evm_opts.env.chain_id = Some(NamedChain::Mainnet as u64);

        let (mut evm_env, tx_env, fork_context) =
            evm_opts.env_with_fork_context::<MonadHardfork, BlockEnv, TxEnv>().await.unwrap();
        let fork_context = fork_context.unwrap();

        assert_eq!(fork_context.source_chain_id, NamedChain::Monad as u64);
        assert_eq!(evm_env.cfg_env.chain_id, NamedChain::Mainnet as u64);
        assert_eq!(tx_env.chain_id, Some(NamedChain::Mainnet as u64));
        assert_eq!(
            resolve_execution_spec(
                &Config::default(),
                NetworkConfigs::with_monad(),
                &mut evm_env,
                Some(fork_context.source_chain_id),
                None,
                None,
            ),
            Some(FoundryHardfork::Monad(MonadHardfork::MonadEight))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn infer_network_default_anvil_selects_ethereum() {
        let (_api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;

        let config = Config::figment();
        let mut evm_opts = config.extract::<EvmOpts>().unwrap();
        evm_opts.fork_url = Some(handle.http_endpoint());
        assert_eq!(evm_opts.networks, NetworkConfigs::default());

        evm_opts.infer_network_from_fork().await;

        // Plain anvil (chain id 31337) without tempo flag -> Ethereum (no network flags set).
        assert_eq!(evm_opts.env.chain_id, Some(31337));
        assert!(!evm_opts.networks.is_tempo());
        #[cfg(feature = "optimism")]
        assert!(!evm_opts.networks.is_optimism());
        assert!(!evm_opts.networks.is_celo());
        assert_eq!(evm_opts.networks, NetworkConfigs::default());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_network_detects_ethereum_anvil() {
        let (_api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
        let evm_opts = EvmOpts { fork_url: Some(handle.http_endpoint()), ..Default::default() };

        assert_eq!(
            evm_opts.fork_network().await.unwrap(),
            (NamedChain::AnvilHardhat as u64, NetworkVariant::Ethereum)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_network_uses_anvil_metadata_for_unknown_chain() {
        let chain_id = 98_765_432;
        let (_api, handle) =
            anvil::spawn(anvil::NodeConfig::test().with_chain_id(Some(chain_id))).await;
        let evm_opts = EvmOpts { fork_url: Some(handle.http_endpoint()), ..Default::default() };

        assert_eq!(evm_opts.fork_network().await.unwrap(), (chain_id, NetworkVariant::Ethereum));
    }

    #[test]
    fn known_network_variant_does_not_guess_unknown_chain() {
        assert_eq!(known_network_variant(98_765_432).unwrap(), None);
    }

    #[test]
    #[cfg(feature = "monad")]
    fn known_network_variant_classifies_monad() {
        assert_eq!(
            known_network_variant(NamedChain::Monad as u64).unwrap(),
            Some(NetworkVariant::Monad)
        );
    }

    #[test]
    #[cfg(not(feature = "monad"))]
    fn known_network_variant_rejects_disabled_monad() {
        assert_eq!(
            known_network_variant(NamedChain::Monad as u64).unwrap_err().to_string(),
            "network family `monad` is not enabled in this build"
        );
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

    #[tokio::test]
    async fn create2_deployer_availability_requires_resolved_fork_block() {
        let evm_opts =
            EvmOpts { fork_url: Some("http://127.0.0.1:1".to_string()), ..Default::default() };

        let err = evm_opts.can_use_create2_deployer(None).await.unwrap_err();
        assert!(err.to_string().contains("fork block must be resolved"));
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg(feature = "monad")]
    async fn infer_network_monad_anvil_via_node_info() {
        let (_api, handle) = anvil::spawn(anvil::NodeConfig::test_monad()).await;

        let config = Config::figment();
        let mut evm_opts = config.extract::<EvmOpts>().unwrap();
        evm_opts.fork_url = Some(handle.http_endpoint());
        // Networks not set -> should query anvil_nodeInfo to discover Monad.
        assert_eq!(evm_opts.networks, NetworkConfigs::default());

        evm_opts.infer_network_from_fork().await;

        assert!(evm_opts.networks.is_monad(), "should detect Monad via anvil_nodeInfo");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg(feature = "monad")]
    async fn fork_network_detects_monad_anvil() {
        let (_api, handle) = anvil::spawn(anvil::NodeConfig::test_monad()).await;
        let evm_opts = EvmOpts { fork_url: Some(handle.http_endpoint()), ..Default::default() };

        assert_eq!(
            evm_opts.fork_network().await.unwrap(),
            (NamedChain::AnvilHardhat as u64, NetworkVariant::Monad)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg(feature = "monad")]
    async fn infer_network_monad_anvil_skips_rpc_when_already_set() {
        // Use a URL that would fail if any RPC call were attempted (connection refused).
        // This proves the early-return guard prevents all network requests.
        let config = Config::figment();
        let mut evm_opts = config.extract::<EvmOpts>().unwrap();
        evm_opts.fork_url = Some("http://127.0.0.1:1".to_string());
        // Explicitly set Monad before calling infer (simulates network config).
        evm_opts.networks = NetworkConfigs::with_monad();

        evm_opts.infer_network_from_fork().await;

        // Should still be Monad, the early-return guard skips the RPC call.
        assert!(evm_opts.networks.is_monad());
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
