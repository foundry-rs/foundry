//! The Ethereum network: reth's Ethereum node with the anvil pool and executor wrappers.

use super::{AnvilAdapter, AnvilComponents, AnvilNetwork, Prepared};
use crate::{
    config::NodeConfig,
    evm::AnvilExecutorBuilder,
    fork::ForkBackend,
    logging::{LoggingState, NodeInfoLayer},
    pool::AnvilPoolBuilder,
};
use eyre::Result;
use reth_ethereum::{
    chainspec::ChainSpec,
    engine::local::LocalPayloadAttributesBuilder,
    node::{
        EthereumAddOns, EthereumEthApiBuilder, EthereumExecutorBuilder, EthereumNode,
        EthereumPayloadBuilder,
        builder::{
            components::{
                BasicPayloadServiceBuilder, ComponentsBuilder, NoopConsensusBuilder,
                NoopNetworkBuilder,
            },
            rpc::{BasicEngineApiBuilder, BasicEngineValidatorBuilder},
        },
        node::EthereumEngineValidatorBuilder,
    },
};
use std::sync::Arc;

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
        AnvilExecutorBuilder<EthereumExecutorBuilder>,
        NoopConsensusBuilder,
    >;
    type AddOns = EthereumAddOns<
        super::NodeOf<Self>,
        EthereumEthApiBuilder,
        EthereumEngineValidatorBuilder,
        BasicEngineApiBuilder<EthereumEngineValidatorBuilder>,
        BasicEngineValidatorBuilder<EthereumEngineValidatorBuilder>,
        NodeInfoLayer,
    >;
    type Attributes = LocalPayloadAttributesBuilder<ChainSpec>;

    async fn prepare(config: &mut NodeConfig) -> Result<Prepared<EthereumNode>> {
        if config.is_fork() {
            let (fork, accounts) = ForkBackend::setup(config).await?;
            config.apply_fork(fork.chain_id(), fork.header(), fork.gas_price());
            let chain_spec = config.fork_chain_spec(fork.header(), &accounts)?;
            return Ok(Prepared { chain_spec, fork: Some(fork) });
        }
        Ok(Prepared { chain_spec: config.chain_spec()?, fork: None })
    }

    fn components(anvil: &AnvilComponents) -> Self::Components {
        EthereumNode::components()
            .network(NoopNetworkBuilder::eth())
            .pool(AnvilPoolBuilder {
                state: anvil.impersonation.clone(),
                order: anvil.config.transaction_order,
            })
            .executor(AnvilExecutorBuilder {
                inner: EthereumExecutorBuilder::default(),
                state: anvil.impersonation.clone(),
                block_env: anvil.block_env.clone(),
                anvil_state: anvil.anvil_state.clone(),
            })
            .consensus(NoopConsensusBuilder)
    }

    fn add_ons(logging: LoggingState) -> Self::AddOns {
        EthereumAddOns::default().with_rpc_middleware(NodeInfoLayer::new(logging))
    }

    fn payload_attributes(chain_spec: Arc<ChainSpec>) -> Self::Attributes {
        LocalPayloadAttributesBuilder::new(chain_spec)
    }
}
