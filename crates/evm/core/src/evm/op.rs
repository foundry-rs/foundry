use std::ops::{Deref, DerefMut};

use alloy_evm::{Evm, EvmEnv, precompiles::PrecompilesMap};
use alloy_op_evm::{OpEvmFactory, OpTx};
use alloy_primitives::{Address, Bytes};
use foundry_fork_db::DatabaseError;
use op_revm::{
    L1BlockInfo, OpEvm, OpHaltReason, OpSpecId, OpTransaction, OpTransactionError,
    handler::OpHandler, precompiles::OpPrecompiles,
};
use revm::{
    Context, Journal, MainContext,
    context::{
        BlockEnv, CfgEnv, ContextTr, LocalContextTr,
        result::{
            EVMError, ExecResultAndState, ExecutionResult, HaltReason, InvalidTransaction,
            ResultAndState,
        },
    },
    handler::{EthFrame, EvmTr, FrameResult, Handler, instructions::EthInstructions},
    inspector::{InspectorEvmTr, InspectorHandler},
    interpreter::{
        FrameInput, SharedMemory, interpreter::EthInterpreter, interpreter_action::FrameInit,
    },
};

use crate::{
    FoundryContextExt, FoundryInspectorExt,
    backend::{DatabaseExt, JournaledState},
    evm::{FoundryEvmFactory, NestedEvm},
};

// Modified revm's OpContext with `OpTx`
pub type OpContext<DB> = Context<BlockEnv, OpTx, CfgEnv<OpSpecId>, DB, Journal<DB>, L1BlockInfo>;

type OpEvmHandler<'db, I> =
    OpHandler<OpRevmEvm<'db, I>, EVMError<DatabaseError, OpTransactionError>, EthFrame>;

pub type OpRevmEvm<'db, I> = op_revm::OpEvm<
    OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>,
    I,
    EthInstructions<EthInterpreter, OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>,
    PrecompilesMap,
>;

/// Wraps [`op_revm::OpEvm`] and routes execution through [`OpHandler`].
/// It uses foundry's custom [`OpContext`] as op-revm's one is not compatible with [`OpTx`].
pub struct OpFoundryEvm<
    'db,
    I: FoundryInspectorExt<OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>,
> {
    pub inner: OpRevmEvm<'db, I>,
}

impl FoundryEvmFactory for OpEvmFactory {
    type FoundryContext<'db> = OpContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> = OpFoundryEvm<'db, I>;

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let spec_id = *evm_env.spec_id();
        let inner = Context::mainnet()
            .with_tx(OpTx(OpTransaction::builder().build_fill()))
            .with_cfg(CfgEnv::new_with_spec(OpSpecId::BEDROCK))
            .with_chain(L1BlockInfo::default())
            .with_db(db)
            .with_block(evm_env.block_env)
            .with_cfg(evm_env.cfg_env);
        let mut inner = OpEvm::new(inner, inspector).with_precompiles(PrecompilesMap::from_static(
            OpPrecompiles::new_with_spec(spec_id).precompiles(),
        ));
        inner.ctx_mut().cfg.tx_chain_id_check = true;

        let mut evm: OpFoundryEvm<'_, I> = OpFoundryEvm { inner };
        let networks = Evm::inspector(&evm).get_networks();
        networks.inject_precompiles(evm.precompiles_mut());
        evm
    }

    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> Box<dyn NestedEvm<Spec = OpSpecId, Block = BlockEnv, Tx = OpTx> + 'db> {
        Box::new(self.create_foundry_evm_with_inspector(db, evm_env, inspector).inner)
    }
}

impl<'db, I: FoundryInspectorExt<OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>> Evm
    for OpFoundryEvm<'db, I>
{
    type Precompiles = PrecompilesMap;
    type Inspector = I;
    type DB = &'db mut dyn DatabaseExt<OpEvmFactory>;
    type Error = EVMError<DatabaseError>;
    type HaltReason = OpHaltReason;
    type Spec = OpSpecId;
    type Tx = OpTx;
    type BlockEnv = BlockEnv;

    fn block(&self) -> &BlockEnv {
        &self.inner.ctx_ref().block
    }

    fn chain_id(&self) -> u64 {
        self.inner.ctx_ref().cfg.chain_id
    }

    fn components(&self) -> (&Self::DB, &Self::Inspector, &Self::Precompiles) {
        let (ctx, _, precompiles, _, inspector) = self.inner.all_inspector();
        (&ctx.journaled_state.database, inspector, precompiles)
    }

    fn components_mut(&mut self) -> (&mut Self::DB, &mut Self::Inspector, &mut Self::Precompiles) {
        let (ctx, _, precompiles, _, inspector) = self.inner.all_mut_inspector();
        (&mut ctx.journaled_state.database, inspector, precompiles)
    }

    fn set_inspector_enabled(&mut self, _enabled: bool) {
        unimplemented!("OpFoundryEvm is always inspecting")
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        self.inner.ctx().set_tx(tx);

        let mut handler = OpEvmHandler::<I>::new();
        // Convert OpTransactionError to InvalidTransaction due to missing InvalidTxError impl
        let result = handler.inspect_run(&mut self.inner).map_err(|e| match e {
            EVMError::Transaction(tx_error) => EVMError::Transaction(match tx_error {
                OpTransactionError::Base(invalid_transaction) => invalid_transaction,
                OpTransactionError::DepositSystemTxPostRegolith => {
                    InvalidTransaction::Str("DepositSystemTxPostRegolith".into())
                }
                OpTransactionError::HaltedDepositPostRegolith => {
                    InvalidTransaction::Str("HaltedDepositPostRegolith".into())
                }
                OpTransactionError::MissingEnvelopedTx => {
                    InvalidTransaction::Str("MissingEnvelopedTx".into())
                }
            }),
            EVMError::Header(invalid_header) => EVMError::Header(invalid_header),
            EVMError::Database(db_error) => EVMError::Database(db_error),
            EVMError::Custom(custom_error) => EVMError::Custom(custom_error),
            EVMError::CustomAny(custom_any_error) => EVMError::CustomAny(custom_any_error),
        })?;

        Ok(ResultAndState::new(result, self.inner.ctx_ref().journaled_state.inner.state.clone()))
    }

    fn transact_system_call(
        &mut self,
        _caller: Address,
        _contract: Address,
        _data: Bytes,
    ) -> Result<ExecResultAndState<ExecutionResult<Self::HaltReason>>, Self::Error> {
        unimplemented!()
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec>)
    where
        Self: Sized,
    {
        let Context { block: block_env, cfg: cfg_env, journaled_state, .. } = self.inner.0.ctx;
        (journaled_state.database, EvmEnv { block_env, cfg_env })
    }
}

impl<'db, I: FoundryInspectorExt<OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>> Deref
    for OpFoundryEvm<'db, I>
{
    type Target = OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>;

    fn deref(&self) -> &Self::Target {
        &self.inner.0.ctx
    }
}

impl<'db, I: FoundryInspectorExt<OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>> DerefMut
    for OpFoundryEvm<'db, I>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner.0.ctx
    }
}

/// Maps an OP [`EVMError`] to the common `EVMError<DatabaseError>` used by [`NestedEvm`].
fn map_op_error(e: EVMError<DatabaseError, OpTransactionError>) -> EVMError<DatabaseError> {
    match e {
        EVMError::Database(db) => EVMError::Database(db),
        EVMError::Header(h) => EVMError::Header(h),
        EVMError::Custom(s) => EVMError::Custom(s),
        EVMError::Transaction(t) => EVMError::Custom(format!("op transaction error: {t}")),
        EVMError::CustomAny(custom_any_error) => EVMError::CustomAny(custom_any_error),
    }
}

impl<'db, I: FoundryInspectorExt<OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>> NestedEvm
    for OpRevmEvm<'db, I>
{
    type Spec = OpSpecId;
    type Block = BlockEnv;
    type Tx = OpTx;

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx().journaled_state.inner
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        let mut handler = OpEvmHandler::<I>::new();

        let memory =
            SharedMemory::new_with_buffer(self.ctx_ref().local().shared_memory_buffer().clone());
        let first_frame_input = FrameInit { depth: 0, memory, frame_input: frame };

        let mut frame_result =
            handler.inspect_run_exec_loop(self, first_frame_input).map_err(map_op_error)?;

        handler.last_frame_result(self, &mut frame_result).map_err(map_op_error)?;

        Ok(frame_result)
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<HaltReason>, EVMError<DatabaseError>> {
        self.ctx().set_tx(tx);

        let mut handler = OpEvmHandler::<I>::new();
        let result = handler.inspect_run(self).map_err(map_op_error)?;

        let result = result.map_haltreason(|h| match h {
            OpHaltReason::Base(eth) => eth,
            _ => HaltReason::PrecompileError,
        });

        Ok(ResultAndState::new(result, self.ctx_ref().journaled_state.inner.state.clone()))
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block> {
        self.ctx_ref().evm_clone()
    }
}
