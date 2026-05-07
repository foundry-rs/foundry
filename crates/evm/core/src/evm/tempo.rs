use alloy_evm::{Evm, EvmEnv, EvmFactory};
use alloy_primitives::Bytes;
use foundry_evm_hardforks::TempoHardfork;
use foundry_fork_db::DatabaseError;
use revm::{
    context::{
        ContextTr, LocalContextTr,
        result::{EVMError, HaltReason, ResultAndState},
    },
    handler::{EvmTr, FrameResult, Handler},
    inspector::InspectorHandler,
    interpreter::{FrameInput, SharedMemory, interpreter_action::FrameInit},
    state::Bytecode,
};
use tempo_evm::{TempoBlockEnv, TempoEvmFactory, TempoHaltReason, evm::TempoEvm};
use tempo_precompiles::storage::StorageCtx;
use tempo_revm::{
    TempoInvalidTransaction, TempoTxEnv, evm::TempoContext, gas_params::tempo_gas_params,
    handler::TempoEvmHandler,
};

use crate::{
    FoundryContextExt, FoundryInspectorExt,
    backend::{DatabaseExt, JournaledState},
    constants::{CALLER, TEST_CONTRACT_ADDRESS},
    evm::{FoundryEvmFactory, NestedEvm},
    tempo::{TEMPO_PRECOMPILE_ADDRESSES, TEMPO_TIP20_TOKENS, initialize_tempo_genesis_inner},
};

// Will be removed when the next revm release includes bluealloy/revm#3518.
pub type TempoRevmEvm<'db, I> = tempo_revm::TempoEvm<&'db mut dyn DatabaseExt<TempoEvmFactory>, I>;

/// Initialize Tempo precompiles and contracts for a newly created EVM.
///
/// In non-fork mode, runs full genesis initialization (precompile sentinel bytecode,
/// TIP20 fee tokens, standard contracts) via [`StorageCtx::enter_evm`].
///
/// In fork mode, warms up precompile and TIP20 token addresses with sentinel bytecode
/// to prevent repeated RPC round-trips for addresses that are Rust-native precompiles
/// on Tempo nodes (no real EVM bytecode on-chain).
pub(crate) fn initialize_tempo_evm<
    'db,
    I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>,
>(
    evm: &mut TempoEvm<&'db mut dyn DatabaseExt<TempoEvmFactory>, I>,
    is_forked: bool,
) {
    let ctx = evm.ctx_mut();
    StorageCtx::enter_evm(&mut ctx.journaled_state, &ctx.block, &ctx.cfg, &ctx.tx, || {
        if is_forked {
            // In fork mode, warm up precompile accounts to avoid repeated RPC fetches.
            let mut sctx = StorageCtx;
            let sentinel = Bytecode::new_legacy(Bytes::from_static(&[0xef]));
            for addr in TEMPO_PRECOMPILE_ADDRESSES.iter().chain(TEMPO_TIP20_TOKENS.iter()) {
                sctx.set_code(*addr, sentinel.clone())
                    .expect("failed to warm tempo precompile address");
            }
        } else {
            // In non-fork mode, run full genesis initialization.
            initialize_tempo_genesis_inner(TEST_CONTRACT_ADDRESS, CALLER)
                .expect("tempo genesis initialization failed");
        }
    });
}

impl FoundryEvmFactory for TempoEvmFactory {
    type FoundryContext<'db> = TempoContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> =
        TempoEvm<&'db mut dyn DatabaseExt<Self>, I>;

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let is_forked = db.is_forked_mode();
        let spec = *evm_env.spec_id();
        let mut tempo_evm = Self::default().create_evm_with_inspector(db, evm_env, inspector);
        tempo_evm.cfg.gas_params = tempo_gas_params(spec);
        tempo_evm.cfg.tx_chain_id_check = true;
        if tempo_evm.cfg.tx_gas_limit_cap.is_none() {
            tempo_evm.cfg.tx_gas_limit_cap = spec.tx_gas_limit_cap();
        }

        let networks = tempo_evm.inspector().get_networks();
        networks.inject_precompiles(tempo_evm.precompiles_mut());

        initialize_tempo_evm(&mut tempo_evm, is_forked);
        tempo_evm
    }

    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> Box<dyn NestedEvm<Spec = TempoHardfork, Block = TempoBlockEnv, Tx = TempoTxEnv> + 'db>
    {
        Box::new(self.create_foundry_evm_with_inspector(db, evm_env, inspector).into_inner())
    }
}

/// Maps a Tempo [`EVMError`] to the common `EVMError<DatabaseError>` used by [`NestedEvm`].
///
/// This exists because [`NestedEvm`] currently uses Eth-typed errors. When `NestedEvm` gains
/// an associated `Error` type, this mapping can be removed.
pub(crate) fn map_tempo_error(
    e: EVMError<DatabaseError, TempoInvalidTransaction>,
) -> EVMError<DatabaseError> {
    match e {
        EVMError::Database(db) => EVMError::Database(db),
        EVMError::Header(h) => EVMError::Header(h),
        EVMError::Custom(s) => EVMError::Custom(s),
        EVMError::CustomAny(custom_any_error) => EVMError::CustomAny(custom_any_error),
        EVMError::Transaction(t) => match t {
            TempoInvalidTransaction::EthInvalidTransaction(eth) => EVMError::Transaction(eth),
            t => EVMError::Custom(format!("tempo transaction error: {t}")),
        },
    }
}

impl<'db, I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>> NestedEvm
    for TempoRevmEvm<'db, I>
{
    type Spec = TempoHardfork;
    type Block = TempoBlockEnv;
    type Tx = TempoTxEnv;

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx_mut().journaled_state.inner
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        let mut handler = TempoEvmHandler::new();

        let memory =
            SharedMemory::new_with_buffer(self.ctx_ref().local().shared_memory_buffer().clone());
        let first_frame_input = FrameInit { depth: 0, memory, frame_input: frame };

        let mut frame_result =
            handler.inspect_run_exec_loop(self, first_frame_input).map_err(map_tempo_error)?;

        handler.last_frame_result(self, &mut frame_result).map_err(map_tempo_error)?;

        Ok(frame_result)
    }

    fn transact_raw(&mut self, tx: Self::Tx) -> Result<ResultAndState, EVMError<DatabaseError>> {
        self.set_tx(tx);

        let mut handler = TempoEvmHandler::new();
        let result = handler.inspect_run(self).map_err(map_tempo_error)?;

        let result = result.map_haltreason(|h| match h {
            TempoHaltReason::Ethereum(eth) => eth,
            _ => HaltReason::PrecompileError,
        });

        Ok(ResultAndState::new(result, self.ctx.journaled_state.inner.state.clone()))
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block> {
        self.ctx_ref().evm_clone()
    }
}
