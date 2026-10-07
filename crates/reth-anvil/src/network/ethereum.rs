//! The Ethereum network: reth's Ethereum node with the anvil pool and executor wrappers.

use super::{AnvilAdapter, AnvilComponents, AnvilNetwork, Prepared};
use crate::{
    config::NodeConfig,
    engine::AnvilEngineValidatorBuilder,
    evm::{AnvilEvmFactory, AnvilExecutorBuilder, EvmSettings, PrecompileBuilder},
    fork::{ForkBackend, ForkInfo},
    logging::{LoggingState, NodeInfoLayer},
    pending::AnvilEthApiBuilder,
    pool::{AnvilPoolBuilder, PoolSettings},
};
use alloy_evm::eth::spec::EthExecutorSpec;
use alloy_primitives::Address;
use eyre::Result;
use foundry_evm_networks::celo::transfer::{self as celo_transfer, CELO_TRANSFER_ADDRESS};
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
use std::{fmt, sync::Arc};
use tower::layer::util::Identity;

/// The Ethereum network.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ethereum;

impl AnvilNetwork for Ethereum {
    type Node = EthereumNode;
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
            .executor(AnvilExecutorBuilder {
                inner: EthereumEvmBuilder::new(
                    network_precompiles(&anvil.config),
                    anvil.fork.clone(),
                    anvil.console.is_some(),
                ),
                state: anvil.impersonation.clone(),
                block_env: anvil.block_env.clone(),
                anvil_state: anvil.anvil_state.clone(),
                settings: EvmSettings::from_config(&anvil.config),
                console: anvil.console.clone(),
            })
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

    fn payload_attributes(chain_spec: Arc<ChainSpec>) -> Self::Attributes {
        LocalPayloadAttributesBuilder::new(chain_spec)
    }
}

/// Resolves the chain spec and the fork of a network built on reth's Ethereum node types.
pub(super) async fn prepare(config: &mut NodeConfig) -> Result<Prepared<EthereumNode>> {
    if config.is_fork() {
        let (fork, accounts) = ForkBackend::setup(config).await?;
        config.apply_fork(fork.chain_id(), fork.header(), fork.gas_price());
        if let Some(info) = fork.node_info() {
            config.adopt_fork_identity(&info);
        }
        let chain_spec = config.fork_chain_spec(fork.header(), &accounts)?;
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
    ) -> Self {
        Self { precompiles, fork, console }
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
        );
        let mut evm_config = EthEvmConfig::new_with_evm_factory(ctx.chain_spec(), factory);
        if let Some(cache) = ctx.sender_recovery_cache() {
            evm_config = evm_config.with_sender_recovery_cache(cache.clone());
        }
        Ok(evm_config)
    }
}

/// The precompiles the configured network adds: Celo's native transfer on Celo.
fn network_precompiles(config: &NodeConfig) -> Vec<(Address, PrecompileBuilder)> {
    let mut precompiles: Vec<(Address, PrecompileBuilder)> = Vec::new();
    if config.networks.is_celo() {
        precompiles.push((CELO_TRANSFER_ADDRESS, Arc::new(celo_transfer::precompile)));
    }
    precompiles
}
