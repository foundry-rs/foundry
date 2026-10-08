//! The networks reth-anvil can run: each one assembles a reth node type with the anvil pieces
//! installed.

use crate::{
    api::{AnvilRpc, CallBatch, NodeIdentity},
    block_env::BlockEnvOverrides,
    config::NodeConfig,
    console::ConsolePrinter,
    evm::{AnvilExecutionPayload, AnvilNextBlockEnv},
    fork::{EthereumFork, ForkBackend, ForkInfo, ForkNetwork},
    impersonation::ImpersonationState,
    logging::LoggingState,
    pool::SharedTransactionOrder,
    provider::AnvilProvider,
    state::SharedAnvilState,
    time::{AnvilPayloadAttributes, TimeManager},
    txpool::TxPoolKey,
};
use alloy_consensus::{Transaction, TxReceipt, transaction::TxHashRef};
use eyre::Result;
use jsonrpsee::RpcModule;
use reth_ethereum::{
    chainspec::{EthChainSpec, EthereumHardforks, Hardforks},
    evm::primitives::ConfigureEvm,
    node::{
        api::{
            BlockTy, FullNodeComponents, FullNodeTypes, FullNodeTypesAdapter, HeaderTy, NodeTypes,
            NodeTypesWithDBAdapter, PayloadAttributesBuilder, PayloadTypes,
        },
        builder::{
            Node, NodeAdapter, NodeComponents, NodeComponentsBuilder,
            rpc::{EngineValidatorAddOn, RethRpcAddOns},
        },
    },
    pool::TransactionPoolExt,
    primitives::{NodePrimitives, SignerRecoverable, header::HeaderMut},
    provider::{db::DatabaseEnv, providers::NodeTypesForProvider},
};
use reth_rpc_eth_api::{FullEthApiServer, RpcTypes, helpers::EthTransactions};
use std::{fmt::Debug, sync::Arc};

pub mod ethereum;

/// The database every network runs on.
pub type AnvilDb = Arc<DatabaseEnv>;

/// The node types of a network with the database attached.
pub type AnvilTypes<N> = NodeTypesWithDBAdapter<N, AnvilDb>;

/// The full node types of a network: its node types, the database, and the anvil provider.
pub type AnvilAdapter<N, F = EthereumFork> =
    FullNodeTypesAdapter<N, AnvilDb, AnvilProvider<AnvilTypes<N>, F>>;

/// The node components a network builds.
pub type ComponentsOf<Net> = <<Net as AnvilNetwork>::Components as NodeComponentsBuilder<
    AnvilAdapter<<Net as AnvilNetwork>::Node, <Net as AnvilNetwork>::Fork>,
>>::Components;

/// The launched node of a network.
pub type NodeOf<Net> = NodeAdapter<
    AnvilAdapter<<Net as AnvilNetwork>::Node, <Net as AnvilNetwork>::Fork>,
    ComponentsOf<Net>,
>;

/// The network's anvil RPC implementation, including its native request types.
pub type AnvilRpcOf<Net> = AnvilRpc<
    <NodeOf<Net> as FullNodeComponents>::Pool,
    <NodeOf<Net> as FullNodeTypes>::Provider,
    <<Net as AnvilNetwork>::AddOns as RethRpcAddOns<NodeOf<Net>>>::EthApi,
    <<Net as AnvilNetwork>::Node as NodeTypes>::ChainSpec,
    Net,
>;

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
    /// The transaction order of the pool.
    pub order: SharedTransactionOrder,
    /// The node config.
    pub config: NodeConfig,
    /// The fork, if any.
    pub fork: Option<Arc<dyn ForkInfo>>,
    /// The `console.log` printer, when printing.
    pub console: Option<ConsolePrinter>,
    /// The block time manager.
    pub time: TimeManager,
}

/// What a network prepares before the node launches: the chain spec, and the fork when the
/// config forks a remote chain.
pub struct Prepared<N: NodeTypes, F: ForkNetwork<Primitives = N::Primitives> = EthereumFork> {
    /// The chain spec.
    pub chain_spec: Arc<N::ChainSpec>,
    /// The fork, if any.
    pub fork: Option<Arc<ForkBackend<F>>>,
}

/// A network reth-anvil can run.
pub trait AnvilNetwork: Sized + Clone + Debug + Unpin + Send + Sync + 'static {
    /// The reth node type.
    type Node: Node<AnvilAdapter<Self::Node, Self::Fork>>
        + NodeTypesForProvider
        + NodeTypes<
            Primitives: NodePrimitives<
                SignedTx: Transaction + SignerRecoverable + TxHashRef,
                Receipt: TxReceipt,
                BlockHeader: HeaderMut,
            >,
            ChainSpec: EthChainSpec + EthereumHardforks + Hardforks,
            Payload: PayloadTypes<
                PayloadAttributes: AnvilPayloadAttributes<Self>,
                ExecutionData: AnvilExecutionPayload<Self>,
            >,
        >;
    /// Converts fork data into the node primitives.
    type Fork: ForkNetwork<Primitives = <Self::Node as NodeTypes>::Primitives>;
    /// The node components builder, with the anvil pool and executor wrappers installed.
    type Components: NodeComponentsBuilder<
            AnvilAdapter<Self::Node, Self::Fork>,
            Components: NodeComponents<
                AnvilAdapter<Self::Node, Self::Fork>,
                Evm: ConfigureEvm<NextBlockEnvCtx: AnvilNextBlockEnv<Self>>,
                Pool: TransactionPoolExt<Block = BlockTy<Self::Node>>,
            >,
        >;
    /// The RPC add-ons.
    type AddOns: RethRpcAddOns<
            NodeOf<Self>,
            EthApi: FullEthApiServer<
                NetworkTypes: RpcTypes<TransactionRequest: Default + CallBatch<Self>>,
                Evm: ConfigureEvm<NextBlockEnvCtx: AnvilNextBlockEnv<Self>>,
                Primitives: NodePrimitives<SignedTx: TxPoolKey<Self>>,
            > + EthTransactions
                        + Clone,
        > + EngineValidatorAddOn<NodeOf<Self>>;
    /// Whether the first block takes the genesis base fee, as anvil gives it on Ethereum, instead
    /// of the fee the chain spec's rule gives it.
    const FIRST_BLOCK_KEEPS_GENESIS_BASE_FEE: bool = true;

    /// Builds the payload attributes of the next block.
    type Attributes: PayloadAttributesBuilder<PayloadAttributesOf<Self::Node>, HeaderTy<Self::Node>>;

    /// Resolves the chain spec and the fork. May adjust the config, for example to adopt the
    /// chain id of a fork.
    fn prepare(
        config: &mut NodeConfig,
    ) -> impl Future<Output = Result<Prepared<Self::Node, Self::Fork>>> + Send;

    /// Builds the node components.
    fn components(anvil: &AnvilComponents) -> Self::Components;

    /// Builds the RPC add-ons.
    fn add_ons(anvil: &AnvilComponents, logging: LoggingState) -> Self::AddOns;

    /// Installs network RPC methods and configures state-write notifications.
    ///
    /// The returned module can add methods or replace standard handlers. Both the in-process
    /// API and each RPC transport serve these handlers, including after a reset.
    fn extend_rpc(
        rpc: AnvilRpcOf<Self>,
        _anvil: &AnvilComponents,
    ) -> Result<(AnvilRpcOf<Self>, RpcModule<()>)> {
        Ok((rpc, RpcModule::new(())))
    }

    /// Builds the payload attributes builder.
    fn payload_attributes(
        chain_spec: Arc<<Self::Node as NodeTypes>::ChainSpec>,
    ) -> Self::Attributes;

    /// Returns what `anvil_nodeInfo` reports about the network.
    fn identity(config: &NodeConfig) -> Result<NodeIdentity> {
        Ok(NodeIdentity { network: Some(config.networks.execution_profile_name()), hardfork: None })
    }
}
