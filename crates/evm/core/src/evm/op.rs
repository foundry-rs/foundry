use alloy_evm::{EvmEnv, EvmFactory, precompiles::PrecompilesMap};
use alloy_op_evm::{OpEvm, OpEvmContext, OpEvmFactory, OpTx};
use foundry_fork_db::DatabaseError;
use op_alloy_network::Optimism;
use op_revm::{
    L1BlockInfo, OpEvm as RevmEvm, OpHaltReason, OpSpecId, OpTransactionError, handler::OpHandler,
};
use revm::{
    context::{
        BlockEnv, Journal,
        result::{EVMError, HaltReason, ResultAndState},
    },
    handler::{EthFrame, EvmTr, FrameResult, instructions::EthInstructions},
    inspector::InspectorHandler,
    interpreter::{FrameInput, InstructionResult, interpreter::EthInterpreter},
};

use crate::{
    FoundryChain, FoundryContextExt, FoundryInspectorExt,
    backend::{DatabaseExt, JournaledState},
    evm::{
        FoundryEvmFactory, FoundryEvmNetwork, IntoInstructionResult, NestedEvm, NestedEvmFor,
        run_inspected_frame,
    },
};

impl FoundryChain<OpTx> for L1BlockInfo {}

#[derive(Clone, Copy, Debug, Default)]
pub struct OpEvmNetwork;
impl FoundryEvmNetwork for OpEvmNetwork {
    type Network = Optimism;
    type EvmFactory = OpEvmFactory;
}

impl IntoInstructionResult for OpHaltReason {
    fn into_instruction_result(self) -> InstructionResult {
        match self {
            Self::Base(eth) => eth.into(),
            Self::FailedDeposit => InstructionResult::Stop,
        }
    }
}

type OpEvmHandler<'db, I> =
    OpHandler<OpRevmEvm<'db, I>, EVMError<DatabaseError, OpTransactionError>, EthFrame>;

pub type OpRevmEvm<'db, I> = RevmEvm<
    OpEvmContext<&'db mut dyn DatabaseExt<OpEvmFactory>>,
    I,
    EthInstructions<EthInterpreter, OpEvmContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>,
    PrecompilesMap,
>;

impl FoundryEvmFactory for OpEvmFactory {
    type Chain = L1BlockInfo;
    type FoundryContext<'db> = OpEvmContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> =
        OpEvm<&'db mut dyn DatabaseExt<Self>, I, Self::Precompiles>;

    fn create_evm_with_context<DB: alloy_evm::Database>(
        &self,
        db: DB,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        chain_context: Self::Chain,
    ) -> Self::Evm<DB, revm::inspector::NoOpInspector> {
        let mut evm = self.create_evm(db, evm_env);
        evm.ctx_mut().chain = chain_context;
        evm
    }

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        chain_context: Self::Chain,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let mut op_evm = Self::default().create_evm_with_inspector(db, evm_env, inspector);
        op_evm.ctx_mut().chain = chain_context;
        op_evm.cfg.tx_chain_id_check = true;
        op_evm
    }

    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        chain_context: Self::Chain,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> NestedEvmFor<'db, Self> {
        Box::new(
            self.create_foundry_evm_with_inspector(db, evm_env, chain_context, inspector)
                .into_inner(),
        )
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
    type Chain = L1BlockInfo;
    type Journal = Journal<&'db mut dyn DatabaseExt<OpEvmFactory>>;

    fn tx_mut(&mut self) -> &mut Self::Tx {
        self.ctx_mut().tx_mut()
    }

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx().journaled_state.inner
    }

    fn chain_mut(&mut self) -> &mut Self::Chain {
        &mut self.ctx_mut().chain
    }

    fn journal_mut(&mut self) -> &mut Self::Journal {
        &mut self.ctx_mut().journaled_state
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        run_inspected_frame(self, OpEvmHandler::<I>::new(), frame).map_err(map_op_error)
    }

    fn transact_raw(&mut self, tx: Self::Tx) -> eyre::Result<ResultAndState<HaltReason>> {
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
