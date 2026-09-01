use crate::{
    EvmEnv, FoundryBlock, FoundryTransaction,
    constants::DEFAULT_CREATE2_DEPLOYER,
    fork::{CreateFork, ResolvedFork},
    utils::{apply_chain_and_block_specific_env_changes_for_chain, block_env_from_header},
};
#[cfg(test)]
use alloy_chains::NamedChain;
use alloy_consensus::BlockHeader;
use alloy_eips::BlockNumHash;
use alloy_network::{AnyNetwork, BlockResponse, Network, primitives::HeaderResponse};
use alloy_primitives::{Address, B256, BlockNumber, ChainId, U256};
use alloy_provider::{Provider, RootProvider};
use alloy_rpc_types::{
    BlockId, BlockNumberOrTag,
    anvil::{Metadata, NodeInfo},
};
use eyre::{OptionExt, WrapErr};
use foundry_common::{
    ALCHEMY_FREE_TIER_CUPS, NON_ARCHIVE_NODE_WARNING,
    provider::{ProviderBuilder, is_rpc_method_not_found},
};
use foundry_config::{Chain, Config, ExecutionSpec, FoundryHardfork, GasLimit};
use foundry_evm_hardforks::TempoHardfork;
use foundry_evm_networks::{NetworkConfigs, NetworkVariant};
use revm::{context::CfgEnv, primitives::hardfork::SpecId};
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use url::Url;

/// EVM execution options, including the configured remote fork.
///
/// Fork handling separates two responsibilities. Endpoint discovery identifies and caches the
/// execution family without selecting a block. Exact resolution binds the configured source and
/// selector to a block number, hash, and endpoint context. Forge normally performs discovery
/// before resolution, while callers that already know the network can resolve directly.
///
/// An implicit `latest` selector remains unchanged in these options; only the returned
/// [`ResolvedFork`] is pinned to the exact block observed during resolution.
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

    /// JWT secret to use with the configured RPC endpoint.
    #[serde(rename = "eth_rpc_jwt")]
    pub rpc_jwt: Option<String>,

    /// Headers to use with the configured RPC endpoint.
    #[serde(rename = "eth_rpc_headers")]
    pub rpc_headers: Option<Vec<String>>,

    /// Request timeout to use with the configured RPC endpoint.
    #[serde(rename = "eth_rpc_timeout")]
    pub rpc_timeout: Option<u64>,

    /// Whether to accept invalid certificates from the configured RPC endpoint.
    #[serde(default, rename = "eth_rpc_accept_invalid_certs")]
    pub rpc_accept_invalid_certs: bool,

    /// Whether to disable automatic proxy detection for the configured RPC endpoint.
    #[serde(default, rename = "eth_rpc_no_proxy")]
    pub rpc_no_proxy: bool,

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

    /// Most recently discovered endpoint identity, cached for network dispatch and revalidation.
    #[serde(skip)]
    pub fork_endpoint: Option<ForkEndpointIdentity>,

    /// Endpoint identity promoted to an invariant that later discovery must match.
    #[serde(skip)]
    pub expected_fork_endpoint: Option<ForkEndpointIdentity>,

    /// Whether the active network selection was inferred from the fork endpoint.
    #[serde(skip)]
    pub fork_network_is_inferred: bool,

    /// Whether `env.chain_id` was inferred from the fork endpoint.
    #[serde(skip)]
    pub fork_chain_id_is_inferred: bool,

    /// Whether `fork_block_number` was pinned from the fork endpoint's latest block.
    #[serde(skip)]
    pub fork_block_number_is_inferred: bool,
}

/// A snapshot of the execution and upstream-source identity exposed by an RPC endpoint.
///
/// This identity is independent of the block selector Forge resolves. In particular,
/// `source_fork_block_*` describes the upstream block from which an Anvil endpoint was itself
/// forked; the target block selected for local execution belongs to [`ResolvedFork`]. Equality
/// detects observable endpoint identity changes, including Anvil instance resets, and
/// execution-profile changes between related RPC reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkEndpointIdentity {
    /// RPC URL from which this identity was discovered.
    ///
    /// Request headers and JWT credentials are bound separately by [`ResolvedFork`].
    pub endpoint: String,
    /// Chain ID exposed through `eth_chainId`.
    pub execution_chain_id: ChainId,
    /// Underlying source chain ID, when the endpoint is itself a fork.
    pub source_chain_id: ChainId,
    /// EVM family exposed by the endpoint.
    pub network: NetworkVariant,
    /// Complete execution profile exposed by the endpoint.
    pub network_profile: NetworkConfigs,
    /// Raw hardfork name reported by an Anvil endpoint.
    pub reported_hardfork: Option<String>,
    /// Parsed hardfork exposed by an Anvil endpoint, when recognized.
    pub hardfork: Option<FoundryHardfork>,
    /// Anvil instance identifier used to detect resets during multi-call discovery.
    pub instance_id: Option<B256>,
    /// Block number from which an Anvil endpoint was forked.
    pub source_fork_block_number: Option<BlockNumber>,
    /// Block hash from which an Anvil endpoint was forked.
    pub source_fork_block_hash: Option<B256>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EndpointHardforkPolicy {
    /// Permit an unrecognized hardfork during early network-family discovery.
    Optional,
    /// Require a recognized hardfork before constructing an execution environment.
    Required,
}

/// Best-effort Anvil detection that becomes strict after positive identification.
///
/// Before the first successful `anvil_nodeInfo` response, any probe failure means that the
/// optional capability is unavailable. Mandatory standard RPC reads still expose endpoint-wide
/// failures. Once a response or cached endpoint identity identifies Anvil, every later probe
/// failure is returned so it cannot hide an endpoint reset or execution-profile change.
#[derive(Clone, Copy, Debug, Default)]
struct AnvilNodeInfoProbe {
    identified: bool,
}

impl AnvilNodeInfoProbe {
    const fn new(identified: bool) -> Self {
        Self { identified }
    }

    async fn request<N: Network, P: Provider<N>>(
        &mut self,
        provider: &P,
    ) -> eyre::Result<Option<NodeInfo>> {
        match provider.raw_request::<_, NodeInfo>("anvil_nodeInfo".into(), ()).await {
            Ok(node_info) => {
                self.identified = true;
                Ok(Some(node_info))
            }
            Err(_) if !self.identified => Ok(None),
            Err(error) => Err(error).wrap_err("failed to determine network family from endpoint"),
        }
    }
}

#[derive(Clone, Copy)]
enum ForkBlockTarget {
    Configured(BlockNumberOrTag),
    Resolved(BlockNumHash),
}

fn endpoint_hardfork(
    network: NetworkVariant,
    hardfork: &str,
    policy: EndpointHardforkPolicy,
) -> eyre::Result<Option<FoundryHardfork>> {
    match network.parse_hardfork(hardfork) {
        Ok(hardfork) => Ok(Some(hardfork)),
        Err(_) if policy == EndpointHardforkPolicy::Optional => Ok(None),
        Err(error) => Err(eyre::Report::msg(error)).wrap_err_with(|| {
            format!("unsupported hardfork `{hardfork}` reported for `{network}`")
        }),
    }
}

/// Endpoint identity paired with the remote block number backing a fork.
///
/// [`ResolvedFork`] adds the exact block hash and configured RPC source. The remote
/// `block_number` may differ from a remapped EVM block number on L2s, and the source chain ID
/// remains distinct from a configured `CHAINID` opcode override.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub struct ForkContext {
    /// Chain ID exposed through `eth_chainId`.
    pub execution_chain_id: ChainId,
    /// Underlying source chain ID.
    pub source_chain_id: ChainId,
    /// Execution network exposed by the endpoint.
    pub network: NetworkVariant,
    /// Complete execution profile exposed by the endpoint.
    pub network_profile: NetworkConfigs,
    /// Actual block number fetched from the fork endpoint.
    pub block_number: BlockNumber,
    /// Exact hardfork reported by the fork endpoint, when available.
    pub hardfork: Option<FoundryHardfork>,
    /// Anvil instance identifier, when exposed by the endpoint.
    pub instance_id: Option<B256>,
    /// Block number from which the endpoint itself was forked.
    pub source_fork_block_number: Option<BlockNumber>,
    /// Block hash from which the endpoint itself was forked.
    pub source_fork_block_hash: Option<B256>,
}

impl ForkContext {
    /// Returns whether this block context belongs to `identity`.
    pub fn matches_identity(self, identity: &ForkEndpointIdentity) -> bool {
        self.execution_chain_id == identity.execution_chain_id
            && self.source_chain_id == identity.source_chain_id
            && self.network == identity.network
            && self.network_profile == identity.network_profile
            && self.hardfork == identity.hardfork
            && self.instance_id == identity.instance_id
            && self.source_fork_block_number == identity.source_fork_block_number
            && self.source_fork_block_hash == identity.source_fork_block_hash
    }

    /// Returns whether two block contexts belong to the same endpoint identity.
    pub(crate) fn has_same_endpoint_identity(self, other: Self) -> bool {
        self.execution_chain_id == other.execution_chain_id
            && self.source_chain_id == other.source_chain_id
            && self.network == other.network
            && self.network_profile == other.network_profile
            && self.hardfork == other.hardfork
            && self.instance_id == other.instance_id
            && self.source_fork_block_number == other.source_fork_block_number
            && self.source_fork_block_hash == other.source_fork_block_hash
    }

    /// Returns whether two contexts can reuse a backend pinned by block number.
    pub fn has_same_backend_target(self, other: Self) -> bool {
        self.has_same_endpoint_identity(other) && self.block_number == other.block_number
    }
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
            rpc_jwt: None,
            rpc_headers: None,
            rpc_timeout: None,
            rpc_accept_invalid_certs: false,
            rpc_no_proxy: false,
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
            fork_endpoint: None,
            expected_fork_endpoint: None,
            fork_network_is_inferred: false,
            fork_chain_id_is_inferred: false,
            fork_block_number_is_inferred: false,
        }
    }
}

impl EvmOpts {
    /// Clears state derived from a fork endpoint while preserving explicit configuration.
    fn clear_endpoint_derived_fork_state(&mut self) {
        if self.fork_block_number_is_inferred {
            self.fork_block_number = None;
            self.fork_block_number_is_inferred = false;
        }
        self.fork_endpoint = None;
        self.expected_fork_endpoint = None;
        if self.fork_network_is_inferred {
            self.networks = self.networks.with_rpc_profile(NetworkConfigs::default());
            self.fork_network_is_inferred = false;
        }
        if self.fork_chain_id_is_inferred {
            self.env.chain_id = None;
            self.fork_chain_id_is_inferred = false;
        }
    }

    /// Selects a new fork URL and clears endpoint-derived state from the previous source.
    pub fn set_fork_url(&mut self, fork_url: String) {
        if self.fork_url.as_ref() == Some(&fork_url) {
            return;
        }
        self.fork_url = Some(fork_url);
        self.clear_endpoint_derived_fork_state();
    }

    /// Invalidates endpoint-derived state when the effective RPC source changed since `fork` was
    /// resolved.
    ///
    /// The effective source consists of the URL, active headers, and JWT. Explicit block, network,
    /// and chain-ID selections are retained so the new source is resolved under the same user
    /// configuration.
    pub fn invalidate_fork_endpoint_if_source_changed(&mut self, fork: &ResolvedFork) -> bool {
        let matches = self.fork_url.as_deref().is_some_and(|fork_url| {
            fork.matches_source(fork_url, self.fork_source_headers(), self.rpc_jwt.as_deref())
        });
        if matches {
            return false;
        }
        self.clear_endpoint_derived_fork_state();
        true
    }

    /// Applies an explicit network override, replacing any endpoint-inferred selection.
    pub fn set_explicit_network(&mut self, network: NetworkVariant) {
        self.networks = network.into();
        self.fork_network_is_inferred = false;
    }

    /// Requires subsequent local fork construction to use this exact endpoint identity.
    pub fn expect_fork_endpoint(
        &mut self,
        identity: ForkEndpointIdentity,
        network_is_inferred: bool,
    ) {
        self.fork_endpoint = Some(identity.clone());
        self.expected_fork_endpoint = Some(identity);
        self.fork_network_is_inferred = network_is_inferred;
    }

    fn ensure_expected_fork_endpoint(&self, identity: &ForkEndpointIdentity) -> eyre::Result<()> {
        if let Some(expected) = &self.expected_fork_endpoint
            && expected != identity
        {
            eyre::bail!(
                "fork endpoint {} changed after its execution context was selected",
                fork_endpoint_description(&identity.endpoint)
            );
        }
        Ok(())
    }

    fn chain_id_override(&self) -> Option<ChainId> {
        (!self.fork_chain_id_is_inferred).then_some(self.env.chain_id).flatten()
    }

    fn endpoint_network_fallback(&self) -> NetworkConfigs {
        if self.networks.has_network_selection() && !self.fork_network_is_inferred {
            self.networks
        } else {
            NetworkConfigs::default()
        }
    }

    fn anvil_node_info_probe(&self) -> AnvilNodeInfoProbe {
        let endpoint = self.fork_url.as_deref();
        let identified = [self.fork_endpoint.as_ref(), self.expected_fork_endpoint.as_ref()]
            .into_iter()
            .flatten()
            .any(|identity| {
                Some(identity.endpoint.as_str()) == endpoint && identity.reported_hardfork.is_some()
            });
        AnvilNodeInfoProbe::new(identified)
    }

    fn fork_source_headers(&self) -> Option<&[String]> {
        self.fork_headers.as_deref().or(self.rpc_headers.as_deref())
    }

    fn resolved_fork(
        &self,
        fork_url: &str,
        block: BlockNumHash,
        context: ForkContext,
    ) -> ResolvedFork {
        ResolvedFork::new(
            fork_url,
            self.fork_source_headers(),
            self.rpc_jwt.as_deref(),
            self.fork_block_number,
            block,
            context,
        )
    }

    /// Converts an implicit `latest` selector into a block-number selector in place.
    ///
    /// This only reads the endpoint's current block number; it does not resolve a block hash or
    /// endpoint identity and is therefore not a reorg-safe fork snapshot. Use
    /// [`Self::resolve_fork`] when later reads and backend construction must share an exact
    /// identity.
    pub async fn pin_fork_block(&mut self) -> eyre::Result<Option<BlockNumber>> {
        if self.fork_block_number.is_none()
            && let Some(fork_url) = &self.fork_url
        {
            self.fork_block_number = Some(
                self.fork_provider_with_url::<AnyNetwork>(fork_url)?.get_block_number().await?,
            );
            self.fork_block_number_is_inferred = true;
        }
        Ok(self.fork_block_number)
    }

    /// Resolves the configured fork source and selector to an exact snapshot without changing
    /// `self`.
    ///
    /// Endpoint identity is read before and after the target block and the operation is retried if
    /// it changes. The result binds the configured RPC source, configured selector, exact block
    /// number and hash, and [`ForkContext`]. When the selector is implicit `latest`, only the
    /// returned [`ResolvedFork`] is pinned.
    pub async fn resolve_fork(&self) -> eyre::Result<Option<ResolvedFork>> {
        let Some(fork_url) = &self.fork_url else { return Ok(None) };
        let provider = self.fork_provider_with_url::<AnyNetwork>(fork_url)?;
        let target = ForkBlockTarget::Configured(match self.fork_block_number {
            Some(block_number) => BlockNumberOrTag::Number(block_number),
            None => BlockNumberOrTag::Latest,
        });
        let mut node_info_probe = self.anvil_node_info_probe();
        let (_, block, context) =
            self.resolve_fork_block_with_context(&provider, target, &mut node_info_probe).await?;
        Ok(Some(self.resolved_fork(fork_url, block, context)))
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

    /// Returns whether the configured CREATE2 deployer existed at the resolved fork block.
    pub async fn can_use_create2_deployer_resolved(
        &self,
        fork: Option<&ResolvedFork>,
    ) -> eyre::Result<bool> {
        let Some(_) = &self.fork_url else {
            eyre::ensure!(fork.is_none(), "resolved fork provided without a configured fork");
            return Ok(self.create2_deployer == DEFAULT_CREATE2_DEPLOYER);
        };
        let fork = fork.ok_or_else(|| eyre::eyre!("fork must be resolved"))?;
        let provider = self.provider_for_resolved_fork::<AnyNetwork>(fork)?;
        self.ensure_resolved_fork_endpoint(&provider, fork).await?;
        let available = !provider
            .get_code_at(self.create2_deployer)
            .block_id(fork.exact_block_id())
            .await?
            .is_empty();
        self.ensure_resolved_fork_endpoint(&provider, fork).await?;
        Ok(available)
    }

    /// Returns whether `fork` was resolved from the currently configured source and selector.
    pub fn resolved_fork_matches(&self, fork: &ResolvedFork) -> bool {
        self.fork_url.as_deref().is_some_and(|fork_url| {
            fork.matches(
                fork_url,
                self.fork_source_headers(),
                self.rpc_jwt.as_deref(),
                self.fork_block_number,
            )
        })
    }

    /// Returns a provider configured for an already resolved fork.
    pub fn provider_for_resolved_fork<N: Network>(
        &self,
        fork: &ResolvedFork,
    ) -> eyre::Result<RootProvider<N>> {
        eyre::ensure!(
            self.resolved_fork_matches(fork),
            "resolved fork does not match the configured source and selector"
        );
        self.fork_provider_with_url(
            self.fork_url.as_deref().expect("a matching resolved fork requires a fork URL"),
        )
    }

    async fn ensure_resolved_fork_endpoint<N: Network, P: Provider<N>>(
        &self,
        provider: &P,
        fork: &ResolvedFork,
    ) -> eyre::Result<()> {
        let mut node_info_probe = self.anvil_node_info_probe();
        node_info_probe.identified |= fork.context().hardfork.is_some();
        let identity = self
            .resolve_fork_endpoint_once(
                provider,
                &mut node_info_probe,
                EndpointHardforkPolicy::Required,
            )
            .await?;
        self.ensure_expected_fork_endpoint(&identity)?;
        eyre::ensure!(
            fork.context().matches_identity(&identity),
            "fork endpoint {} changed after its block and execution context were resolved",
            fork_endpoint_description(self.fork_url.as_deref().unwrap_or_default())
        );
        Ok(())
    }

    /// Returns an account nonce at the exact block and endpoint identity of `fork`.
    pub async fn transaction_count_at_resolved_fork(
        &self,
        account: Address,
        fork: &ResolvedFork,
    ) -> eyre::Result<u64> {
        let provider = self.provider_for_resolved_fork::<AnyNetwork>(fork)?;
        self.ensure_resolved_fork_endpoint(&provider, fork).await?;
        let nonce = provider.get_transaction_count(account).block_id(fork.exact_block_id()).await?;
        self.ensure_resolved_fork_endpoint(&provider, fork).await?;
        Ok(nonce)
    }

    /// Returns a `RootProvider` for the given fork URL configured with options in `self` and
    /// annotated `Network` type.
    pub fn fork_provider_with_url<N: Network>(
        &self,
        fork_url: &str,
    ) -> eyre::Result<RootProvider<N>> {
        let mut builder = ProviderBuilder::new(fork_url)
            .maybe_max_retry(self.fork_retries)
            .maybe_initial_backoff(self.fork_retry_backoff)
            .maybe_headers(self.fork_headers.clone().or_else(|| self.rpc_headers.clone()))
            .compute_units_per_second(self.get_compute_units_per_second())
            .accept_invalid_certs(self.rpc_accept_invalid_certs)
            .no_proxy(self.rpc_no_proxy);
        if let Some(jwt) = &self.rpc_jwt {
            builder = builder.jwt(jwt);
        }
        if let Some(timeout) = self.rpc_timeout {
            builder = builder.timeout(std::time::Duration::from_secs(timeout));
        }
        builder.build()
    }

    /// Discovers and caches the network configuration and endpoint identity for a fork.
    ///
    /// This is the dispatch phase of fork setup: explicit network selections are preserved, while
    /// inferred chain and network settings are applied when no override exists. The method does not
    /// resolve or pin a fork block; block resolution happens later through [`Self::resolve_fork`]
    /// or [`Self::env_resolved`].
    pub async fn infer_network_from_fork(&mut self) -> eyre::Result<()> {
        let previous_identity = self.fork_endpoint.clone();
        let Some(fork_url) = self.fork_url.clone() else {
            self.fork_endpoint = None;
            self.expected_fork_endpoint = None;
            if self.fork_network_is_inferred {
                self.networks = self.networks.with_rpc_profile(NetworkConfigs::default());
                self.fork_network_is_inferred = false;
            }
            if self.fork_chain_id_is_inferred {
                self.env.chain_id = None;
                self.fork_chain_id_is_inferred = false;
            }
            if self.fork_block_number_is_inferred {
                self.fork_block_number = None;
                self.fork_block_number_is_inferred = false;
            }
            return Ok(());
        };
        let explicit_network =
            self.networks.has_network_selection() && !self.fork_network_is_inferred;
        let identity = self.discover_fork_endpoint().await?;
        self.ensure_expected_fork_endpoint(&identity)?;
        if self.fork_network_is_inferred
            && previous_identity.as_ref().is_some_and(|previous| {
                previous.endpoint == fork_url
                    && previous.network_profile != identity.network_profile
            })
        {
            eyre::bail!(
                "fork endpoint {} changed execution profile from `{}` to `{}`; rebuild \
                 the EVM for the new network",
                fork_endpoint_description(&fork_url),
                self.networks.execution_profile_name(),
                identity.network_profile.execution_profile_name()
            );
        }

        if self.env.chain_id.is_none() || self.fork_chain_id_is_inferred {
            self.env.chain_id = Some(identity.execution_chain_id);
            self.fork_chain_id_is_inferred = true;
        }
        if !explicit_network {
            self.networks = self.networks.with_rpc_profile(identity.network_profile);
            self.fork_network_is_inferred = true;
        }
        debug_assert_eq!(identity.endpoint, fork_url);
        self.fork_endpoint = Some(identity);
        Ok(())
    }

    /// Discovers a stable identity snapshot without selecting a fork block.
    ///
    /// Each attempt reads `eth_chainId` and the optional Anvil identity methods before and after,
    /// and returns only when both snapshots agree. Before Anvil is positively identified, a failed
    /// `anvil_nodeInfo` probe is treated as absence of optional Anvil identity information for that
    /// snapshot. Later probe failures are strict. This method does not mutate or cache the returned
    /// identity in `self`.
    pub async fn discover_fork_endpoint(&self) -> eyre::Result<ForkEndpointIdentity> {
        let fork_url = self.fork_url.as_deref().ok_or_eyre("fork URL is not configured")?;
        let provider = self.fork_provider_with_url::<AnyNetwork>(fork_url)?;
        let unknown_fallback = self.endpoint_network_fallback();
        let mut node_info_probe = self.anvil_node_info_probe();
        for _ in 0..3 {
            let before_chain_id = provider
                .get_chain_id()
                .await
                .wrap_err("failed to retrieve chain ID from fork endpoint")?;
            let before_node_info = node_info_probe.request(&provider).await?;
            let before = Self::resolve_fork_endpoint_identity(
                &provider,
                fork_url,
                before_chain_id,
                before_node_info,
                Some(unknown_fallback),
                EndpointHardforkPolicy::Optional,
            )
            .await?;
            let after_chain_id = provider
                .get_chain_id()
                .await
                .wrap_err("failed to confirm chain ID from fork endpoint")?;
            let after_node_info = node_info_probe.request(&provider).await?;
            let after = Self::resolve_fork_endpoint_identity(
                &provider,
                fork_url,
                after_chain_id,
                after_node_info,
                Some(unknown_fallback),
                EndpointHardforkPolicy::Optional,
            )
            .await?;
            if before_chain_id == after_chain_id && before == after {
                return Ok(before);
            }
        }
        eyre::bail!(
            "fork endpoint {} changed while its identity was being resolved",
            fork_endpoint_description(fork_url)
        );
    }

    /// Reads one endpoint identity snapshot without performing a stability check.
    ///
    /// The shared node-info probe starts permissive for endpoints that do not expose Anvil methods
    /// and becomes strict after Anvil is identified. Callers either bracket mutable remote reads
    /// with two snapshots or use this helper to revalidate an already resolved fork.
    async fn resolve_fork_endpoint_once<N: Network, P: Provider<N>>(
        &self,
        provider: &P,
        node_info_probe: &mut AnvilNodeInfoProbe,
        hardfork_policy: EndpointHardforkPolicy,
    ) -> eyre::Result<ForkEndpointIdentity> {
        let fork_url = self.fork_url.as_deref().ok_or_eyre("fork URL is not configured")?;
        let cached_anvil = self.fork_endpoint.as_ref().is_some_and(|identity| {
            identity.endpoint == fork_url && identity.reported_hardfork.is_some()
        });
        let node_info = node_info_probe.request(provider).await?;
        let execution_chain_id = if cached_anvil && let Some(node_info) = &node_info {
            node_info.environment.chain_id
        } else {
            provider
                .get_chain_id()
                .await
                .wrap_err("failed to retrieve chain ID from fork endpoint")?
        };
        Self::resolve_fork_endpoint_identity(
            provider,
            fork_url,
            execution_chain_id,
            node_info,
            Some(self.endpoint_network_fallback()),
            hardfork_policy,
        )
        .await
    }

    /// Resolves the chain ID and network family exposed by the configured fork endpoint.
    ///
    /// Unlike [`Self::infer_network_from_fork`], this always inspects the endpoint. Endpoints whose
    /// custom chain ID does not identify a supported network family retain Foundry's historical
    /// Ethereum fallback when `anvil_nodeInfo` is unavailable.
    pub async fn fork_network(&self) -> eyre::Result<(ChainId, NetworkVariant)> {
        let identity = self.discover_fork_endpoint().await?;

        Ok((identity.execution_chain_id, identity.network))
    }

    /// Builds an identity from one observed chain ID and optional Anvil node-info response.
    ///
    /// Anvil metadata is consulted only after node info has positively identified the endpoint.
    async fn resolve_fork_endpoint_identity<N: Network, P: Provider<N>>(
        provider: &P,
        endpoint: &str,
        execution_chain_id: ChainId,
        node_info: Option<NodeInfo>,
        unknown_fallback: Option<NetworkConfigs>,
        hardfork_policy: EndpointHardforkPolicy,
    ) -> eyre::Result<ForkEndpointIdentity> {
        match node_info {
            Some(node_info) => {
                let (
                    source_chain_id,
                    instance_id,
                    source_fork_block_number,
                    source_fork_block_hash,
                ) = match provider.raw_request::<_, Metadata>("anvil_metadata".into(), ()).await {
                    Ok(metadata) => {
                        let forked_network = metadata.forked_network;
                        (
                            forked_network.map(|fork| fork.chain_id).unwrap_or(execution_chain_id),
                            Some(metadata.instance_id),
                            forked_network.map(|fork| fork.fork_block_number),
                            forked_network.map(|fork| fork.fork_block_hash),
                        )
                    }
                    Err(error) if is_rpc_method_not_found(&error) => {
                        (execution_chain_id, None, None, None)
                    }
                    Err(error) => {
                        return Err(error)
                            .wrap_err("failed to retrieve Anvil fork source identity");
                    }
                };
                let identity_chain_id =
                    if node_info.network.is_some() { execution_chain_id } else { source_chain_id };
                let network_profile = NetworkConfigs::from_rpc_identity_profile_with_fallback(
                    identity_chain_id,
                    Some(node_info.network.as_deref()),
                    unknown_fallback,
                )
                .map_err(eyre::Report::msg)?
                .ok_or_else(|| {
                    eyre::eyre!("Anvil metadata did not identify an execution profile")
                })?;
                let network = network_profile.execution_network();
                let hardfork = endpoint_hardfork(network, &node_info.hard_fork, hardfork_policy)?;

                Ok(ForkEndpointIdentity {
                    endpoint: endpoint.to_string(),
                    execution_chain_id,
                    source_chain_id,
                    network,
                    network_profile,
                    reported_hardfork: Some(node_info.hard_fork),
                    hardfork,
                    instance_id,
                    source_fork_block_number,
                    source_fork_block_hash,
                })
            }
            None => {
                let network_profile = NetworkConfigs::from_rpc_identity_profile_with_fallback(
                    execution_chain_id,
                    None,
                    unknown_fallback,
                )
                .map_err(eyre::Report::msg)?;
                if let Some(network_profile) = network_profile {
                    let network = network_profile.execution_network();
                    return Ok(ForkEndpointIdentity {
                        endpoint: endpoint.to_string(),
                        execution_chain_id,
                        source_chain_id: execution_chain_id,
                        network,
                        network_profile,
                        reported_hardfork: None,
                        hardfork: None,
                        instance_id: None,
                        source_fork_block_number: None,
                        source_fork_block_hash: None,
                    });
                }
                Err(eyre::eyre!(
                    "cannot determine network family for unknown chain ID \
                     {execution_chain_id}: the fork endpoint does not expose `anvil_nodeInfo`"
                ))
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
        let (evm_env, tx, fork) = self.env_resolved().await?;
        Ok((evm_env, tx, fork.as_ref().map(ResolvedFork::number)))
    }

    /// Returns the EVM and transaction environments with an exact resolved fork snapshot.
    ///
    /// Resolution brackets block and gas-price reads with endpoint identity checks and retries if
    /// the endpoint changes. Downstream preflight reads and backend construction should reuse the
    /// returned [`ResolvedFork`] instead of carrying only its block number.
    pub async fn env_resolved<
        SPEC: Into<SpecId> + Default + Copy,
        BLOCK: FoundryBlock + Default,
        TX: FoundryTransaction + Default,
    >(
        &self,
    ) -> eyre::Result<(EvmEnv<SPEC, BLOCK>, TX, Option<ResolvedFork>)> {
        let Some(fork_url) = &self.fork_url else {
            return Ok((self.local_evm_env(), self.local_tx_env(), None));
        };
        let provider = self.fork_provider_with_url::<AnyNetwork>(fork_url)?;
        let mut node_info_probe = self.anvil_node_info_probe();
        for _ in 0..3 {
            let (evm_env, block, context) =
                self.fork_evm_env_resolved_with_context(&provider, &mut node_info_probe).await?;
            let gas_price =
                option_try_or_else(self.env.gas_price.map(|value| value as u128), async || {
                    provider.get_gas_price().await
                })
                .await?;
            let identity = self
                .resolve_fork_endpoint_once(
                    &provider,
                    &mut node_info_probe,
                    EndpointHardforkPolicy::Required,
                )
                .await?;
            self.ensure_expected_fork_endpoint(&identity)?;
            if context.matches_identity(&identity) {
                let chain_id = self.chain_id_override().unwrap_or(context.execution_chain_id);
                let tx = self.fork_tx_env(gas_price, chain_id);
                return Ok((evm_env, tx, Some(self.resolved_fork(fork_url, block, context))));
            }
        }
        eyre::bail!(
            "fork endpoint {} changed while its EVM and transaction environments were being \
             resolved",
            fork_endpoint_description(fork_url)
        );
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
        let (evm_env, tx, fork) = self.env_resolved().await?;
        Ok((evm_env, tx, fork.as_ref().map(ResolvedFork::context)))
    }

    /// Returns the EVM and transaction environments at an already resolved fork.
    pub async fn env_with_resolved_fork<
        SPEC: Into<SpecId> + Default + Copy,
        BLOCK: FoundryBlock + Default,
        TX: FoundryTransaction + Default,
    >(
        &self,
        fork: Option<&ResolvedFork>,
    ) -> eyre::Result<(EvmEnv<SPEC, BLOCK>, TX)> {
        let Some(_) = &self.fork_url else {
            eyre::ensure!(fork.is_none(), "resolved fork provided without a configured fork");
            return Ok((self.local_evm_env(), self.local_tx_env()));
        };
        let fork = fork.ok_or_else(|| eyre::eyre!("fork must be resolved"))?;
        let provider = self.provider_for_resolved_fork::<AnyNetwork>(fork)?;
        let mut node_info_probe = self.anvil_node_info_probe();
        node_info_probe.identified |= fork.context().hardfork.is_some();
        for _ in 0..3 {
            let (evm_env, endpoint) = self
                .fork_evm_env_at_resolved_with_context(&provider, fork, &mut node_info_probe)
                .await?;
            let gas_price =
                option_try_or_else(self.env.gas_price.map(|value| value as u128), async || {
                    provider.get_gas_price().await
                })
                .await?;
            let identity = self
                .resolve_fork_endpoint_once(
                    &provider,
                    &mut node_info_probe,
                    EndpointHardforkPolicy::Required,
                )
                .await?;
            self.ensure_expected_fork_endpoint(&identity)?;
            if endpoint == fork.context() && endpoint.matches_identity(&identity) {
                let chain_id = self.chain_id_override().unwrap_or(endpoint.execution_chain_id);
                return Ok((evm_env, self.fork_tx_env(gas_price, chain_id)));
            }
        }
        eyre::bail!(
            "fork endpoint {} changed while its resolved environment was being reconstructed",
            fork_endpoint_description(self.fork_url.as_deref().unwrap_or_default())
        );
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
        let mut node_info_probe = self.anvil_node_info_probe();
        let (evm_env, block, _) =
            self.fork_evm_env_resolved_with_context(provider, &mut node_info_probe).await?;
        Ok((evm_env, block.number))
    }

    /// Returns the EVM environment and block identity fetched from the fork endpoint.
    pub(crate) async fn fork_evm_env_resolved<
        SPEC: Into<SpecId> + Default + Copy,
        BLOCK: FoundryBlock + Default,
        N: Network,
        P: Provider<N>,
    >(
        &self,
        provider: &P,
    ) -> eyre::Result<(EvmEnv<SPEC, BLOCK>, ResolvedFork)> {
        let mut node_info_probe = self.anvil_node_info_probe();
        let (evm_env, block, context) =
            self.fork_evm_env_resolved_with_context(provider, &mut node_info_probe).await?;
        let fork_url = self.fork_url.as_deref().unwrap_or_default();
        Ok((evm_env, self.resolved_fork(fork_url, block, context)))
    }

    /// Returns the fork environment, exact block, and endpoint identity resolved together.
    async fn fork_evm_env_resolved_with_context<
        SPEC: Into<SpecId> + Default + Copy,
        BLOCK: FoundryBlock + Default,
        N: Network,
        P: Provider<N>,
    >(
        &self,
        provider: &P,
        node_info_probe: &mut AnvilNodeInfoProbe,
    ) -> eyre::Result<(EvmEnv<SPEC, BLOCK>, BlockNumHash, ForkContext)> {
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

        let target = ForkBlockTarget::Configured(match self.fork_block_number {
            Some(block_number) => BlockNumberOrTag::Number(block_number),
            None => BlockNumberOrTag::Latest,
        });
        let (block, fork_block, context) =
            self.resolve_fork_block_with_context(provider, target, node_info_probe).await?;
        let chain_id = self.chain_id_override().unwrap_or(context.execution_chain_id);
        let evm_env =
            self.fork_env_from_block::<SPEC, BLOCK, N>(chain_id, context.source_chain_id, &block);
        Ok((evm_env, fork_block, context))
    }

    fn fork_env_error_context(&self) -> String {
        let mut message = "could not instantiate forked environment".to_string();
        if let Some(fork_url) = self.fork_url.as_deref()
            && let Ok(url) = Url::parse(fork_url)
            && let Some(host) = url.host()
        {
            write!(message, " with provider {host}").unwrap();
        }
        message
    }

    async fn resolve_fork_block_with_context<N: Network, P: Provider<N>>(
        &self,
        provider: &P,
        target: ForkBlockTarget,
        node_info_probe: &mut AnvilNodeInfoProbe,
    ) -> eyre::Result<(N::BlockResponse, BlockNumHash, ForkContext)> {
        let endpoint = self.fork_url.as_deref().unwrap_or_default();
        let mut stable = None;
        for _ in 0..3 {
            let before = self
                .resolve_fork_endpoint_once(
                    provider,
                    node_info_probe,
                    EndpointHardforkPolicy::Required,
                )
                .await
                .wrap_err_with(|| self.fork_env_error_context())?;
            let block = match target {
                ForkBlockTarget::Configured(block_number) => provider
                    .get_block_by_number(block_number)
                    .await
                    .wrap_err_with(|| self.fork_env_error_context())?,
                ForkBlockTarget::Resolved(block) => provider
                    .get_block_by_hash(block.hash)
                    .await
                    .wrap_err("could not instantiate resolved fork environment")?,
            };
            let after = self
                .resolve_fork_endpoint_once(
                    provider,
                    node_info_probe,
                    EndpointHardforkPolicy::Required,
                )
                .await
                .wrap_err_with(|| self.fork_env_error_context())?;
            if before == after {
                stable = Some((before, block));
                break;
            }
        }
        let (identity, block) = stable.ok_or_else(|| {
            eyre::eyre!(
                "fork endpoint {} changed while its block and execution context were being \
                 resolved",
                fork_endpoint_description(endpoint)
            )
        })?;
        self.ensure_expected_fork_endpoint(&identity)?;
        if self.fork_network_is_inferred
            && !self.networks.has_same_execution_profile(&identity.network_profile)
        {
            eyre::bail!(
                "fork endpoint {} changed execution profile from `{}` to `{}`; rebuild the EVM \
                 for the new network",
                fork_endpoint_description(endpoint),
                self.networks.execution_profile_name(),
                identity.network_profile.execution_profile_name()
            );
        }

        let block = match block {
            Some(block) => block,
            None => match target {
                ForkBlockTarget::Configured(block_number) => {
                    let block_number_message = match block_number {
                        BlockNumberOrTag::Number(block_number) => {
                            format!("block number: {block_number}")
                        }
                        block_number => format!("{block_number} block"),
                    };
                    let latest_message = if let Ok(latest_block) = provider.get_block_number().await
                    {
                        if let Some(block_number) = self.fork_block_number
                            && block_number <= latest_block
                        {
                            error!("{NON_ARCHIVE_NODE_WARNING}");
                        }
                        format!("; latest block number: {latest_block}")
                    } else {
                        Default::default()
                    };
                    eyre::bail!("failed to get {block_number_message}{latest_message}");
                }
                ForkBlockTarget::Resolved(block) => {
                    eyre::bail!("failed to get block hash: {}", block.hash);
                }
            },
        };
        let actual = BlockNumHash::new(block.header().number(), block.header().hash());
        if let ForkBlockTarget::Resolved(expected) = target {
            eyre::ensure!(
                actual == expected,
                "resolved fork block changed: expected {expected:?}, got {actual:?}"
            );
        }

        let context = ForkContext {
            execution_chain_id: identity.execution_chain_id,
            source_chain_id: identity.source_chain_id,
            network: identity.network,
            network_profile: identity.network_profile,
            block_number: actual.number,
            hardfork: identity.hardfork,
            instance_id: identity.instance_id,
            source_fork_block_number: identity.source_fork_block_number,
            source_fork_block_hash: identity.source_fork_block_hash,
        };
        Ok((block, actual, context))
    }

    async fn fork_evm_env_at_resolved_with_context<
        SPEC: Into<SpecId> + Default + Copy,
        BLOCK: FoundryBlock + Default,
        N: Network,
        P: Provider<N>,
    >(
        &self,
        provider: &P,
        expected: &ResolvedFork,
        node_info_probe: &mut AnvilNodeInfoProbe,
    ) -> eyre::Result<(EvmEnv<SPEC, BLOCK>, ForkContext)> {
        let (block, _, context) = self
            .resolve_fork_block_with_context(
                provider,
                ForkBlockTarget::Resolved(expected.block()),
                node_info_probe,
            )
            .await?;
        eyre::ensure!(
            context == expected.context(),
            "fork endpoint {} changed after its block and execution context were resolved",
            fork_endpoint_description(self.fork_url.as_deref().unwrap_or_default())
        );
        let chain_id = self.chain_id_override().unwrap_or(context.execution_chain_id);
        let evm_env =
            self.fork_env_from_block::<SPEC, BLOCK, N>(chain_id, context.source_chain_id, &block);
        Ok((evm_env, context))
    }

    /// Reconstructs the fork environment at an already resolved exact block.
    pub(crate) async fn fork_evm_env_at_resolved<
        SPEC: Into<SpecId> + Default + Copy,
        BLOCK: FoundryBlock + Default,
        N: Network,
        P: Provider<N>,
    >(
        &self,
        provider: &P,
        expected: &ResolvedFork,
    ) -> eyre::Result<EvmEnv<SPEC, BLOCK>> {
        let mut node_info_probe = self.anvil_node_info_probe();
        node_info_probe.identified |= expected.context().hardfork.is_some();
        let (evm_env, _) = self
            .fork_evm_env_at_resolved_with_context(provider, expected, &mut node_info_probe)
            .await?;
        Ok(evm_env)
    }

    fn fork_env_from_block<
        SPEC: Into<SpecId> + Default + Copy,
        BLOCK: FoundryBlock + Default,
        N: Network,
    >(
        &self,
        chain_id: ChainId,
        source_chain_id: ChainId,
        block: &N::BlockResponse,
    ) -> EvmEnv<SPEC, BLOCK> {
        let mut evm_env = EvmEnv {
            cfg_env: self.cfg_env(chain_id),
            block_env: block_env_from_header(block.header()),
        };

        apply_chain_and_block_specific_env_changes_for_chain::<N, _, _>(
            &mut evm_env,
            block,
            source_chain_id,
            self.networks,
        );

        evm_env
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

    /// Returns the `TxEnv` with gas price and chain ID from a stable fork snapshot.
    fn fork_tx_env<TX: FoundryTransaction + Default>(
        &self,
        gas_price: u128,
        chain_id: ChainId,
    ) -> TX {
        let mut tx_env = TX::default();
        tx_env.set_caller(self.sender);
        tx_env.set_chain_id(Some(chain_id));
        tx_env.set_gas_price(gas_price);
        tx_env.set_gas_limit(self.gas_limit());
        tx_env
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
        chain_id: u64,
        fork_block_number: Option<BlockNumber>,
    ) -> Option<CreateFork> {
        self.get_fork_with_identity(config, chain_id, fork_block_number, None)
    }

    /// Returns a fork configuration pinned to an already resolved block identity.
    pub fn get_fork_resolved(
        &self,
        config: &Config,
        chain_id: u64,
        fork: Option<&ResolvedFork>,
    ) -> Option<CreateFork> {
        let fork_block_number = fork.map(ResolvedFork::number);
        let source_chain_id = fork.map(|fork| fork.context().source_chain_id).unwrap_or(chain_id);
        self.get_fork_with_identity(config, source_chain_id, fork_block_number, fork.cloned())
    }

    fn get_fork_with_identity(
        &self,
        config: &Config,
        chain_id: u64,
        fork_block_number: Option<BlockNumber>,
        resolved: Option<ResolvedFork>,
    ) -> Option<CreateFork> {
        let url = self.fork_url.clone()?;
        let enable_caching = config.enable_caching(&url, chain_id);

        // Pin fork_block_number to the block that was already fetched in env, so subsequent
        // fork operations use the same block. This prevents inconsistencies when forking at
        // "latest" where the chain could advance between calls.
        let mut evm_opts = self.clone();
        if evm_opts.fork_block_number.is_none() {
            evm_opts.fork_block_number = fork_block_number;
            evm_opts.fork_block_number_is_inferred = fork_block_number.is_some();
        }

        Some(CreateFork { url, enable_caching, evm_opts, resolved })
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

/// Describes how an execution spec should use a source chain's hardfork schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionSpecContext {
    /// Local execution without a fork.
    Local,
    /// Local execution backed by a fork.
    Fork {
        /// Underlying source chain ID.
        source_chain_id: ChainId,
        /// Exact hardfork reported by the endpoint.
        endpoint_hardfork: Option<FoundryHardfork>,
    },
    /// Historical execution whose semantics must match the source block.
    Historical {
        /// Underlying source chain ID.
        source_chain_id: ChainId,
        /// Exact hardfork reported by the endpoint.
        endpoint_hardfork: Option<FoundryHardfork>,
    },
}

impl ExecutionSpecContext {
    /// Returns a local fork execution context.
    pub const fn fork(
        source_chain_id: ChainId,
        endpoint_hardfork: Option<FoundryHardfork>,
    ) -> Self {
        Self::Fork { source_chain_id, endpoint_hardfork }
    }

    /// Returns the context for an optional local fork source.
    pub const fn local_or_fork(
        source_chain_id: Option<ChainId>,
        endpoint_hardfork: Option<FoundryHardfork>,
    ) -> Self {
        if let Some(source_chain_id) = source_chain_id {
            Self::Fork { source_chain_id, endpoint_hardfork }
        } else {
            Self::Local
        }
    }

    /// Returns a historical execution context.
    pub const fn historical(
        source_chain_id: ChainId,
        endpoint_hardfork: Option<FoundryHardfork>,
    ) -> Self {
        Self::Historical { source_chain_id, endpoint_hardfork }
    }

    fn endpoint_hardfork(self, networks: NetworkConfigs) -> Option<FoundryHardfork> {
        match self {
            Self::Historical { endpoint_hardfork, .. } => endpoint_hardfork,
            Self::Fork { endpoint_hardfork, .. } if networks.active_network_name().is_some() => {
                endpoint_hardfork
            }
            Self::Local | Self::Fork { .. } => None,
        }
    }

    fn schedule_chain_id(self, networks: NetworkConfigs) -> Option<ChainId> {
        match self {
            Self::Historical { source_chain_id, .. } => Some(source_chain_id),
            Self::Fork { source_chain_id, .. } if networks.active_network_name().is_some() => {
                Some(source_chain_id)
            }
            Self::Local | Self::Fork { .. } => None,
        }
    }
}

/// Resolves and applies the execution spec for an EVM environment.
///
/// A direct caller override takes precedence over a configured namespaced hardfork, followed by
/// exact endpoint metadata and the hardfork selected from the source schedule. Local Ethereum
/// forks retain Foundry's configured EVM version, namespaced local networks follow their source
/// identity, and historical execution always follows the source block's identity.
///
/// Returns the exact namespaced hardfork, when applicable, so execution and trace decoding can use
/// the same hardfork.
pub fn resolve_execution_spec<SPEC, BLOCK>(
    config: &Config,
    networks: NetworkConfigs,
    evm_env: &mut EvmEnv<SPEC, BLOCK>,
    context: ExecutionSpecContext,
    explicit_spec: Option<SPEC>,
    explicit_hardfork: Option<FoundryHardfork>,
) -> Option<FoundryHardfork>
where
    SPEC: ExecutionSpec + Into<SpecId> + Copy,
    BLOCK: FoundryBlock,
{
    let supports = |hardfork| SPEC::from_foundry_hardfork(hardfork).is_some();
    let configured_hardfork = config.hardfork.filter(|&hardfork| supports(hardfork));
    let endpoint_hardfork =
        context.endpoint_hardfork(networks).filter(|&hardfork| supports(hardfork));
    let timestamp_hardfork = context
        .schedule_chain_id(networks)
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
        let hardfork = networks.is_monad().then(|| {
            FoundryHardfork::Monad(config.evm_spec_id::<foundry_evm_hardforks::MonadHardfork>())
        });
        #[cfg(not(feature = "monad"))]
        let hardfork = None;
        #[cfg(feature = "base")]
        let hardfork = hardfork.or_else(|| {
            networks.is_base().then(|| {
                FoundryHardfork::Base(
                    config.evm_spec_id::<foundry_evm_hardforks::BaseSpecId>().upgrade(),
                )
            })
        });
        hardfork
    };

    let resolved_hardfork = if explicit_spec.is_some() {
        explicit_hardfork
    } else {
        configured_hardfork.or(endpoint_hardfork).or(timestamp_hardfork).or(fallback_hardfork)
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

fn fork_endpoint_description(endpoint: &str) -> String {
    Url::parse(endpoint)
        .ok()
        .and_then(|url| url.host_str().map(|host| format!("provider {host}")))
        .unwrap_or_else(|| "configured provider".to_string())
}

async fn option_try_or_else<T, E>(
    option: Option<T>,
    f: impl AsyncFnOnce() -> Result<T, E>,
) -> Result<T, E> {
    if let Some(value) = option { Ok(value) } else { f().await }
}

#[cfg(test)]
mod tests {
    use alloy_network::TransactionBuilder;
    use alloy_primitives::bytes;
    use alloy_rpc_types::TransactionRequest;
    use alloy_serde::WithOtherFields;
    #[cfg(feature = "base")]
    use foundry_evm_hardforks::BaseUpgrade;
    use foundry_test_utils::rpc::{
        spawn_rpc_proxy_internal_error_after, spawn_rpc_proxy_method_not_found_before,
        spawn_rpc_proxy_rejecting_method_after,
    };
    #[cfg(feature = "optimism")]
    use op_revm::OpSpecId;
    use revm::context::{BlockEnv, TxEnv};

    use super::*;

    fn resolved_context(block_number: BlockNumber) -> ForkContext {
        ForkContext {
            execution_chain_id: 1,
            source_chain_id: 1,
            network: NetworkVariant::Ethereum,
            network_profile: NetworkConfigs::default(),
            block_number,
            hardfork: None,
            instance_id: None,
            source_fork_block_number: None,
            source_fork_block_hash: None,
        }
    }

    #[test]
    fn fork_endpoint_description_redacts_credentials_and_paths() {
        let endpoint = "https://user:secret@example.com/v2/private-api-key?token=another-secret";
        let description = fork_endpoint_description(endpoint);

        assert_eq!(description, "provider example.com");
        assert!(!description.contains("secret"));
        assert!(!description.contains("private-api-key"));
    }

    #[test]
    fn endpoint_network_fallback_preserves_explicit_selection() {
        let default = EvmOpts::default().endpoint_network_fallback();
        assert!(!default.has_network_selection());

        let explicit = EvmOpts { networks: NetworkConfigs::with_ethereum(), ..Default::default() }
            .endpoint_network_fallback();
        assert!(explicit.has_network_selection());
        assert_eq!(explicit.execution_network(), NetworkVariant::Ethereum);
    }

    #[test]
    fn backend_target_matches_number_pinned_fork_identity() {
        let context = ForkContext {
            execution_chain_id: 1,
            source_chain_id: 1,
            network: NetworkVariant::Ethereum,
            network_profile: NetworkConfigs::default(),
            block_number: 10,
            hardfork: None,
            instance_id: None,
            source_fork_block_number: None,
            source_fork_block_hash: None,
        };

        let mut different_number = context;
        different_number.block_number += 1;
        assert!(!context.has_same_backend_target(different_number));

        let mut different_profile = context;
        different_profile.network_profile = NetworkConfigs::with_celo();
        assert!(!context.has_same_backend_target(different_profile));

        let mut different_hardfork = context;
        different_hardfork.hardfork =
            Some(FoundryHardfork::Ethereum(foundry_evm_hardforks::EthereumHardfork::Prague));
        assert!(!context.has_same_backend_target(different_hardfork));

        let mut different_instance = context;
        different_instance.instance_id = Some(B256::with_last_byte(3));
        assert!(!context.has_same_backend_target(different_instance));
    }

    #[tokio::test]
    async fn expected_fork_endpoint_requires_exact_identity() {
        let identity = ForkEndpointIdentity {
            endpoint: "http://localhost:8545".to_string(),
            execution_chain_id: 1,
            source_chain_id: 1,
            network: NetworkVariant::Ethereum,
            network_profile: NetworkConfigs::default(),
            reported_hardfork: Some("Prague".to_string()),
            hardfork: Some(FoundryHardfork::Ethereum(
                foundry_evm_hardforks::EthereumHardfork::Prague,
            )),
            instance_id: Some(B256::with_last_byte(1)),
            source_fork_block_number: None,
            source_fork_block_hash: None,
        };
        let mut evm_opts = EvmOpts::default();
        evm_opts.expect_fork_endpoint(identity.clone(), true);

        assert!(evm_opts.ensure_expected_fork_endpoint(&identity).is_ok());
        assert_eq!(evm_opts.fork_endpoint, Some(identity.clone()));
        assert_eq!(evm_opts.expected_fork_endpoint, Some(identity.clone()));
        assert!(evm_opts.fork_network_is_inferred);

        let mut changed_instance = identity.clone();
        changed_instance.instance_id = Some(B256::with_last_byte(2));
        assert!(evm_opts.ensure_expected_fork_endpoint(&changed_instance).is_err());

        let mut changed_unknown_hardfork = identity;
        changed_unknown_hardfork.reported_hardfork = Some("Future".to_string());
        changed_unknown_hardfork.hardfork = None;
        assert!(evm_opts.ensure_expected_fork_endpoint(&changed_unknown_hardfork).is_err());

        evm_opts.infer_network_from_fork().await.unwrap();
        assert_eq!(evm_opts.fork_endpoint, None);
        assert_eq!(evm_opts.expected_fork_endpoint, None);
    }

    #[test]
    fn fork_expected_anvil_identity_seeds_strict_node_info_probe() {
        let authoritative = ForkEndpointIdentity {
            endpoint: "http://localhost:8545".to_string(),
            execution_chain_id: 1,
            source_chain_id: 1,
            network: NetworkVariant::Ethereum,
            network_profile: NetworkConfigs::default(),
            reported_hardfork: Some("Prague".to_string()),
            hardfork: Some(FoundryHardfork::Ethereum(
                foundry_evm_hardforks::EthereumHardfork::Prague,
            )),
            instance_id: Some(B256::with_last_byte(1)),
            source_fork_block_number: None,
            source_fork_block_hash: None,
        };
        let mut anonymous = authoritative.clone();
        anonymous.reported_hardfork = None;
        anonymous.hardfork = None;
        anonymous.instance_id = None;
        let mut evm_opts = EvmOpts {
            fork_url: Some(authoritative.endpoint.clone()),
            fork_endpoint: Some(anonymous),
            expected_fork_endpoint: Some(authoritative),
            ..Default::default()
        };

        assert!(evm_opts.anvil_node_info_probe().identified);

        evm_opts.fork_url = Some("http://localhost:9545".to_string());
        assert!(!evm_opts.anvil_node_info_probe().identified);
    }

    #[test]
    fn selecting_new_fork_url_clears_only_endpoint_derived_state() {
        let identity = ForkEndpointIdentity {
            endpoint: "http://localhost:8545".to_string(),
            execution_chain_id: 42,
            source_chain_id: 42,
            network: NetworkVariant::Ethereum,
            network_profile: NetworkConfigs::with_celo(),
            reported_hardfork: None,
            hardfork: None,
            instance_id: None,
            source_fork_block_number: None,
            source_fork_block_hash: None,
        };
        let mut inferred = EvmOpts {
            fork_url: Some(identity.endpoint.clone()),
            fork_block_number: Some(123),
            fork_endpoint: Some(identity.clone()),
            expected_fork_endpoint: Some(identity.clone()),
            networks: identity.network_profile,
            fork_network_is_inferred: true,
            fork_chain_id_is_inferred: true,
            fork_block_number_is_inferred: true,
            ..Default::default()
        };
        inferred.env.chain_id = Some(identity.execution_chain_id);

        inferred.set_fork_url("http://localhost:9545".to_string());

        assert_eq!(inferred.fork_url.as_deref(), Some("http://localhost:9545"));
        assert_eq!(inferred.fork_block_number, None);
        assert_eq!(inferred.fork_endpoint, None);
        assert_eq!(inferred.expected_fork_endpoint, None);
        assert_eq!(inferred.networks, NetworkConfigs::default());
        assert_eq!(inferred.env.chain_id, None);
        assert!(!inferred.fork_network_is_inferred);
        assert!(!inferred.fork_chain_id_is_inferred);
        assert!(!inferred.fork_block_number_is_inferred);

        let mut explicit = EvmOpts {
            fork_url: Some(identity.endpoint.clone()),
            fork_block_number: Some(123),
            fork_endpoint: Some(identity.clone()),
            expected_fork_endpoint: Some(identity),
            networks: NetworkConfigs::with_celo(),
            ..Default::default()
        };
        explicit.env.chain_id = Some(42);

        explicit.set_fork_url("http://localhost:9545".to_string());

        assert_eq!(explicit.networks, NetworkConfigs::with_celo());
        assert_eq!(explicit.env.chain_id, Some(42));
        assert_eq!(explicit.fork_block_number, Some(123));
        assert_eq!(explicit.fork_endpoint, None);
        assert_eq!(explicit.expected_fork_endpoint, None);
        assert!(!explicit.fork_block_number_is_inferred);
    }

    #[test]
    fn changed_fork_auth_clears_only_endpoint_derived_state() {
        let endpoint = "http://localhost:8545";
        let identity = ForkEndpointIdentity {
            endpoint: endpoint.to_string(),
            execution_chain_id: 42,
            source_chain_id: 42,
            network: NetworkVariant::Ethereum,
            network_profile: NetworkConfigs::with_celo(),
            reported_hardfork: None,
            hardfork: None,
            instance_id: Some(B256::with_last_byte(1)),
            source_fork_block_number: None,
            source_fork_block_hash: None,
        };
        let context = ForkContext {
            execution_chain_id: identity.execution_chain_id,
            source_chain_id: identity.source_chain_id,
            network: identity.network,
            network_profile: identity.network_profile,
            block_number: 123,
            hardfork: identity.hardfork,
            instance_id: identity.instance_id,
            source_fork_block_number: identity.source_fork_block_number,
            source_fork_block_hash: identity.source_fork_block_hash,
        };
        let mut evm_opts = EvmOpts {
            fork_url: Some(endpoint.to_string()),
            fork_block_number: Some(123),
            fork_headers: Some(vec!["Authorization: first".to_string()]),
            rpc_jwt: Some("first-jwt".to_string()),
            fork_endpoint: Some(identity.clone()),
            expected_fork_endpoint: Some(identity),
            networks: NetworkConfigs::with_celo(),
            ..Default::default()
        };
        evm_opts.env.chain_id = Some(42);
        let fork = evm_opts.resolved_fork(
            endpoint,
            BlockNumHash::new(context.block_number, B256::with_last_byte(2)),
            context,
        );

        evm_opts.fork_headers = Some(vec!["Authorization: second".to_string()]);
        evm_opts.rpc_jwt = Some("second-jwt".to_string());

        assert!(evm_opts.invalidate_fork_endpoint_if_source_changed(&fork));
        assert_eq!(evm_opts.fork_endpoint, None);
        assert_eq!(evm_opts.expected_fork_endpoint, None);
        assert_eq!(evm_opts.fork_block_number, Some(123));
        assert_eq!(evm_opts.networks, NetworkConfigs::with_celo());
        assert_eq!(evm_opts.env.chain_id, Some(42));
    }

    #[test]
    fn changed_fork_selector_preserves_endpoint_identity() {
        let endpoint = "http://localhost:8545";
        let identity = ForkEndpointIdentity {
            endpoint: endpoint.to_string(),
            execution_chain_id: 1,
            source_chain_id: 1,
            network: NetworkVariant::Ethereum,
            network_profile: NetworkConfigs::default(),
            reported_hardfork: None,
            hardfork: None,
            instance_id: Some(B256::with_last_byte(1)),
            source_fork_block_number: None,
            source_fork_block_hash: None,
        };
        let mut evm_opts = EvmOpts {
            fork_url: Some(endpoint.to_string()),
            fork_headers: Some(vec!["x-source: stable".to_string()]),
            rpc_jwt: Some("stable-jwt".to_string()),
            fork_endpoint: Some(identity.clone()),
            expected_fork_endpoint: Some(identity.clone()),
            ..Default::default()
        };
        let fork = evm_opts.resolved_fork(
            endpoint,
            BlockNumHash::new(1, B256::with_last_byte(2)),
            resolved_context(1),
        );

        evm_opts.fork_block_number = Some(1);

        assert!(!evm_opts.invalidate_fork_endpoint_if_source_changed(&fork));
        assert_eq!(evm_opts.fork_endpoint, Some(identity.clone()));
        assert_eq!(evm_opts.expected_fork_endpoint, Some(identity));
    }

    #[test]
    fn selecting_same_fork_url_preserves_pinned_state() {
        let mut evm_opts = EvmOpts {
            fork_url: Some("http://localhost:8545".to_string()),
            fork_block_number: Some(123),
            networks: NetworkConfigs::with_celo(),
            fork_network_is_inferred: true,
            fork_chain_id_is_inferred: true,
            fork_block_number_is_inferred: true,
            ..Default::default()
        };
        evm_opts.env.chain_id = Some(42);

        evm_opts.set_fork_url("http://localhost:8545".to_string());

        assert_eq!(evm_opts.fork_block_number, Some(123));
        assert_eq!(evm_opts.networks, NetworkConfigs::with_celo());
        assert_eq!(evm_opts.env.chain_id, Some(42));
        assert!(evm_opts.fork_network_is_inferred);
        assert!(evm_opts.fork_chain_id_is_inferred);
        assert!(evm_opts.fork_block_number_is_inferred);
    }

    #[cfg(feature = "monad")]
    fn monad_env(timestamp: u64) -> EvmEnv<foundry_evm_hardforks::MonadHardfork, BlockEnv> {
        let mut block = BlockEnv::default();
        block.set_timestamp(U256::from(timestamp));
        let mut cfg = CfgEnv::new_with_spec(foundry_evm_hardforks::MonadHardfork::default());
        cfg.chain_id = NamedChain::Monad as u64;
        EvmEnv::new(cfg, block)
    }

    #[test]
    #[cfg(feature = "monad")]
    fn resolve_execution_spec_uses_monad_ten_activation_timestamp() {
        let config = Config::default();
        let networks = NetworkConfigs::with_monad();
        let activation =
            foundry_evm_hardforks::MonadHardfork::MonadTen.mainnet_activation_timestamp().unwrap();

        let mut before = monad_env(activation - 1);
        assert_eq!(
            resolve_execution_spec(
                &config,
                networks,
                &mut before,
                ExecutionSpecContext::fork(NamedChain::Monad as u64, None),
                None,
                None,
            ),
            Some(FoundryHardfork::Monad(foundry_evm_hardforks::MonadHardfork::MonadNine))
        );
        assert_eq!(before.cfg_env.spec, foundry_evm_hardforks::MonadHardfork::MonadNine);

        let mut after = monad_env(activation);
        assert_eq!(
            resolve_execution_spec(
                &config,
                networks,
                &mut after,
                ExecutionSpecContext::fork(NamedChain::Monad as u64, None),
                None,
                None,
            ),
            Some(FoundryHardfork::Monad(foundry_evm_hardforks::MonadHardfork::MonadTen))
        );
        assert_eq!(after.cfg_env.spec, foundry_evm_hardforks::MonadHardfork::MonadTen);
    }

    #[test]
    #[cfg(feature = "monad")]
    fn resolve_execution_spec_prefers_exact_endpoint_hardfork() {
        let config = Config::default();
        let activation =
            foundry_evm_hardforks::MonadHardfork::MonadNine.mainnet_activation_timestamp().unwrap();
        let mut env = monad_env(activation);
        let endpoint_hardfork =
            FoundryHardfork::Monad(foundry_evm_hardforks::MonadHardfork::MonadEight);

        assert_eq!(
            resolve_execution_spec(
                &config,
                NetworkConfigs::with_monad(),
                &mut env,
                ExecutionSpecContext::fork(NamedChain::Monad as u64, Some(endpoint_hardfork),),
                None,
                None,
            ),
            Some(endpoint_hardfork)
        );
        assert_eq!(env.cfg_env.spec, foundry_evm_hardforks::MonadHardfork::MonadEight);
    }

    #[test]
    #[cfg(feature = "monad")]
    fn resolve_execution_spec_ignores_schedule_for_local_env() {
        let config = Config::default();
        let networks = NetworkConfigs::with_monad();
        let activation =
            foundry_evm_hardforks::MonadHardfork::MonadTen.mainnet_activation_timestamp().unwrap();
        let mut env = monad_env(activation - 1);

        assert_eq!(
            resolve_execution_spec(
                &config,
                networks,
                &mut env,
                ExecutionSpecContext::Local,
                None,
                None,
            ),
            Some(FoundryHardfork::Monad(foundry_evm_hardforks::MonadHardfork::MonadTen))
        );
        assert_eq!(env.cfg_env.spec, foundry_evm_hardforks::MonadHardfork::MonadTen);
    }

    #[test]
    fn resolve_execution_spec_preserves_ethereum_config_for_local_forks() {
        let config = Config::default();
        let mut block = BlockEnv::default();
        block.set_timestamp(U256::from(1_500_000_000u64));
        let mut env = EvmEnv::new(CfgEnv::new_with_spec(SpecId::LONDON), block);

        assert_eq!(
            resolve_execution_spec(
                &config,
                NetworkConfigs::default(),
                &mut env,
                ExecutionSpecContext::fork(NamedChain::Mainnet as u64, None),
                None,
                None,
            ),
            None
        );
        assert_eq!(env.cfg_env.spec, config.evm_spec_id::<SpecId>());

        let endpoint_hardfork =
            FoundryHardfork::Ethereum(foundry_evm_hardforks::EthereumHardfork::Frontier);
        assert_eq!(
            resolve_execution_spec(
                &config,
                NetworkConfigs::default(),
                &mut env,
                ExecutionSpecContext::fork(NamedChain::Mainnet as u64, Some(endpoint_hardfork),),
                None,
                None,
            ),
            None
        );
        assert_eq!(env.cfg_env.spec, config.evm_spec_id::<SpecId>());
    }

    #[test]
    fn resolve_execution_spec_uses_ethereum_schedule_for_historical_execution() {
        let config = Config::default();
        let timestamp = 1_500_000_000u64;
        let expected =
            FoundryHardfork::from_chain_and_timestamp(NamedChain::Mainnet as u64, timestamp)
                .unwrap();
        let mut block = BlockEnv::default();
        block.set_timestamp(U256::from(timestamp));
        let mut env = EvmEnv::new(CfgEnv::new_with_spec(SpecId::LONDON), block);

        assert_eq!(
            resolve_execution_spec(
                &config,
                NetworkConfigs::default(),
                &mut env,
                ExecutionSpecContext::historical(NamedChain::Mainnet as u64, None),
                None,
                None,
            ),
            Some(expected)
        );
        assert_eq!(env.cfg_env.spec, SpecId::from(expected));
    }

    #[test]
    #[cfg(feature = "optimism")]
    fn resolve_execution_spec_uses_optimism_schedule_for_local_forks() {
        let config = Config::default();
        let chain_id = NamedChain::Optimism as u64;
        let timestamp = u64::MAX;
        let expected = FoundryHardfork::from_chain_and_timestamp(chain_id, timestamp).unwrap();
        let mut block = BlockEnv::default();
        block.set_timestamp(U256::from(timestamp));
        let mut env = EvmEnv::new(CfgEnv::new_with_spec(OpSpecId::default()), block);

        assert_eq!(
            resolve_execution_spec(
                &config,
                NetworkConfigs::with_optimism(),
                &mut env,
                ExecutionSpecContext::fork(chain_id, None),
                None,
                None,
            ),
            Some(expected)
        );
        assert_eq!(env.cfg_env.spec, OpSpecId::from_foundry_hardfork(expected).unwrap());
    }

    #[test]
    #[cfg(feature = "monad")]
    fn resolve_execution_spec_honors_explicit_precedence() {
        let networks = NetworkConfigs::with_monad();
        let activation =
            foundry_evm_hardforks::MonadHardfork::MonadNine.mainnet_activation_timestamp().unwrap();
        let mut configured = Config {
            hardfork: Some(FoundryHardfork::Monad(foundry_evm_hardforks::MonadHardfork::MonadNine)),
            ..Default::default()
        };
        let mut env = monad_env(activation - 1);

        assert_eq!(
            resolve_execution_spec(
                &configured,
                networks,
                &mut env,
                ExecutionSpecContext::fork(NamedChain::Monad as u64, None),
                None,
                None,
            ),
            configured.hardfork
        );
        assert_eq!(env.cfg_env.spec, foundry_evm_hardforks::MonadHardfork::MonadNine);

        configured.hardfork = None;
        assert_eq!(
            resolve_execution_spec(
                &configured,
                networks,
                &mut env,
                ExecutionSpecContext::fork(NamedChain::Monad as u64, None),
                Some(foundry_evm_hardforks::MonadHardfork::MonadEight),
                Some(FoundryHardfork::Monad(foundry_evm_hardforks::MonadHardfork::MonadEight)),
            ),
            Some(FoundryHardfork::Monad(foundry_evm_hardforks::MonadHardfork::MonadEight))
        );
        assert_eq!(env.cfg_env.spec, foundry_evm_hardforks::MonadHardfork::MonadEight);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg(feature = "monad")]
    async fn fork_context_preserves_source_chain_with_execution_override() {
        let activation =
            foundry_evm_hardforks::MonadHardfork::MonadNine.mainnet_activation_timestamp().unwrap();
        let (_api, handle) = anvil::spawn(
            anvil::NodeConfig::test()
                .with_networks(NetworkConfigs::with_ethereum())
                .with_chain_id(Some(NamedChain::Monad as u64))
                .with_genesis_timestamp(Some(activation - 1)),
        )
        .await;
        let mut inferred = EvmOpts { fork_url: Some(handle.http_endpoint()), ..Default::default() };
        inferred.infer_network_from_fork().await.unwrap();
        assert_eq!(inferred.networks, NetworkConfigs::default());

        let mut evm_opts = EvmOpts { fork_url: Some(handle.http_endpoint()), ..Default::default() };
        evm_opts.env.chain_id = Some(NamedChain::Mainnet as u64);
        evm_opts.networks = NetworkConfigs::with_monad();

        let (mut evm_env, tx_env, fork_context) = evm_opts
            .env_with_fork_context::<foundry_evm_hardforks::MonadHardfork, BlockEnv, TxEnv>()
            .await
            .unwrap();
        let fork_context = fork_context.unwrap();

        assert_eq!(fork_context.source_chain_id, NamedChain::Monad as u64);
        assert_eq!(fork_context.network, NetworkVariant::Ethereum);
        assert!(matches!(fork_context.hardfork, Some(FoundryHardfork::Ethereum(_))));
        assert_eq!(evm_env.cfg_env.chain_id, NamedChain::Mainnet as u64);
        assert_eq!(tx_env.chain_id, Some(NamedChain::Mainnet as u64));
        assert_eq!(
            resolve_execution_spec(
                &Config::default(),
                NetworkConfigs::with_monad(),
                &mut evm_env,
                ExecutionSpecContext::fork(fork_context.source_chain_id, fork_context.hardfork,),
                None,
                None,
            ),
            Some(FoundryHardfork::Monad(foundry_evm_hardforks::MonadHardfork::MonadEight))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn infer_network_default_and_custom_anvil_selects_ethereum() {
        for chain_id in [NamedChain::Mainnet as u64, NamedChain::AnvilHardhat as u64, 98_765_432] {
            let (_api, handle) =
                anvil::spawn(anvil::NodeConfig::test().with_chain_id(Some(chain_id))).await;

            let config = Config::figment();
            let mut evm_opts = config.extract::<EvmOpts>().unwrap();
            evm_opts.fork_url = Some(handle.http_endpoint());
            assert_eq!(evm_opts.networks, NetworkConfigs::default());

            evm_opts.infer_network_from_fork().await.unwrap();

            assert_eq!(evm_opts.env.chain_id, Some(chain_id));
            assert!(!evm_opts.networks.is_tempo());
            #[cfg(feature = "optimism")]
            assert!(!evm_opts.networks.is_optimism());
            #[cfg(feature = "base")]
            assert!(!evm_opts.networks.is_base());
            assert!(!evm_opts.networks.is_celo());
            assert_eq!(evm_opts.networks, NetworkConfigs::default());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn infer_network_preserves_explicit_ethereum() {
        let networks = NetworkConfigs::with_ethereum();
        let mut evm_opts = EvmOpts {
            fork_url: Some("http://127.0.0.1:1".to_string()),
            networks,
            ..Default::default()
        };

        let error = evm_opts.infer_network_from_fork().await.unwrap_err();

        assert!(error.to_string().contains("failed to retrieve chain ID"));
        assert_eq!(evm_opts.networks, networks);
        assert_eq!(evm_opts.env.chain_id, None);
    }

    #[tokio::test]
    async fn clearing_fork_restores_inferred_network_defaults() {
        let mut profiles = vec![NetworkConfigs::with_celo(), NetworkConfigs::with_tempo()];
        #[cfg(feature = "optimism")]
        profiles.push(NetworkConfigs::with_optimism());
        #[cfg(feature = "monad")]
        profiles.push(NetworkConfigs::with_monad());
        #[cfg(feature = "base")]
        profiles.push(NetworkConfigs::with_base());

        for networks in profiles {
            let mut evm_opts = EvmOpts {
                networks,
                fork_endpoint: Some(ForkEndpointIdentity {
                    endpoint: "http://localhost:8545".to_string(),
                    execution_chain_id: 1,
                    source_chain_id: 1,
                    network: networks.execution_network(),
                    network_profile: networks,
                    reported_hardfork: None,
                    hardfork: None,
                    instance_id: None,
                    source_fork_block_number: None,
                    source_fork_block_hash: None,
                }),
                fork_block_number: Some(123),
                fork_network_is_inferred: true,
                fork_chain_id_is_inferred: true,
                fork_block_number_is_inferred: true,
                ..Default::default()
            };
            evm_opts.env.chain_id = Some(1);

            evm_opts.infer_network_from_fork().await.unwrap();

            assert_eq!(evm_opts.networks, NetworkConfigs::default());
            assert_eq!(evm_opts.fork_endpoint, None);
            assert!(!evm_opts.fork_network_is_inferred);
            assert_eq!(evm_opts.env.chain_id, None);
            assert!(!evm_opts.fork_chain_id_is_inferred);
            assert_eq!(evm_opts.fork_block_number, None);
            assert!(!evm_opts.fork_block_number_is_inferred);
        }
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

    #[tokio::test]
    #[cfg(feature = "base")]
    async fn base_endpoint_identity_uses_generic_chain_profile() {
        let endpoint = "http://127.0.0.1:1";
        let provider = EvmOpts::default().fork_provider_with_url::<AnyNetwork>(endpoint).unwrap();

        let identity = EvmOpts::resolve_fork_endpoint_identity(
            &provider,
            endpoint,
            NamedChain::Base as u64,
            None,
            None,
            EndpointHardforkPolicy::Optional,
        )
        .await
        .unwrap();

        assert_eq!(identity.execution_chain_id, NamedChain::Base as u64);
        assert_eq!(identity.source_chain_id, NamedChain::Base as u64);
        assert_eq!(identity.network, NetworkVariant::Base);
        assert!(identity.network_profile.is_base());
        assert_eq!(identity.reported_hardfork, None);
        assert_eq!(identity.hardfork, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg(feature = "base")]
    async fn base_anvil_identity_uses_generic_network_and_hardfork_parsing() {
        let (_api, handle) = anvil::spawn(
            anvil::NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Beryl.into())),
        )
        .await;
        let endpoint = handle.http_endpoint();
        let provider = EvmOpts::default().fork_provider_with_url::<AnyNetwork>(&endpoint).unwrap();
        let execution_chain_id = provider.get_chain_id().await.unwrap();
        let node_info =
            provider.raw_request::<_, NodeInfo>("anvil_nodeInfo".into(), ()).await.unwrap();
        assert_eq!(node_info.network.as_deref(), Some("base"));
        assert_eq!(node_info.hard_fork, "Beryl");

        let identity = EvmOpts::resolve_fork_endpoint_identity(
            &provider,
            &endpoint,
            execution_chain_id,
            Some(node_info),
            None,
            EndpointHardforkPolicy::Required,
        )
        .await
        .unwrap();

        assert_eq!(identity.network, NetworkVariant::Base);
        assert!(identity.network_profile.is_base());
        assert_eq!(identity.reported_hardfork.as_deref(), Some("Beryl"));
        assert_eq!(identity.hardfork, Some(FoundryHardfork::Base(BaseUpgrade::Beryl)));
        assert!(identity.instance_id.is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_non_anvil_node_info_rpc_error_is_optional() {
        let (_api, handle) =
            anvil::spawn(anvil::NodeConfig::test().with_chain_id(Some(NamedChain::Mainnet as u64)))
                .await;
        let fork_url =
            spawn_rpc_proxy_internal_error_after(handle.http_endpoint(), "anvil_nodeInfo", 0).await;
        let mut evm_opts = EvmOpts { fork_url: Some(fork_url), ..Default::default() };

        evm_opts.infer_network_from_fork().await.unwrap();

        assert_eq!(evm_opts.networks, NetworkConfigs::default());
        assert_eq!(evm_opts.env.chain_id, Some(NamedChain::Mainnet as u64));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_node_info_failure_is_strict_after_anvil_is_identified() {
        let (_api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
        let fork_url =
            spawn_rpc_proxy_rejecting_method_after(handle.http_endpoint(), "anvil_nodeInfo", 1)
                .await;
        let evm_opts = EvmOpts { fork_url: Some(fork_url), ..Default::default() };

        let error = evm_opts.discover_fork_endpoint().await.unwrap_err();

        assert!(
            error.to_string().contains("failed to determine network family from endpoint"),
            "{error}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_node_info_probe_becomes_strict_after_initial_failure() {
        let (_api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
        let fork_url =
            spawn_rpc_proxy_method_not_found_before(handle.http_endpoint(), "anvil_nodeInfo", 1)
                .await;
        let evm_opts = EvmOpts { fork_url: Some(fork_url), ..Default::default() };

        let identity = evm_opts.discover_fork_endpoint().await.unwrap();

        assert!(identity.reported_hardfork.is_some());
        assert!(identity.instance_id.is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_cached_anvil_identity_keeps_node_info_probe_strict() {
        let (_api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
        let fork_url =
            spawn_rpc_proxy_rejecting_method_after(handle.http_endpoint(), "anvil_nodeInfo", 2)
                .await;
        let mut evm_opts = EvmOpts { fork_url: Some(fork_url), ..Default::default() };
        evm_opts.infer_network_from_fork().await.unwrap();
        assert!(evm_opts.fork_endpoint.as_ref().unwrap().reported_hardfork.is_some());

        let error = evm_opts.fork_network().await.unwrap_err();

        assert!(
            error.to_string().contains("failed to determine network family from endpoint"),
            "{error}"
        );
    }

    #[test]
    fn known_network_variant_does_not_guess_unknown_chain() {
        assert_eq!(NetworkVariant::from_known_chain_id(98_765_432).unwrap(), None);
    }

    #[test]
    #[cfg(feature = "monad")]
    fn known_network_variant_classifies_monad() {
        assert_eq!(
            NetworkVariant::from_known_chain_id(NamedChain::Monad as u64).unwrap(),
            Some(NetworkVariant::Monad)
        );
    }

    #[test]
    #[cfg(not(feature = "monad"))]
    fn known_network_variant_rejects_disabled_monad() {
        assert_eq!(
            NetworkVariant::from_known_chain_id(NamedChain::Monad as u64).unwrap_err(),
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

        evm_opts.infer_network_from_fork().await.unwrap();

        assert!(evm_opts.networks.is_tempo(), "should detect tempo via anvil_nodeInfo");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn infer_network_tempo_propagates_unavailable_rpc() {
        let config = Config::figment();
        let mut evm_opts = config.extract::<EvmOpts>().unwrap();
        evm_opts.fork_url = Some("http://127.0.0.1:1".to_string());
        evm_opts.networks = NetworkConfigs::with_tempo();

        let error = evm_opts.infer_network_from_fork().await.unwrap_err();

        assert!(error.to_string().contains("failed to retrieve chain ID"));
        assert!(evm_opts.networks.is_tempo());
    }

    #[tokio::test]
    async fn create2_deployer_availability_requires_resolved_fork_block() {
        let evm_opts =
            EvmOpts { fork_url: Some("http://127.0.0.1:1".to_string()), ..Default::default() };

        let err = evm_opts.can_use_create2_deployer(None).await.unwrap_err();
        assert!(err.to_string().contains("fork block must be resolved"));
    }

    #[test]
    fn resolved_fork_matches_source_headers_and_selector() {
        let headers = vec!["Authorization: one".to_string()];
        let fork = ResolvedFork::new(
            "http://127.0.0.1:1",
            Some(&headers),
            None,
            None,
            BlockNumHash::new(1, B256::ZERO),
            resolved_context(1),
        );
        let mut evm_opts = EvmOpts {
            fork_url: Some("http://127.0.0.1:1".to_string()),
            fork_headers: Some(headers),
            rpc_headers: Some(vec!["x-fallback: ignored".to_string()]),
            ..Default::default()
        };

        assert!(evm_opts.resolved_fork_matches(&fork));
        let debug = format!("{fork:?}");
        assert!(!debug.contains("127.0.0.1"));
        assert!(!debug.contains("Authorization"));
        assert!(!debug.contains("one"));

        evm_opts.fork_headers = Some(vec!["Authorization: two".to_string()]);
        assert!(!evm_opts.resolved_fork_matches(&fork));

        evm_opts.fork_headers = Some(vec!["Authorization: one".to_string()]);
        evm_opts.fork_block_number = Some(1);
        assert!(!evm_opts.resolved_fork_matches(&fork));

        evm_opts.fork_block_number = None;
        evm_opts.fork_url = Some("http://127.0.0.1:2".to_string());
        assert!(!evm_opts.resolved_fork_matches(&fork));

        let fork = ResolvedFork::new(
            "http://127.0.0.1:1",
            None,
            None,
            None,
            BlockNumHash::new(1, B256::ZERO),
            resolved_context(1),
        );
        evm_opts.fork_url = Some("http://127.0.0.1:1".to_string());
        evm_opts.fork_headers = Some(Vec::new());
        evm_opts.rpc_headers = None;
        assert!(evm_opts.resolved_fork_matches(&fork));

        let rpc_headers = vec!["Authorization: fallback".to_string()];
        let fork = ResolvedFork::new(
            "http://127.0.0.1:1",
            Some(&rpc_headers),
            Some("secret-jwt"),
            None,
            BlockNumHash::new(1, B256::ZERO),
            resolved_context(1),
        );
        evm_opts.fork_headers = None;
        evm_opts.rpc_headers = Some(rpc_headers);
        evm_opts.rpc_jwt = Some("secret-jwt".to_string());
        assert!(evm_opts.resolved_fork_matches(&fork));
        assert!(!format!("{fork:?}").contains("secret-jwt"));

        evm_opts.rpc_jwt = Some("different-jwt".to_string());
        assert!(!evm_opts.resolved_fork_matches(&fork));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create2_deployer_preflight_uses_exact_fork_hash() {
        let (_api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
        let provider = handle.http_provider();
        let sender = handle.dev_accounts().next().unwrap();

        let init_code = bytes!("6001600c60003960016000f300");
        let receipt = provider
            .send_transaction(WithOtherFields::new(
                TransactionRequest::default().from(sender).with_deploy_code(init_code),
            ))
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        let deployer = receipt.contract_address.unwrap();
        let block_number = receipt.block_number.unwrap();

        let evm_opts = EvmOpts {
            fork_url: Some(handle.http_endpoint()),
            fork_block_number: Some(block_number),
            create2_deployer: deployer,
            ..Default::default()
        };
        let (_, _, fork) = evm_opts.env_resolved::<SpecId, BlockEnv, TxEnv>().await.unwrap();
        let fork = fork.unwrap();
        assert!(evm_opts.can_use_create2_deployer_resolved(Some(&fork)).await.unwrap());

        provider
            .raw_request::<_, ()>("anvil_reorg".into(), (1_u64, Vec::<serde_json::Value>::new()))
            .await
            .unwrap();
        let replacement = provider.get_block_by_number(block_number.into()).await.unwrap().unwrap();
        assert_ne!(replacement.header().hash(), fork.hash());
        assert!(
            !evm_opts.can_use_create2_deployer(Some(block_number)).await.unwrap(),
            "the replacement block must not contain the deployer"
        );

        assert!(
            !matches!(evm_opts.can_use_create2_deployer_resolved(Some(&fork)).await, Ok(false)),
            "the exact lookup fell back to the replacement block"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg(feature = "monad")]
    async fn infer_network_monad_anvil_via_node_info() {
        for chain_id in [NamedChain::Mainnet as u64, NamedChain::AnvilHardhat as u64, 98_765_432] {
            let (_api, handle) =
                anvil::spawn(anvil::NodeConfig::test_monad().with_chain_id(Some(chain_id))).await;

            let config = Config::figment();
            let mut evm_opts = config.extract::<EvmOpts>().unwrap();
            evm_opts.fork_url = Some(handle.http_endpoint());
            assert_eq!(evm_opts.networks, NetworkConfigs::default());

            evm_opts.infer_network_from_fork().await.unwrap();

            assert_eq!(evm_opts.env.chain_id, Some(chain_id));
            assert!(
                evm_opts.networks.is_monad(),
                "should detect Monad via anvil_nodeInfo for chain {chain_id}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg(feature = "monad")]
    async fn fork_context_carries_exact_monad_anvil_hardfork() {
        let activation =
            foundry_evm_hardforks::MonadHardfork::MonadNine.mainnet_activation_timestamp().unwrap();
        let (_api, handle) = anvil::spawn(
            anvil::NodeConfig::test_monad()
                .with_chain_id(Some(NamedChain::Monad as u64))
                .with_genesis_timestamp(Some(activation))
                .with_hardfork(Some(foundry_evm_hardforks::MonadHardfork::MonadEight.into())),
        )
        .await;
        let mut evm_opts = EvmOpts { fork_url: Some(handle.http_endpoint()), ..Default::default() };

        evm_opts.infer_network_from_fork().await.unwrap();
        let (mut evm_env, _, context) = evm_opts
            .env_with_fork_context::<foundry_evm_hardforks::MonadHardfork, BlockEnv, TxEnv>()
            .await
            .unwrap();
        let context = context.unwrap();

        assert_eq!(
            context.hardfork,
            Some(FoundryHardfork::Monad(foundry_evm_hardforks::MonadHardfork::MonadEight))
        );
        assert_eq!(
            resolve_execution_spec(
                &Config::default(),
                evm_opts.networks,
                &mut evm_env,
                ExecutionSpecContext::fork(context.source_chain_id, context.hardfork),
                None,
                None,
            ),
            context.hardfork
        );
        assert_eq!(evm_env.cfg_env.spec, foundry_evm_hardforks::MonadHardfork::MonadEight);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg(feature = "monad")]
    async fn fork_context_refreshes_identity_after_same_url_reset() {
        let (_api, monad_eight) = anvil::spawn(
            anvil::NodeConfig::test_monad()
                .with_chain_id(Some(NamedChain::MonadTestnet as u64))
                .with_hardfork(Some(foundry_evm_hardforks::MonadHardfork::MonadEight.into())),
        )
        .await;
        let (_api, monad_nine) = anvil::spawn(
            anvil::NodeConfig::test_monad()
                .with_chain_id(Some(NamedChain::Monad as u64))
                .with_hardfork(Some(foundry_evm_hardforks::MonadHardfork::MonadNine.into())),
        )
        .await;
        let (fork_api, fork_handle) = anvil::spawn(
            anvil::NodeConfig::test()
                .with_chain_id(Some(NamedChain::Mainnet as u64))
                .with_no_storage_caching(true)
                .with_eth_rpc_url(Some(monad_eight.http_endpoint()))
                .with_fork_block_number(Some(0u64)),
        )
        .await;

        let mut evm_opts =
            EvmOpts { fork_url: Some(fork_handle.http_endpoint()), ..Default::default() };
        evm_opts.infer_network_from_fork().await.unwrap();
        let cached = evm_opts.fork_endpoint.as_ref().unwrap();
        assert_eq!(cached.source_chain_id, NamedChain::MonadTestnet as u64);
        assert_eq!(
            cached.hardfork,
            Some(FoundryHardfork::Monad(foundry_evm_hardforks::MonadHardfork::MonadEight))
        );

        let (_, _, first) = evm_opts
            .env_with_fork_context::<foundry_evm_hardforks::MonadHardfork, BlockEnv, TxEnv>()
            .await
            .unwrap();
        assert_eq!(first.unwrap().source_chain_id, NamedChain::MonadTestnet as u64);

        fork_api
            .anvil_reset(Some(alloy_rpc_types::anvil::Forking {
                json_rpc_url: Some(monad_nine.http_endpoint()),
                block_number: Some(0),
            }))
            .await
            .unwrap();

        let (_, _, second) = evm_opts
            .env_with_fork_context::<foundry_evm_hardforks::MonadHardfork, BlockEnv, TxEnv>()
            .await
            .unwrap();
        let second = second.unwrap();
        assert_eq!(second.source_chain_id, NamedChain::Monad as u64);
        assert_eq!(
            second.hardfork,
            Some(FoundryHardfork::Monad(foundry_evm_hardforks::MonadHardfork::MonadNine))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn expected_fork_endpoint_detects_same_url_reset() {
        let (api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
        let mut evm_opts = EvmOpts { fork_url: Some(handle.http_endpoint()), ..Default::default() };
        evm_opts.infer_network_from_fork().await.unwrap();
        let identity = evm_opts.fork_endpoint.clone().unwrap();
        let original_instance = identity.instance_id;
        let network_is_inferred = evm_opts.fork_network_is_inferred;
        evm_opts.expect_fork_endpoint(identity, network_is_inferred);

        api.anvil_reset(None).await.unwrap();

        assert_ne!(api.instance_id(), original_instance.unwrap());
        let error = evm_opts.env_with_fork_context::<SpecId, BlockEnv, TxEnv>().await.unwrap_err();
        assert!(
            error.to_string().contains("changed after its execution context was selected"),
            "{error}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolved_fork_detects_same_url_reset_without_cached_expectation() {
        let (api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
        let evm_opts = EvmOpts { fork_url: Some(handle.http_endpoint()), ..Default::default() };
        let fork = evm_opts.resolve_fork().await.unwrap().unwrap();
        let original_instance = fork.context().instance_id;

        api.anvil_reset(None).await.unwrap();

        assert_ne!(api.instance_id(), original_instance.unwrap());
        let error = evm_opts
            .env_with_resolved_fork::<SpecId, BlockEnv, TxEnv>(Some(&fork))
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("changed after its block and execution context were resolved"),
            "{error}"
        );
    }

    #[test]
    #[cfg(feature = "monad")]
    fn unknown_endpoint_hardfork_is_optional_only_for_remote_execution() {
        assert_eq!(NetworkVariant::from_node_info_name("monad").unwrap(), NetworkVariant::Monad);
        assert_eq!(
            endpoint_hardfork(
                NetworkVariant::Monad,
                "MonadFuture",
                EndpointHardforkPolicy::Optional
            )
            .unwrap(),
            None
        );

        let error = endpoint_hardfork(
            NetworkVariant::Monad,
            "MonadFuture",
            EndpointHardforkPolicy::Required,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("unsupported hardfork `MonadFuture` reported for `monad`")
        );
        assert_eq!(
            endpoint_hardfork(NetworkVariant::Monad, "MonadNine", EndpointHardforkPolicy::Required)
                .unwrap(),
            Some(FoundryHardfork::Monad(foundry_evm_hardforks::MonadHardfork::MonadNine))
        );
    }

    #[test]
    #[cfg(feature = "base")]
    fn unknown_base_endpoint_hardfork_is_optional_only_for_remote_execution() {
        assert_eq!(NetworkVariant::from_node_info_name("base").unwrap(), NetworkVariant::Base);
        assert_eq!(
            endpoint_hardfork(NetworkVariant::Base, "BaseFuture", EndpointHardforkPolicy::Optional)
                .unwrap(),
            None
        );

        let error =
            endpoint_hardfork(NetworkVariant::Base, "BaseFuture", EndpointHardforkPolicy::Required)
                .unwrap_err();
        assert!(
            error.to_string().contains("unsupported hardfork `BaseFuture` reported for `base`")
        );
        assert_eq!(
            endpoint_hardfork(NetworkVariant::Base, "Beryl", EndpointHardforkPolicy::Required)
                .unwrap(),
            Some(FoundryHardfork::Base(BaseUpgrade::Beryl))
        );
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
    async fn infer_network_monad_propagates_unavailable_rpc() {
        let config = Config::figment();
        let mut evm_opts = config.extract::<EvmOpts>().unwrap();
        evm_opts.fork_url = Some("http://127.0.0.1:1".to_string());
        evm_opts.networks = NetworkConfigs::with_monad();

        let error = evm_opts.infer_network_from_fork().await.unwrap_err();

        assert!(error.to_string().contains("failed to retrieve chain ID"));
        assert!(evm_opts.networks.is_monad());
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg(feature = "base")]
    async fn infer_network_base_propagates_unavailable_rpc() {
        let mut evm_opts = EvmOpts {
            fork_url: Some("http://127.0.0.1:1".to_string()),
            networks: NetworkConfigs::with_base(),
            ..Default::default()
        };

        let error = evm_opts.infer_network_from_fork().await.unwrap_err();

        assert!(error.to_string().contains("failed to retrieve chain ID"));
        assert!(evm_opts.networks.is_base());
        assert_eq!(evm_opts.fork_endpoint, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn flaky_infer_network_tempo_moderato_rpc() {
        let config = Config::figment();
        let mut evm_opts = config.extract::<EvmOpts>().unwrap();
        evm_opts.fork_url = Some("https://rpc.moderato.tempo.xyz".to_string());
        assert_eq!(evm_opts.networks, NetworkConfigs::default());

        evm_opts.infer_network_from_fork().await.unwrap();

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
        assert!(fork.evm_opts.fork_block_number_is_inferred);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_backend_rejects_changed_resolved_context() {
        let (_api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
        let mut evm_opts = EvmOpts { fork_url: Some(handle.http_endpoint()), ..Default::default() };
        evm_opts.infer_network_from_fork().await.unwrap();
        let (_, _, resolved) = evm_opts.env_resolved::<SpecId, BlockEnv, TxEnv>().await.unwrap();
        let resolved = resolved.unwrap();
        let mut context = resolved.context();
        let mut invalid_instance = context.instance_id.unwrap_or_default();
        invalid_instance[31] ^= 1;
        context.instance_id = Some(invalid_instance);
        let invalid = ResolvedFork::new(
            evm_opts.fork_url.as_deref().unwrap(),
            evm_opts.fork_source_headers(),
            evm_opts.rpc_jwt.as_deref(),
            evm_opts.fork_block_number,
            resolved.block(),
            context,
        );
        let fork = evm_opts
            .get_fork_resolved(&Config::default(), context.execution_chain_id, Some(&invalid))
            .unwrap();

        let error =
            crate::backend::Backend::<crate::evm::EthEvmNetwork>::spawn(Some(fork)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("changed after its block and execution context were resolved"),
            "{error}"
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
        assert!(!fork.evm_opts.fork_block_number_is_inferred);
    }
}
