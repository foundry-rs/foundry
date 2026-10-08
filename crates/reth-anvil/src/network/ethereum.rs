//! The Ethereum network: reth's Ethereum node with the anvil pool and executor wrappers.

use super::{AnvilAdapter, AnvilComponents, AnvilNetwork, AnvilRpcOf, Prepared};
use crate::{
    config::{ForkSource, NodeConfig},
    engine::AnvilEngineValidatorBuilder,
    evm::{AnvilEvmFactory, AnvilExecutorBuilder, PrecompileBuilder},
    fork::{EthereumFork, ForkBackend, ForkInfo, dump_head},
    impersonation::ImpersonationState,
    logging::{LoggingState, NodeInfoLayer},
    pending::AnvilEthApiBuilder,
    pool::{AnvilPoolBuilder, PoolSettings},
};
use alloy_evm::eth::spec::EthExecutorSpec;
use alloy_primitives::{Address, Bytes, U256};
use eyre::Result;
use foundry_evm_networks::{
    arbitrum,
    celo::transfer::{self as celo_transfer, CELO_TRANSFER_ADDRESS},
};
use jsonrpsee::{
    RpcModule,
    core::RpcResult,
    types::{ErrorObjectOwned, Params, error::INTERNAL_ERROR_CODE},
};
use reth_ethereum::{
    EthPrimitives,
    chainspec::{ChainSpec, EthereumHardforks, Hardforks},
    engine::local::LocalPayloadAttributesBuilder,
    evm::{EthEvmConfig, factory::RethEvmFactory},
    node::{
        EthereumAddOns, EthereumNode, EthereumPayloadBuilder,
        builder::{
            BuilderContext, FullNodeTypes, NodeTypes,
            components::{
                BasicPayloadServiceBuilder, ComponentsBuilder, ExecutorBuilder,
                NoopConsensusBuilder, NoopNetworkBuilder,
            },
            rpc::{BasicEngineApiBuilder, BasicEngineValidatorBuilder, RpcAddOns},
        },
    },
};
use serde::de::DeserializeOwned;
use std::{fmt, sync::Arc};
use tower::layer::util::Identity;

/// The Ethereum network.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ethereum;

impl AnvilNetwork for Ethereum {
    type Node = EthereumNode;
    type Fork = EthereumFork;
    type Components = ComponentsBuilder<
        AnvilAdapter<EthereumNode>,
        AnvilPoolBuilder,
        BasicPayloadServiceBuilder<EthereumPayloadBuilder>,
        NoopNetworkBuilder,
        AnvilExecutorBuilder<EthereumEvmBuilder>,
        NoopConsensusBuilder,
    >;
    type AddOns = EthereumAddOns<
        super::NodeOf<Self>,
        AnvilEthApiBuilder,
        AnvilEngineValidatorBuilder,
        BasicEngineApiBuilder<AnvilEngineValidatorBuilder>,
        BasicEngineValidatorBuilder<AnvilEngineValidatorBuilder>,
        NodeInfoLayer,
    >;
    type Attributes = LocalPayloadAttributesBuilder<ChainSpec>;

    async fn prepare(config: &mut NodeConfig) -> Result<Prepared<EthereumNode>> {
        prepare(config).await
    }

    fn components(anvil: &AnvilComponents) -> Self::Components {
        EthereumNode::components()
            .network(NoopNetworkBuilder::eth())
            .pool(AnvilPoolBuilder {
                state: anvil.impersonation.clone(),
                order: anvil.order.clone(),
                settings: PoolSettings::from_config(&anvil.config),
            })
            .executor(AnvilExecutorBuilder::new(
                EthereumEvmBuilder::new(
                    network_precompiles(&anvil.config),
                    anvil.fork.clone(),
                    anvil.console.is_some(),
                    anvil.impersonation.clone(),
                ),
                anvil,
            ))
            .consensus(NoopConsensusBuilder)
    }

    fn add_ons(anvil: &AnvilComponents, logging: LoggingState) -> Self::AddOns {
        EthereumAddOns::new(RpcAddOns::new(
            AnvilEthApiBuilder::new(anvil.time.clone(), anvil.block_env.clone()),
            AnvilEngineValidatorBuilder,
            BasicEngineApiBuilder::default(),
            BasicEngineValidatorBuilder::default(),
            NodeInfoLayer::new(logging),
            Identity::new(),
        ))
    }

    fn extend_rpc(
        rpc: AnvilRpcOf<Self>,
        _anvil: &AnvilComponents,
    ) -> Result<(AnvilRpcOf<Self>, RpcModule<()>)> {
        // Preserve Ethereum's legacy rejection of Tempo-only RPC methods. Their implementations
        // and native request types belong to the external Tempo crate.
        let mut module = RpcModule::new(());
        for method in ["anvil_dealTIP20", "anvil_setFeeAmmLiquidity"] {
            module.register_method(method, |params, _, _| {
                unsupported_network_method::<(Address, Address, U256)>(params)
            })?;
        }
        for method in ["anvil_setFeeToken", "anvil_setValidatorFeeToken"] {
            module.register_method(method, |params, _, _| {
                unsupported_network_method::<(Address, Address)>(params)
            })?;
        }
        module.register_method("eth_signRawTransaction", |params, _, _| {
            unsupported_network_method::<(Bytes,)>(params)
        })?;
        Ok((rpc, module))
    }

    fn payload_attributes(chain_spec: Arc<ChainSpec>) -> Self::Attributes {
        LocalPayloadAttributesBuilder::new(chain_spec)
    }
}

/// Keeps the existing parameter validation and error for unsupported network methods.
fn unsupported_network_method<T: DeserializeOwned>(params: Params<'_>) -> RpcResult<()> {
    params.parse::<T>()?;
    Err(ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, "Not implemented", None::<()>))
}

/// Resolves the chain spec and the fork of a network built on reth's Ethereum node types.
pub(super) async fn prepare(config: &mut NodeConfig) -> Result<Prepared<EthereumNode>> {
    if config.is_fork() {
        let (fork, accounts) = ForkBackend::<EthereumFork>::setup(config).await?;
        config.apply_fork(fork.chain_id(), fork.header().header(), fork.gas_price());
        if let Some(info) = fork.node_info() {
            config.adopt_fork_identity(info);
        }
        // A state dump whose head lies above the fork block continues at its head, with the
        // dump's blocks above the fork block; one at or below it only overlays its accounts.
        if config.init_state.as_ref().is_some_and(|state| {
            state.head_number().is_some_and(|number| number > fork.block_number())
        }) && let Some(mut state) = config.init_state.take()
        {
            config.ensure_dump_head::<EthereumFork>(&mut state, config.get_hardfork()?)?;
            let head = dump_head::<EthereumFork>(&state)?;
            let fork = Arc::try_unwrap(fork)
                .map_err(|_| eyre::eyre!("the fork backend is shared"))?
                .into_dump_fork(&state, &head)?;
            let chain_spec = config.dump_chain_spec(&head, &state)?;
            config.init_state = Some(state);
            return Ok(Prepared { chain_spec, fork: Some(Arc::new(fork)) });
        }
        let source = ForkSource {
            chain_id: fork.chain_id(),
            node_info: fork.node_info(),
            replay_timestamp: fork.replay_timestamp(),
        };
        let chain_spec = config.fork_chain_spec(fork.header(), &accounts, source)?;
        return Ok(Prepared { chain_spec, fork: Some(fork) });
    }
    // A state dump with a block environment continues at its head block, which becomes the
    // genesis block; the blocks before it come from the dump.
    if config.init_state.as_ref().is_some_and(|state| state.block.is_some()) {
        let mut state = config.init_state.take().expect("checked above");
        config.ensure_dump_head::<EthereumFork>(&mut state, config.get_hardfork()?)?;
        let head = dump_head::<EthereumFork>(&state)?;
        let fork = ForkBackend::from_dump(config, &state, &head)?;
        let chain_spec = config.dump_chain_spec(&head, &state)?;
        config.init_state = Some(state);
        return Ok(Prepared { chain_spec, fork: Some(fork) });
    }
    Ok(Prepared { chain_spec: config.chain_spec()?, fork: None })
}

/// Builds reth's Ethereum EVM config with the anvil precompiles installed and the fork's block
/// hashes.
#[derive(Clone)]
pub struct EthereumEvmBuilder {
    precompiles: Vec<(Address, PrecompileBuilder)>,
    fork: Option<Arc<dyn ForkInfo>>,
    console: bool,
    impersonation: ImpersonationState,
}

impl fmt::Debug for EthereumEvmBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let addresses: Vec<_> = self.precompiles.iter().map(|(address, _)| address).collect();
        f.debug_struct("EthereumEvmBuilder")
            .field("precompiles", &addresses)
            .field("fork", &self.fork)
            .field("console", &self.console)
            .finish()
    }
}

impl EthereumEvmBuilder {
    /// Creates the builder with the precompiles to install, the fork, if any, and whether to
    /// collect `console.log` calls.
    pub const fn new(
        precompiles: Vec<(Address, PrecompileBuilder)>,
        fork: Option<Arc<dyn ForkInfo>>,
        console: bool,
        impersonation: ImpersonationState,
    ) -> Self {
        Self { precompiles, fork, console, impersonation }
    }
}

impl<Types, Node> ExecutorBuilder<Node> for EthereumEvmBuilder
where
    Types: NodeTypes<
            ChainSpec: Hardforks + EthExecutorSpec + EthereumHardforks,
            Primitives = EthPrimitives,
        >,
    Node: FullNodeTypes<Types = Types>,
{
    type EVM = EthEvmConfig<Types::ChainSpec, AnvilEvmFactory<RethEvmFactory>>;

    async fn build_evm(self, ctx: &BuilderContext<Node>) -> Result<Self::EVM> {
        let factory = AnvilEvmFactory::new(
            RethEvmFactory::default(),
            self.precompiles,
            self.fork,
            self.console,
            self.impersonation,
        );
        let mut evm_config = EthEvmConfig::new_with_evm_factory(ctx.chain_spec(), factory);
        if let Some(cache) = ctx.sender_recovery_cache() {
            evm_config = evm_config.with_sender_recovery_cache(cache.clone());
        }
        Ok(evm_config)
    }
}

/// The precompiles the configured network adds: Celo's native transfer on Celo, and `ArbSys`
/// on Arbitrum chains.
fn network_precompiles(config: &NodeConfig) -> Vec<(Address, PrecompileBuilder)> {
    let mut precompiles: Vec<(Address, PrecompileBuilder)> = Vec::new();
    if config.networks.is_celo() {
        precompiles.push((CELO_TRANSFER_ADDRESS, Arc::new(|_| celo_transfer::precompile())));
    }
    if arbitrum::is_arbitrum_chain(config.get_chain_id()) {
        precompiles.push((arbitrum::ARB_SYS_ADDRESS, Arc::new(arbitrum::arb_sys_precompile)));
    }
    precompiles
}
