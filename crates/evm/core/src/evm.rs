use std::{cell::RefCell, rc::Rc};

pub use crate::ic::*;
use crate::{
    backend::DatabaseExt, handler::FoundryHandler, precompiles::MaybeOdysseyPrecompiles, Env,
    InspectorExt,
};
use alloy_evm::eth::EthEvmContext;
use alloy_primitives::map::foldhash::HashMap;
use revm::{
    context::{
        result::{EVMError, HaltReason},
        Cfg, ContextTr, Evm, EvmData, JournalInner, Transaction,
    },
    handler::{
        execution,
        instructions::{EthInstructions, InstructionProvider},
        EthFrame, EvmTr, Handler, ItemOrResult,
    },
    inspector::{inspect_instructions, InspectorEvmTr},
    interpreter::{interpreter::EthInterpreter, FrameInput, Interpreter, InterpreterTypes},
    Database, Journal,
};

/// [`revm::Context`] type used by Foundry.
pub type FoundryEvmCtx<'db> = EthEvmContext<&'db mut dyn DatabaseExt>;

/// Type alias for revm's EVM used by Foundry.
pub struct FoundryEvm<'db, INSP> {
    pub inner: Evm<
        FoundryEvmCtx<'db>,
        INSP,
        EthInstructions<EthInterpreter, FoundryEvmCtx<'db>>,
        MaybeOdysseyPrecompiles,
    >,
}

/// Implementation of revm's [`Evm`] for Foundry.
impl<'db, INSP: InspectorExt> FoundryEvm<'db, INSP> {
    pub fn new(ctx: FoundryEvmCtx<'db>, inspector: INSP) -> Self {
        let is_odyssey = inspector.is_odyssey();
        let evm = Evm {
            data: EvmData { ctx, inspector },
            instruction: EthInstructions::default(),
            precompiles: MaybeOdysseyPrecompiles::new(is_odyssey),
        };
        FoundryEvm { inner: evm }
    }
}

/// Implementation of revm's [`EvmTr`] for FoundryEvm.
impl<'db, INSP: InspectorExt> EvmTr for FoundryEvm<'db, INSP> {
    type Context = FoundryEvmCtx<'db>;
    type Instructions = EthInstructions<EthInterpreter, FoundryEvmCtx<'db>>;
    type Precompiles = MaybeOdysseyPrecompiles;

    fn ctx(&mut self) -> &mut Self::Context {
        &mut self.inner.data.ctx
    }

    fn ctx_ref(&self) -> &Self::Context {
        self.inner.ctx_ref()
    }

    fn ctx_instructions(&mut self) -> (&mut Self::Context, &mut Self::Instructions) {
        self.inner.ctx_instructions()
    }

    fn run_interpreter(
        &mut self,
        interpreter: &mut Interpreter<
            <Self::Instructions as InstructionProvider>::InterpreterTypes,
        >,
    ) -> <<Self::Instructions as InstructionProvider>::InterpreterTypes as InterpreterTypes>::Output
    {
        self.inner.run_interpreter(interpreter)
    }

    fn ctx_precompiles(&mut self) -> (&mut Self::Context, &mut Self::Precompiles) {
        self.inner.ctx_precompiles()
    }
}

/// Implementation of revm's [`InspectorEvmTr`] for FoundryEvm.
impl<INSP: InspectorExt> InspectorEvmTr for FoundryEvm<'_, INSP> {
    type Inspector = INSP;

    fn inspector(&mut self) -> &mut Self::Inspector {
        self.inner.inspector()
    }

    fn ctx_inspector(&mut self) -> (&mut Self::Context, &mut Self::Inspector) {
        self.inner.ctx_inspector()
    }

    fn run_inspect_interpreter(
        &mut self,
        interpreter: &mut Interpreter<
            <Self::Instructions as InstructionProvider>::InterpreterTypes,
        >,
    ) -> <<Self::Instructions as InstructionProvider>::InterpreterTypes as InterpreterTypes>::Output
    {
        let context = &mut self.inner.data.ctx;
        let instructions = &mut self.inner.instruction;
        let inspector = &mut self.inner.data.inspector;

        inspect_instructions(context, interpreter, inspector, instructions.instruction_table())
    }
}

/// Creates a new EVM with the given context and inspector inside of a handler.
pub fn new_evm_with_inspector<'i, 'db, I: InspectorExt + ?Sized>(
    db: &'db mut dyn DatabaseExt,
    env: &Env,
    inspector: &'i mut I,
) -> FoundryHandler<'db, &'i mut I> {
    new_evm_with_context(
        FoundryEvmCtx {
            journaled_state: Journal::new_with_inner(
                db,
                JournalInner::new(env.evm_env.cfg_env.spec),
            ),
            block: env.evm_env.block_env.clone(),
            cfg: env.evm_env.cfg_env.clone(),
            tx: env.tx.clone(),
            chain: (),
            error: Ok(()),
        },
        inspector,
    )
}

/// Creates a new EVM with with the given context inside of a handler.
pub fn new_evm_with_context<'db, 'i, I: InspectorExt + ?Sized>(
    ctx: FoundryEvmCtx<'db>,
    inspector: &'i mut I,
) -> FoundryHandler<'db, &'i mut I> {
    let handler = FoundryHandler::new(ctx, inspector);
    handler
}
