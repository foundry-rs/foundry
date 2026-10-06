use crate::{block_env::BlockEnvOverrides, impersonation::ImpersonationState};
use alloy_consensus::Header;
use alloy_eips::Decodable2718;
use alloy_primitives::Bytes;
use alloy_rpc_types_engine::ExecutionData;
use eyre::Result;
use reth_ethereum::{
    Block, EthPrimitives, TransactionSigned,
    chainspec::EthereumHardforks,
    evm::{
        EthEvmConfig,
        primitives::{
            ConfigureEngineEvm, ConfigureEvm, EvmEnvFor, ExecutableTxIterator, ExecutionCtxFor,
            NextBlockEnvAttributes,
        },
    },
    node::builder::{BuilderContext, FullNodeTypes, NodeTypes, components::ExecutorBuilder},
    primitives::{SealedBlock, SealedHeader, SignedTransaction},
    storage::errors::any::AnyError,
};
use std::fmt::Debug;

/// Wraps an inner EVM config to override sender recovery for impersonated transactions during
/// engine payload execution, and to apply the block environment overrides for the gas limit and
/// the base fee.
#[derive(Debug, Clone)]
pub struct AnvilEvmConfig<Evm> {
    inner: Evm,
    state: ImpersonationState,
    block_env: BlockEnvOverrides,
}

impl<Evm> AnvilEvmConfig<Evm> {
    /// Wraps the given EVM config.
    pub const fn new(inner: Evm, state: ImpersonationState, block_env: BlockEnvOverrides) -> Self {
        Self { inner, state, block_env }
    }
}

impl<Evm> ConfigureEvm for AnvilEvmConfig<Evm>
where
    Evm: ConfigureEvm<Primitives = EthPrimitives, NextBlockEnvCtx = NextBlockEnvAttributes>,
{
    type Primitives = Evm::Primitives;
    type Error = Evm::Error;
    type NextBlockEnvCtx = Evm::NextBlockEnvCtx;
    type BlockExecutorFactory = Evm::BlockExecutorFactory;
    type BlockAssembler = Evm::BlockAssembler;

    fn block_executor_factory(&self) -> &Self::BlockExecutorFactory {
        self.inner.block_executor_factory()
    }

    fn block_assembler(&self) -> &Self::BlockAssembler {
        self.inner.block_assembler()
    }

    fn evm_env(&self, header: &Header) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.inner.evm_env(header)
    }

    fn next_evm_env(
        &self,
        parent: &Header,
        attributes: &Self::NextBlockEnvCtx,
    ) -> Result<EvmEnvFor<Self>, Self::Error> {
        let mut attributes = attributes.clone();
        if let Some(gas_limit) = self.block_env.gas_limit() {
            attributes.gas_limit = gas_limit;
        }
        let mut env = self.inner.next_evm_env(parent, &attributes)?;
        env.set_base_fee_opt(self.block_env.take_next_base_fee());
        Ok(env)
    }

    fn context_for_block<'a>(
        &self,
        block: &'a SealedBlock<Block>,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        self.inner.context_for_block(block)
    }

    fn context_for_next_block(
        &self,
        parent: &SealedHeader,
        attributes: Self::NextBlockEnvCtx,
    ) -> Result<ExecutionCtxFor<'_, Self>, Self::Error> {
        self.inner.context_for_next_block(parent, attributes)
    }
}

impl<Evm> ConfigureEngineEvm<ExecutionData> for AnvilEvmConfig<Evm>
where
    Evm: ConfigureEvm<Primitives = EthPrimitives, NextBlockEnvCtx = NextBlockEnvAttributes>
        + ConfigureEngineEvm<ExecutionData>,
{
    fn evm_env_for_payload(&self, payload: &ExecutionData) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.inner.evm_env_for_payload(payload)
    }

    fn context_for_payload<'a>(
        &self,
        payload: &'a ExecutionData,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        self.inner.context_for_payload(payload)
    }

    fn tx_iterator_for_payload(
        &self,
        payload: &ExecutionData,
    ) -> Result<impl ExecutableTxIterator<Self>, Self::Error> {
        let txs = payload.payload.transactions().clone();
        let state = self.state.clone();

        let convert = move |raw: Bytes| {
            let tx = TransactionSigned::decode_2718_exact(raw.as_ref()).map_err(AnyError::new)?;
            let signer = match state.tx_sender(&tx.recalculate_hash()) {
                Some(sender) => sender,
                None => tx.try_recover().map_err(AnyError::new)?,
            };
            Ok::<_, AnyError>(tx.with_signer(signer))
        };

        Ok((txs, convert))
    }
}

/// Executor builder that produces an [`AnvilEvmConfig`].
#[derive(Debug, Clone)]
pub struct AnvilExecutorBuilder {
    /// The shared impersonation state.
    pub state: ImpersonationState,
    /// The shared block environment overrides.
    pub block_env: BlockEnvOverrides,
}

impl<Types, Node> ExecutorBuilder<Node> for AnvilExecutorBuilder
where
    Types: NodeTypes<ChainSpec: EthereumHardforks + Clone + Debug, Primitives = EthPrimitives>,
    Node: FullNodeTypes<Types = Types>,
    EthEvmConfig<Types::ChainSpec>: ConfigureEvm<Primitives = EthPrimitives, NextBlockEnvCtx = NextBlockEnvAttributes>
        + ConfigureEngineEvm<ExecutionData>,
{
    type EVM = AnvilEvmConfig<EthEvmConfig<Types::ChainSpec>>;

    async fn build_evm(self, ctx: &BuilderContext<Node>) -> Result<Self::EVM> {
        Ok(AnvilEvmConfig::new(EthEvmConfig::new(ctx.chain_spec()), self.state, self.block_env))
    }
}
