use alloy_evm::{
    EthEvmFactory, Evm, EvmEnv, EvmFactory, eth::EthEvmContext, precompiles::PrecompilesMap,
};
use alloy_network::Ethereum;
use alloy_primitives::{Address, Bytes};
use foundry_evm_networks::apply_bsc_p256_precompile;
use foundry_fork_db::DatabaseError;
use revm::{
    ExecuteEvm, SystemCallEvm,
    context::{
        BlockEnv, CfgEnv, Context, Evm as RevmEvm, Journal, TxEnv,
        result::{EVMError, HaltReason, ResultAndState},
    },
    handler::{EthFrame, EvmTr, FrameResult, instructions::EthInstructions},
    inspector::InspectorHandler,
    interpreter::{FrameInput, InstructionResult, interpreter::EthInterpreter},
    primitives::hardfork::SpecId,
};

use crate::{
    FoundryContextExt, FoundryInspectorExt,
    backend::{DatabaseExt, JournaledState},
    edgecov::install_hooks,
    evm::{
        FoundryEvmFactory, FoundryEvmNetwork, IntoInstructionResult, NestedEvm, NestedEvmFor,
        plain::PlainLoopHandler, run_inspected_frame,
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct EthEvmNetwork;
impl FoundryEvmNetwork for EthEvmNetwork {
    type Network = Ethereum;
    type EvmFactory = EthEvmFactory;
}

type EthEvmHandler<'db, I> =
    PlainLoopHandler<EthRevmEvm<'db, I>, EVMError<DatabaseError>, EthFrame>;

pub type EthRevmEvm<'db, I> = RevmEvm<
    EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>,
    I,
    EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>,
    PrecompilesMap,
    EthFrame,
>;

/// The Ethereum EVM of Foundry's executors: alloy's `EthEvm`, except that inspected transactions
/// run on [`PlainLoopHandler`], so frames skip the per-instruction callbacks the inspector does not
/// need.
pub struct FoundryEthEvm<'db, I> {
    inner: EthRevmEvm<'db, I>,
    inspect: bool,
}

impl<'db, I> FoundryEthEvm<'db, I> {
    /// Wraps a revm EVM; `inspect` selects whether [`Evm::transact`] calls the inspector.
    pub const fn new(inner: EthRevmEvm<'db, I>, inspect: bool) -> Self {
        Self { inner, inspect }
    }

    /// Consumes self and returns the revm EVM.
    pub fn into_inner(self) -> EthRevmEvm<'db, I> {
        self.inner
    }
}

impl<'db, I> std::ops::Deref for FoundryEthEvm<'db, I> {
    type Target = EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>;

    fn deref(&self) -> &Self::Target {
        &self.inner.ctx
    }
}

impl<I> std::ops::DerefMut for FoundryEthEvm<'_, I> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner.ctx
    }
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>> Evm
    for FoundryEthEvm<'db, I>
{
    type DB = &'db mut dyn DatabaseExt<EthEvmFactory>;
    type Tx = TxEnv;
    type Error = EVMError<DatabaseError>;
    type HaltReason = HaltReason;
    type Spec = SpecId;
    type BlockEnv = BlockEnv;
    type Precompiles = PrecompilesMap;
    type Inspector = I;

    fn block(&self) -> &BlockEnv {
        &self.inner.ctx.block
    }

    fn cfg_env(&self) -> &CfgEnv<Self::Spec> {
        &self.inner.ctx.cfg
    }

    fn chain_id(&self) -> u64 {
        self.inner.ctx.cfg.chain_id
    }

    fn transact_raw(&mut self, tx: TxEnv) -> Result<ResultAndState, Self::Error> {
        if !self.inspect {
            return self.inner.transact(tx);
        }
        self.inner.set_tx(tx);
        let output = EthEvmHandler::<I>::default().inspect_run(&mut self.inner);
        // Finalize even on error, so the warm set and state do not leak into the next
        // transaction, as revm's `inspect_tx` does.
        let state = self.inner.finalize();
        Ok(ResultAndState::new(output?, state))
    }

    fn transact_system_call(
        &mut self,
        caller: Address,
        contract: Address,
        data: Bytes,
    ) -> Result<ResultAndState, Self::Error> {
        self.inner.system_call_with_caller(caller, contract, data)
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec>) {
        let Context { block: block_env, cfg: cfg_env, journaled_state, .. } = self.inner.ctx;
        (journaled_state.database, EvmEnv { block_env, cfg_env })
    }

    fn set_inspector_enabled(&mut self, enabled: bool) {
        self.inspect = enabled;
    }

    fn components(&self) -> (&Self::DB, &Self::Inspector, &Self::Precompiles) {
        (&self.inner.ctx.journaled_state.database, &self.inner.inspector, &self.inner.precompiles)
    }

    fn components_mut(&mut self) -> (&mut Self::DB, &mut Self::Inspector, &mut Self::Precompiles) {
        (
            &mut self.inner.ctx.journaled_state.database,
            &mut self.inner.inspector,
            &mut self.inner.precompiles,
        )
    }
}

impl IntoInstructionResult for HaltReason {
    fn into_instruction_result(self) -> InstructionResult {
        self.into()
    }
}

impl FoundryEvmFactory for EthEvmFactory {
    type Chain = ();
    type FoundryContext<'db> = EthEvmContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> = FoundryEthEvm<'db, I>;

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let chain_id = evm_env.cfg_env.chain_id;
        let timestamp = evm_env.block_env.timestamp.saturating_to();
        let hooks = inspector.evm_hooks();
        let mut inner =
            Self::default().create_evm_with_inspector(db, evm_env, inspector).into_inner();
        if hooks.any() {
            install_hooks(&mut inner.instruction, hooks);
        }
        let mut eth_evm = FoundryEthEvm::new(inner, true);
        eth_evm.cfg.tx_chain_id_check = true;
        let networks = eth_evm.inspector().get_networks();
        networks.inject_precompiles(eth_evm.precompiles_mut());
        apply_bsc_p256_precompile(eth_evm.precompiles_mut(), chain_id, timestamp);
        eth_evm
    }

    fn create_nested_evm_with_inspector<'db, I>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv,
        inspector: I,
    ) -> NestedEvmFor<'db, Self>
    where
        I: FoundryInspectorExt<Self::FoundryContext<'db>> + 'db,
    {
        Box::new(self.create_foundry_evm_with_inspector(db, evm_env, inspector).into_inner())
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

    fn precompiles_mut(&mut self) -> &mut alloy_evm::precompiles::PrecompilesMap {
        &mut self.precompiles
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
