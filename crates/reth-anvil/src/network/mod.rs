//! The networks reth-anvil can run: each one assembles a reth node type with the anvil pieces
//! installed.

use crate::{
    block_env::BlockEnvOverrides,
    config::NodeConfig,
    evm::{AnvilExecutionPayload, AnvilNextBlockEnv},
    fork::{AnvilPrimitives, ForkOf},
    impersonation::ImpersonationState,
    logging::LoggingState,
    provider::AnvilProvider,
    state::SharedAnvilState,
    time::AnvilPayloadAttributes,
};
use alloy_consensus::{Transaction, TxReceipt, transaction::TxHashRef};
use eyre::Result;
use reth_ethereum::{
    chainspec::{EthChainSpec, EthereumHardforks, Hardforks},
    evm::primitives::ConfigureEvm,
    node::{
        api::{
            FullNodeTypesAdapter, HeaderTy, NodeTypes, NodeTypesWithDBAdapter,
            PayloadAttributesBuilder, PayloadTypes,
        },
        builder::{
            Node, NodeAdapter, NodeComponents, NodeComponentsBuilder,
            rpc::{EngineValidatorAddOn, RethRpcAddOns},
        },
    },
    primitives::SignerRecoverable,
    provider::{db::DatabaseEnv, providers::NodeTypesForProvider},
};
use reth_rpc_eth_api::{FullEthApiServer, RpcTypes, helpers::EthTransactions};
use std::sync::Arc;

pub mod ethereum;

/// The database every network runs on.
pub type AnvilDb = Arc<DatabaseEnv>;

/// The node types of a network with the database attached.
pub type AnvilTypes<N> = NodeTypesWithDBAdapter<N, AnvilDb>;

/// The full node types of a network: its node types, the database, and the anvil provider.
pub type AnvilAdapter<N> = FullNodeTypesAdapter<N, AnvilDb, AnvilProvider<AnvilTypes<N>>>;

/// The node components a network builds.
pub type ComponentsOf<Net> = <<Net as AnvilNetwork>::Components as NodeComponentsBuilder<
    AnvilAdapter<<Net as AnvilNetwork>::Node>,
>>::Components;

/// The launched node of a network.
pub type NodeOf<Net> = NodeAdapter<AnvilAdapter<<Net as AnvilNetwork>::Node>, ComponentsOf<Net>>;

/// The payload attributes of a network.
pub type PayloadAttributesOf<N> = <<N as NodeTypes>::Payload as PayloadTypes>::PayloadAttributes;

/// The anvil pieces every network installs into its components.
#[derive(Clone, Debug)]
pub struct AnvilComponents {
    /// The impersonation state.
    pub impersonation: ImpersonationState,
    /// The block environment overrides.
    pub block_env: BlockEnvOverrides,
    /// The anvil state writes.
    pub anvil_state: SharedAnvilState,
    /// The node config.
    pub config: NodeConfig,
}

/// What a network prepares before the node launches: the chain spec, and the fork when the
/// config forks a remote chain.
pub struct Prepared<N: NodeTypes<Primitives: AnvilPrimitives>> {
    /// The chain spec.
    pub chain_spec: Arc<N::ChainSpec>,
    /// The fork, if any.
    pub fork: Option<Arc<ForkOf<N::Primitives>>>,
}

/// A network reth-anvil can run.
pub trait AnvilNetwork: Sized + Send + Sync + 'static {
    /// The reth node type.
    type Node: Node<AnvilAdapter<Self::Node>>
        + NodeTypesForProvider
        + NodeTypes<
            Primitives: AnvilPrimitives<
                SignedTx: Transaction + SignerRecoverable + TxHashRef,
                Receipt: TxReceipt,
            >,
            ChainSpec: EthChainSpec + EthereumHardforks + Hardforks,
            Payload: PayloadTypes<
                PayloadAttributes: AnvilPayloadAttributes,
                ExecutionData: AnvilExecutionPayload,
            >,
        >;
    /// The node components builder, with the anvil pool and executor wrappers installed.
    type Components: NodeComponentsBuilder<
            AnvilAdapter<Self::Node>,
            Components: NodeComponents<
                AnvilAdapter<Self::Node>,
                Evm: ConfigureEvm<NextBlockEnvCtx: AnvilNextBlockEnv>,
            >,
        >;
    /// The RPC add-ons.
    type AddOns: RethRpcAddOns<
            NodeOf<Self>,
            EthApi: FullEthApiServer<NetworkTypes: RpcTypes<TransactionRequest: Default>>
                        + EthTransactions
                        + Clone,
        > + EngineValidatorAddOn<NodeOf<Self>>;
    /// Builds the payload attributes of the next block.
    type Attributes: PayloadAttributesBuilder<PayloadAttributesOf<Self::Node>, HeaderTy<Self::Node>>;

    /// Resolves the chain spec and the fork. May adjust the config, for example to adopt the
    /// chain id of a fork.
    fn prepare(
        config: &mut NodeConfig,
    ) -> impl Future<Output = Result<Prepared<Self::Node>>> + Send;

    /// Builds the node components.
    fn components(anvil: &AnvilComponents) -> Self::Components;

    /// Builds the RPC add-ons.
    fn add_ons(logging: LoggingState) -> Self::AddOns;

    /// Builds the payload attributes builder.
    fn payload_attributes(
        chain_spec: Arc<<Self::Node as NodeTypes>::ChainSpec>,
    ) -> Self::Attributes;
}
