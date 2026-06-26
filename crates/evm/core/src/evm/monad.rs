use alloy_evm::{Evm, EvmEnv, EvmFactory};
use alloy_monad_evm::{MonadEvm, MonadEvmFactory, MonadPrecompilesMap};
use foundry_fork_db::DatabaseError;
use monad_revm::{
    MonadBuilder, MonadCfgEnv, MonadContext, MonadEvm as RevmMonadEvm, MonadHardfork,
    handler::MonadHandler, instructions::MonadInstructions, monad_context_with_db,
};
use revm::{
    context::{
        BlockEnv, ContextTr, LocalContextTr, TxEnv,
        result::{EVMError, ResultAndState},
    },
    context_interface::ContextSetters,
    handler::{EthFrame, EvmTr, FrameResult, Handler},
    inspector::InspectorHandler,
    interpreter::{FrameInput, SharedMemory, interpreter_action::FrameInit},
};

use crate::{
    FoundryContextExt, FoundryInspectorExt,
    backend::{DatabaseExt, JournaledState},
    evm::{FoundryEvmFactory, NestedEvm},
};

type MonadEvmHandler<'db, I> =
    MonadHandler<MonadRevmEvm<'db, I>, EVMError<DatabaseError>, EthFrame>;

pub type MonadRevmEvm<'db, I> = RevmMonadEvm<
    MonadContext<&'db mut dyn DatabaseExt<MonadEvmFactory>>,
    I,
    MonadInstructions<MonadContext<&'db mut dyn DatabaseExt<MonadEvmFactory>>>,
    MonadPrecompilesMap,
>;

impl FoundryEvmFactory for MonadEvmFactory {
    type FoundryContext<'db> = MonadContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> =
        MonadEvm<&'db mut dyn DatabaseExt<Self>, I>;

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let mut monad_evm = self.create_evm_with_inspector(db, evm_env, inspector);
        monad_evm.cfg.tx_chain_id_check = true;
        monad_evm.inspector().get_networks().inject_precompiles(monad_evm.precompiles_mut());
        monad_evm
    }

    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> Box<dyn NestedEvm<Spec = MonadHardfork, Block = BlockEnv, Tx = TxEnv> + 'db> {
        let spec = evm_env.cfg_env.spec;
        let monad_cfg = MonadCfgEnv::from(evm_env.cfg_env);
        let mut evm = monad_context_with_db(db)
            .with_block(evm_env.block_env)
            .with_cfg(monad_cfg)
            .build_monad_with_inspector(inspector)
            .with_precompiles(MonadPrecompilesMap::new_with_spec(spec));

        evm.0.ctx.cfg.tx_chain_id_check = true;
        evm.0.inspector.get_networks().inject_precompiles(&mut evm.0.precompiles);

        Box::new(evm)
    }
}

impl<'db, I: FoundryInspectorExt<MonadContext<&'db mut dyn DatabaseExt<MonadEvmFactory>>>> NestedEvm
    for MonadRevmEvm<'db, I>
{
    type Spec = MonadHardfork;
    type Block = BlockEnv;
    type Tx = TxEnv;

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx_mut().journaled_state.inner
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        let mut handler = MonadEvmHandler::<I>::new();
        let reservoir = frame.reservoir();

        let memory =
            SharedMemory::new_with_buffer(self.ctx_ref().local().shared_memory_buffer().clone());
        let first_frame_input = FrameInit { depth: 0, memory, frame_input: frame };

        let mut frame_result = handler.inspect_run_exec_loop(self, first_frame_input)?;

        handler.last_frame_result(self, reservoir, &mut frame_result)?;

        Ok(frame_result)
    }

    fn transact_raw(&mut self, tx: Self::Tx) -> Result<ResultAndState, EVMError<DatabaseError>> {
        ContextSetters::set_tx(&mut self.0.ctx, tx);

        let mut handler = MonadEvmHandler::<I>::new();
        let result = handler.inspect_run(self)?;

        Ok(ResultAndState::new(result, self.ctx_ref().journaled_state.inner.state.clone()))
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block> {
        self.ctx_ref().evm_clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monad_evm_factory_implements_foundry_evm_factory() {
        fn assert_foundry_factory<F: FoundryEvmFactory>() {}

        assert_foundry_factory::<MonadEvmFactory>();
    }
}
