use alloy_evm::{
    EthEvm, EthEvmFactory, Evm, EvmEnv, EvmFactory, eth::EthEvmContext, precompiles::PrecompilesMap,
};
use foundry_fork_db::DatabaseError;
use revm::{
    context::{
        BlockEnv, ContextTr, Evm as RevmEvm, LocalContextTr, TxEnv,
        result::{EVMError, ResultAndState},
    },
    handler::{
        EthFrame, EvmTr, FrameResult, Handler, MainnetHandler, instructions::EthInstructions,
    },
    inspector::InspectorHandler,
    interpreter::{
        FrameInput, SharedMemory, interpreter::EthInterpreter, interpreter_action::FrameInit,
    },
    primitives::hardfork::SpecId,
};

use crate::{
    FoundryContextExt, FoundryContextState, FoundryInspectorExt,
    backend::{DatabaseExt, JournaledState},
    evm::{FoundryEvmFactory, NestedEvm},
};

type EthEvmHandler<'db, I> = MainnetHandler<EthRevmEvm<'db, I>, EVMError<DatabaseError>, EthFrame>;

pub type EthRevmEvm<'db, I> = RevmEvm<
    EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>,
    I,
    EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>,
    PrecompilesMap,
    EthFrame,
>;

impl FoundryEvmFactory for EthEvmFactory {
    type ContextAux = ();
    type FoundryContext<'db> = EthEvmContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> =
        EthEvm<&'db mut dyn DatabaseExt<Self>, I, Self::Precompiles>;

    fn create_evm_with_context<DB: alloy_evm::Database>(
        &self,
        db: DB,
        evm_env: EvmEnv,
        _context_aux: Self::ContextAux,
    ) -> Self::Evm<DB, revm::inspector::NoOpInspector> {
        self.create_evm(db, evm_env)
    }

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv,
        _context_aux: Self::ContextAux,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let chain_id = evm_env.cfg_env.chain_id;
        let timestamp = evm_env.block_env.timestamp.saturating_to();
        let mut eth_evm = Self::default().create_evm_with_inspector(db, evm_env, inspector);
        eth_evm.cfg.tx_chain_id_check = true;
        let networks = eth_evm.inspector().get_networks();
        networks.inject_precompiles(eth_evm.precompiles_mut());
        networks.inject_chain_precompiles(eth_evm.precompiles_mut(), chain_id, timestamp);
        eth_evm
    }

    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv,
        context_aux: Self::ContextAux,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> Box<dyn NestedEvm<Spec = SpecId, Block = BlockEnv, Tx = TxEnv, Aux = ()> + 'db> {
        Box::new(
            self.create_foundry_evm_with_inspector(db, evm_env, context_aux, inspector)
                .into_inner(),
        )
    }
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>> NestedEvm
    for EthRevmEvm<'db, I>
{
    type Spec = SpecId;
    type Block = BlockEnv;
    type Tx = TxEnv;
    type Aux = ();

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx_mut().journaled_state.inner
    }

    fn context_state(&self) -> FoundryContextState<Self::Aux> {
        self.ctx_ref().context_state()
    }

    fn aux_state(&self) -> Self::Aux {
        self.ctx_ref().aux_state()
    }

    fn set_context_state(&mut self, state: FoundryContextState<Self::Aux>) {
        self.ctx_mut().set_context_state(state);
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        let mut handler = EthEvmHandler::<I>::default();
        let reservoir = frame.reservoir();

        // Create first frame
        let memory =
            SharedMemory::new_with_buffer(self.ctx_ref().local().shared_memory_buffer().clone());
        let first_frame_input = FrameInit { depth: 0, memory, frame_input: frame };

        // Run execution loop
        let mut frame_result = handler.inspect_run_exec_loop(self, first_frame_input)?;

        // Handle last frame result
        handler.last_frame_result(self, reservoir, &mut frame_result)?;

        Ok(frame_result)
    }

    fn transact_raw(&mut self, tx: Self::Tx) -> Result<ResultAndState, EVMError<DatabaseError>> {
        self.set_tx(tx);

        let result = EthEvmHandler::<I>::default().inspect_run(self)?;

        Ok(ResultAndState::new(result, self.ctx.journaled_state.inner.state.clone()))
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block> {
        self.ctx_ref().evm_clone()
    }
}
