use crate::{
    block_env::BlockEnvOverrides,
    impersonation::ImpersonationState,
    state::{SharedAnvilState, StateOverride},
};
use alloy_eips::Decodable2718;
use alloy_evm::{
    Evm, EvmFactory,
    block::{
        BlockExecutionError, BlockExecutionResult, BlockExecutor, BlockExecutorFactory,
        ExecutableTx, GasOutput, StateDB,
    },
};
use alloy_primitives::{Bytes, U256};
use alloy_rpc_types_engine::ExecutionData;
use eyre::Result;
use reth_ethereum::{
    evm::primitives::{
        ConfigureEngineEvm, ConfigureEvm, EvmEnvFor, ExecutableTxIterator, ExecutionCtxFor,
        NextBlockEnvAttributes, SenderRecoveryCache, execute::BlockAssembler,
    },
    node::{
        api::{BlockTy, NodePrimitives, PayloadTypes},
        builder::{BuilderContext, FullNodeTypes, NodeTypes, components::ExecutorBuilder},
    },
    primitives::{Recovered, SealedBlock, SealedHeader, SignedTransaction},
    storage::errors::any::AnyError,
};
use revm::{
    Inspector,
    context::Block as _,
    state::{Account, EvmState, EvmStorageSlot, TransactionId},
};
use std::{collections::hash_map::Entry, fmt::Debug};

/// Next-block attributes whose gas limit the block environment overrides can set.
pub trait AnvilNextBlockEnv: Clone {
    /// Sets the block gas limit.
    fn set_gas_limit(&mut self, gas_limit: u64);
}

impl AnvilNextBlockEnv for NextBlockEnvAttributes {
    fn set_gas_limit(&mut self, gas_limit: u64) {
        self.gas_limit = gas_limit;
    }
}

/// Execution payloads whose raw transactions the engine EVM config can decode.
pub trait AnvilExecutionPayload {
    /// Returns the encoded transactions of the payload.
    fn raw_transactions(&self) -> Vec<Bytes>;
}

impl AnvilExecutionPayload for ExecutionData {
    fn raw_transactions(&self) -> Vec<Bytes> {
        self.payload.transactions().clone()
    }
}

/// Wraps an inner EVM config to override sender recovery for impersonated transactions during
/// engine payload execution, and to apply the block environment overrides for the gas limit and
/// the base fee.
#[derive(Debug, Clone)]
pub struct AnvilEvmConfig<Evm: ConfigureEvm> {
    inner: Evm,
    executor_factory: AnvilBlockExecutorFactory<Evm::BlockExecutorFactory>,
    state: ImpersonationState,
    block_env: BlockEnvOverrides,
    /// The node's sender cache. Impersonated senders are recorded here so RPC lookups that
    /// recover senders through the cache report the impersonated account.
    sender_cache: Option<SenderRecoveryCache>,
}

impl<Evm: ConfigureEvm<BlockExecutorFactory: Clone>> AnvilEvmConfig<Evm> {
    /// Wraps the given EVM config.
    pub fn new(
        inner: Evm,
        state: ImpersonationState,
        block_env: BlockEnvOverrides,
        anvil_state: SharedAnvilState,
        sender_cache: Option<SenderRecoveryCache>,
    ) -> Self {
        let executor_factory =
            AnvilBlockExecutorFactory::new(inner.block_executor_factory().clone(), anvil_state);
        Self { inner, executor_factory, state, block_env, sender_cache }
    }
}

impl<Evm> ConfigureEvm for AnvilEvmConfig<Evm>
where
    Evm: ConfigureEvm<
            NextBlockEnvCtx: AnvilNextBlockEnv,
            BlockExecutorFactory: Clone + Debug + Send + Sync + Unpin,
            BlockAssembler: BlockAssembler<
                AnvilBlockExecutorFactory<Evm::BlockExecutorFactory>,
                Block = <Evm::Primitives as NodePrimitives>::Block,
            >,
        >,
{
    type Primitives = Evm::Primitives;
    type Error = Evm::Error;
    type NextBlockEnvCtx = Evm::NextBlockEnvCtx;
    type BlockExecutorFactory = AnvilBlockExecutorFactory<Evm::BlockExecutorFactory>;
    type BlockAssembler = Evm::BlockAssembler;

    fn block_executor_factory(&self) -> &Self::BlockExecutorFactory {
        &self.executor_factory
    }

    fn block_assembler(&self) -> &Self::BlockAssembler {
        self.inner.block_assembler()
    }

    fn evm_env(
        &self,
        header: &<Evm::Primitives as NodePrimitives>::BlockHeader,
    ) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.inner.evm_env(header)
    }

    fn next_evm_env(
        &self,
        parent: &<Evm::Primitives as NodePrimitives>::BlockHeader,
        attributes: &Self::NextBlockEnvCtx,
    ) -> Result<EvmEnvFor<Self>, Self::Error> {
        let mut attributes = attributes.clone();
        if let Some(gas_limit) = self.block_env.gas_limit() {
            attributes.set_gas_limit(gas_limit);
        }
        let mut env = self.inner.next_evm_env(parent, &attributes)?;
        env.set_base_fee_opt(self.block_env.take_next_base_fee());
        Ok(env)
    }

    fn context_for_block<'a>(
        &self,
        block: &'a SealedBlock<<Evm::Primitives as NodePrimitives>::Block>,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        self.inner.context_for_block(block)
    }

    fn context_for_next_block(
        &self,
        parent: &SealedHeader<<Evm::Primitives as NodePrimitives>::BlockHeader>,
        attributes: Self::NextBlockEnvCtx,
    ) -> Result<ExecutionCtxFor<'_, Self>, Self::Error> {
        self.inner.context_for_next_block(parent, attributes)
    }
}

impl<Evm, Payload> ConfigureEngineEvm<Payload> for AnvilEvmConfig<Evm>
where
    Evm: ConfigureEvm<
            NextBlockEnvCtx: AnvilNextBlockEnv,
            BlockExecutorFactory: Clone + Debug + Send + Sync + Unpin,
            BlockAssembler: BlockAssembler<
                AnvilBlockExecutorFactory<Evm::BlockExecutorFactory>,
                Block = <Evm::Primitives as NodePrimitives>::Block,
            >,
        > + ConfigureEngineEvm<Payload>,
    Payload: AnvilExecutionPayload,
{
    fn evm_env_for_payload(&self, payload: &Payload) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.inner.evm_env_for_payload(payload)
    }

    fn context_for_payload<'a>(
        &self,
        payload: &'a Payload,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        self.inner.context_for_payload(payload)
    }

    fn tx_iterator_for_payload(
        &self,
        payload: &Payload,
    ) -> Result<impl ExecutableTxIterator<Self>, Self::Error> {
        let txs = payload.raw_transactions();
        let state = self.state.clone();
        let sender_cache = self.sender_cache.clone();

        let convert = move |raw: Bytes| {
            let tx = <Evm::Primitives as NodePrimitives>::SignedTx::decode_2718_exact(raw.as_ref())
                .map_err(AnyError::new)?;
            let signer = match state.tx_sender(&tx.recalculate_hash()) {
                Some(sender) => {
                    if let Some(cache) = &sender_cache {
                        let _ = cache.recover_with(&tx, |_| Ok(sender));
                    }
                    sender
                }
                None => match &sender_cache {
                    Some(cache) => cache.recover(&tx).map_err(AnyError::new)?,
                    None => tx.try_recover().map_err(AnyError::new)?,
                },
            };
            Ok::<_, AnyError>(Recovered::new_unchecked(tx, signer))
        };

        Ok((txs, convert))
    }
}

/// Block executor factory that applies the queued anvil state writes at the start of every block.
#[derive(Debug, Clone)]
pub struct AnvilBlockExecutorFactory<F> {
    inner: F,
    state: SharedAnvilState,
}

impl<F> AnvilBlockExecutorFactory<F> {
    /// Wraps the given factory.
    pub const fn new(inner: F, state: SharedAnvilState) -> Self {
        Self { inner, state }
    }
}

impl<F: BlockExecutorFactory> BlockExecutorFactory for AnvilBlockExecutorFactory<F> {
    type EvmFactory = F::EvmFactory;
    type TxExecutionResult = F::TxExecutionResult;
    type ExecutionCtx<'a> = F::ExecutionCtx<'a>;
    type Transaction = F::Transaction;
    type Receipt = F::Receipt;
    type Executor<'a, DB: StateDB, I: Inspector<<Self::EvmFactory as EvmFactory>::Context<DB>>> =
        AnvilBlockExecutor<F::Executor<'a, DB, I>>;

    fn evm_factory(&self) -> &Self::EvmFactory {
        self.inner.evm_factory()
    }

    fn create_executor<'a, DB, I>(
        &'a self,
        evm: <Self::EvmFactory as EvmFactory>::Evm<DB, I>,
        ctx: Self::ExecutionCtx<'a>,
    ) -> Self::Executor<'a, DB, I>
    where
        DB: StateDB,
        I: Inspector<<Self::EvmFactory as EvmFactory>::Context<DB>>,
    {
        AnvilBlockExecutor {
            inner: self.inner.create_executor(evm, ctx),
            state: self.state.clone(),
        }
    }
}

/// Block executor that applies the queued anvil state writes after the pre-execution changes.
#[derive(Debug)]
pub struct AnvilBlockExecutor<E> {
    inner: E,
    state: SharedAnvilState,
}

impl<E> BlockExecutor for AnvilBlockExecutor<E>
where
    E: BlockExecutor,
    <E::Evm as Evm>::DB: StateDB,
{
    type Transaction = E::Transaction;
    type Receipt = E::Receipt;
    type Evm = E::Evm;
    type Result = E::Result;

    fn apply_pre_execution_changes(&mut self) -> Result<(), BlockExecutionError> {
        self.inner.apply_pre_execution_changes()?;
        let number = self.inner.evm().block().number().saturating_to::<u64>();
        let writes = self.state.write().overrides_for_block(number);
        if writes.is_empty() {
            return Ok(());
        }
        apply_state_writes(self.inner.evm_mut().db_mut(), &writes)
    }

    fn execute_transaction_without_commit(
        &mut self,
        tx: impl ExecutableTx<Self>,
    ) -> Result<Self::Result, BlockExecutionError> {
        self.inner.execute_transaction_without_commit(tx)
    }

    fn commit_transaction(&mut self, output: Self::Result) -> GasOutput {
        self.inner.commit_transaction(output)
    }

    fn finish(
        self,
    ) -> Result<(Self::Evm, BlockExecutionResult<Self::Receipt>), BlockExecutionError> {
        self.inner.finish()
    }

    fn evm_mut(&mut self) -> &mut Self::Evm {
        self.inner.evm_mut()
    }

    fn evm(&self) -> &Self::Evm {
        self.inner.evm()
    }

    fn receipts(&self) -> &[Self::Receipt] {
        self.inner.receipts()
    }
}

/// Executor builder that wraps a network's executor builder in an [`AnvilEvmConfig`].
#[derive(Debug, Clone)]
pub struct AnvilExecutorBuilder<Inner> {
    /// The network's executor builder.
    pub inner: Inner,
    /// The shared impersonation state.
    pub state: ImpersonationState,
    /// The shared block environment overrides.
    pub block_env: BlockEnvOverrides,
    /// The shared anvil state writes.
    pub anvil_state: SharedAnvilState,
}

/// The execution payload of a node.
type ExecutionDataOf<Node> =
    <<<Node as FullNodeTypes>::Types as NodeTypes>::Payload as PayloadTypes>::ExecutionData;

impl<Node, Inner> ExecutorBuilder<Node> for AnvilExecutorBuilder<Inner>
where
    Node: FullNodeTypes,
    Inner: ExecutorBuilder<Node>,
    Inner::EVM: ConfigureEvm<
            Primitives = <Node::Types as NodeTypes>::Primitives,
            NextBlockEnvCtx: AnvilNextBlockEnv,
            BlockExecutorFactory: Clone + Debug + Send + Sync + Unpin,
            BlockAssembler: BlockAssembler<
                AnvilBlockExecutorFactory<<Inner::EVM as ConfigureEvm>::BlockExecutorFactory>,
                Block = BlockTy<Node::Types>,
            >,
        > + ConfigureEngineEvm<ExecutionDataOf<Node>>,
    ExecutionDataOf<Node>: AnvilExecutionPayload,
{
    type EVM = AnvilEvmConfig<Inner::EVM>;

    async fn build_evm(self, ctx: &BuilderContext<Node>) -> Result<Self::EVM> {
        Ok(AnvilEvmConfig::new(
            self.inner.build_evm(ctx).await?,
            self.state,
            self.block_env,
            self.anvil_state,
            ctx.sender_recovery_cache().cloned(),
        ))
    }
}

/// Commits the given state writes to the block's state database as one touched-account change set,
/// so they become part of the block's state transition.
fn apply_state_writes<DB: StateDB>(
    db: &mut DB,
    writes: &[StateOverride],
) -> Result<(), BlockExecutionError> {
    let mut changes = EvmState::default();
    for write in writes {
        let address = match write {
            StateOverride::Balance(address, _)
            | StateOverride::Nonce(address, _)
            | StateOverride::Code(address, _)
            | StateOverride::Storage(address, _, _) => *address,
        };
        let account = match changes.entry(address) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let info =
                    db.basic(address).map_err(BlockExecutionError::other)?.unwrap_or_default();
                let mut account = Account::from(info);
                account.mark_touch();
                entry.insert(account)
            }
        };
        match write {
            StateOverride::Balance(_, balance) => account.info.balance = *balance,
            StateOverride::Nonce(_, nonce) => account.info.nonce = *nonce,
            StateOverride::Code(_, code) => {
                account.info.code_hash = code.hash_slow();
                account.info.code = Some(code.0.clone());
            }
            StateOverride::Storage(_, slot, value) => {
                let slot = U256::from_be_bytes(slot.0);
                let original = db.storage(address, slot).map_err(BlockExecutionError::other)?;
                account.storage.insert(
                    slot,
                    EvmStorageSlot::new_changed(original, *value, TransactionId::ZERO),
                );
            }
        }
    }
    db.commit(changes);
    Ok(())
}
