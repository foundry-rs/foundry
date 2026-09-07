use alloy_evm::{
    EthEvm, EthEvmFactory, Evm, EvmEnv, EvmFactory, eth::EthEvmContext, precompiles::PrecompilesMap,
};
use foundry_evm_networks::apply_bsc_p256_precompile;
use foundry_fork_db::DatabaseError;
use revm::{
    context::{
        BlockEnv, Evm as RevmEvm, Journal, TxEnv,
        result::{EVMError, ResultAndState},
    },
    handler::{EthFrame, EvmTr, FrameResult, MainnetHandler, instructions::EthInstructions},
    inspector::InspectorHandler,
    interpreter::{FrameInput, interpreter::EthInterpreter},
    primitives::hardfork::SpecId,
};

use crate::{
    FoundryContextExt, FoundryInspectorExt,
    backend::{DatabaseExt, JournaledState},
    evm::{FoundryEvmFactory, NestedEvm, NestedEvmFor, run_inspected_frame},
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
    type Chain = ();
    type FoundryContext<'db> = EthEvmContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> =
        EthEvm<&'db mut dyn DatabaseExt<Self>, I, Self::Precompiles>;

    fn create_evm_with_context<DB: alloy_evm::Database>(
        &self,
        db: DB,
        evm_env: EvmEnv,
        _chain_context: Self::Chain,
    ) -> Self::Evm<DB, revm::inspector::NoOpInspector> {
        self.create_evm(db, evm_env)
    }

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv,
        _chain_context: Self::Chain,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let chain_id = evm_env.cfg_env.chain_id;
        let timestamp = evm_env.block_env.timestamp.saturating_to();
        let mut eth_evm = Self::default().create_evm_with_inspector(db, evm_env, inspector);
        eth_evm.cfg.tx_chain_id_check = true;
        let networks = eth_evm.inspector().get_networks();
        networks.inject_precompiles(eth_evm.precompiles_mut());
        apply_bsc_p256_precompile(eth_evm.precompiles_mut(), chain_id, timestamp);
        eth_evm
    }

    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv,
        chain_context: Self::Chain,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> NestedEvmFor<'db, Self> {
        Box::new(
            self.create_foundry_evm_with_inspector(db, evm_env, chain_context, inspector)
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
    type Chain = ();
    type Journal = Journal<&'db mut dyn DatabaseExt<EthEvmFactory>>;

    fn tx_mut(&mut self) -> &mut Self::Tx {
        self.ctx_mut().tx_mut()
    }

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx_mut().journaled_state.inner
    }

    fn chain_mut(&mut self) -> &mut Self::Chain {
        &mut self.ctx_mut().chain
    }

    fn journal_mut(&mut self) -> &mut Self::Journal {
        &mut self.ctx_mut().journaled_state
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        run_inspected_frame(self, EthEvmHandler::<I>::default(), frame)
    }

    fn transact_raw(&mut self, tx: Self::Tx) -> eyre::Result<ResultAndState> {
        self.set_tx(tx);

        let result = EthEvmHandler::<I>::default().inspect_run(self)?;

        Ok(ResultAndState::new(result, self.ctx.journaled_state.inner.state.clone()))
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block> {
        self.ctx_ref().evm_clone()
    }
}
