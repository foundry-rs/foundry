use alloy_evm::{Evm, EvmEnv, EvmFactory, precompiles::PrecompilesMap};
use alloy_op_evm::{OpEvm, OpEvmContext, OpEvmFactory, OpTx};
use foundry_fork_db::DatabaseError;
use op_revm::{OpEvm as RevmEvm, OpHaltReason, OpSpecId, OpTransactionError, handler::OpHandler};
use revm::{
    context::{
        BlockEnv, ContextTr, LocalContextTr,
        result::{EVMError, HaltReason, ResultAndState},
    },
    handler::{EthFrame, EvmTr, FrameResult, Handler, instructions::EthInstructions},
    inspector::InspectorHandler,
    interpreter::{
        FrameInput, SharedMemory, interpreter::EthInterpreter, interpreter_action::FrameInit,
    },
};

use crate::{
    FoundryContextExt, FoundryInspectorExt,
    backend::{DatabaseExt, JournaledState},
    evm::{FoundryEvmFactory, NestedEvm},
};

type OpEvmHandler<'db, I> =
    OpHandler<OpRevmEvm<'db, I>, EVMError<DatabaseError, OpTransactionError>, EthFrame>;

pub type OpRevmEvm<'db, I> = RevmEvm<
    OpEvmContext<&'db mut dyn DatabaseExt<OpEvmFactory>>,
    I,
    EthInstructions<EthInterpreter, OpEvmContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>,
    PrecompilesMap,
>;

impl FoundryEvmFactory for OpEvmFactory {
    type FoundryContext<'db> = OpEvmContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> =
        OpEvm<&'db mut dyn DatabaseExt<Self>, I, Self::Precompiles>;

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let mut op_evm = Self::default().create_evm_with_inspector(db, evm_env, inspector);
        op_evm.cfg.tx_chain_id_check = true;
        op_evm.inspector().get_networks().inject_precompiles(op_evm.precompiles_mut());
        op_evm
    }

    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> Box<dyn NestedEvm<Spec = OpSpecId, Block = BlockEnv, Tx = OpTx> + 'db> {
        Box::new(self.create_foundry_evm_with_inspector(db, evm_env, inspector).into_inner())
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

impl<'db, I: FoundryInspectorExt<OpEvmContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>> NestedEvm
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
