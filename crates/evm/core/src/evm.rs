pub use crate::ic::*;
use crate::{backend::DatabaseExt, precompiles::MaybeOdysseyPrecompiles, Env, InspectorExt};
use alloy_evm::eth::EthEvmContext;
use revm::{
    context::{Evm, EvmData, JournalInner},
    handler::{
        instructions::{EthInstructions, InstructionProvider},
        EvmTr,
    },
    inspector::{inspect_instructions, InspectorEvmTr},
    interpreter::{interpreter::EthInterpreter, Interpreter, InterpreterTypes},
    Journal,
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

/// Creates a new EVM with the given inspector.
pub fn new_evm_with_inspector<'i, 'db, I: InspectorExt + ?Sized>(
    db: &'db mut dyn DatabaseExt,
    env: &mut Env,
    inspector: &'i mut I,
) -> FoundryEvm<'db, &'i mut I> {
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

/// Creates a new EVM with the given context.
pub fn new_evm_with_context<'db, 'i, I: InspectorExt + ?Sized>(
    ctx: FoundryEvmCtx<'db>,
    inspector: &'i mut I,
) -> FoundryEvm<'db, &'i mut I> {
    // handler.append_handler_register_plain(create2_handler_register);

    FoundryEvm::new(ctx, inspector)
}

//  Used for routing certain CREATE2 invocations through CREATE2_DEPLOYER.
//
//  Overrides create hook with CALL frame if [InspectorExt::should_use_create2_factory] returns
//  true. Keeps track of overridden frames and handles outcome in the overridden
//  insert_call_outcome hook by inserting decoded address directly into interpreter.
//
//  Should be installed after [revm::inspector_handle_register] and before any other registers.
// pub fn create2_handler_register<I: InspectorExt>(
//     handler: &mut Handler<'_, I, &mut dyn DatabaseExt>,
// ) {
//     let create2_overrides = Rc::<RefCell<Vec<_>>>::new(RefCell::new(Vec::new()));

//     let create2_overrides_inner = create2_overrides.clone();
//     let old_handle = handler.execution.create.clone();
//     handler.execution.create =
//         Arc::new(move |ctx, mut inputs| -> Result<FrameOrResult, EVMError<DatabaseError>> {
//             let CreateScheme::Create2 { salt } = inputs.scheme else {
//                 return old_handle(ctx, inputs);
//             };
//             if !ctx.external.should_use_create2_factory(&mut ctx.evm, &mut inputs) {
//                 return old_handle(ctx, inputs);
//             }

//             let gas_limit = inputs.gas_limit;

//             // Get CREATE2 deployer.
//             let create2_deployer = ctx.external.create2_deployer();
//             // Generate call inputs for CREATE2 factory.
//             let mut call_inputs = get_create2_factory_call_inputs(salt, *inputs,
// create2_deployer);

//             // Call inspector to change input or return outcome.
//             let outcome = ctx.external.call(&mut ctx.evm, &mut call_inputs);

//             // Push data about current override to the stack.
//             create2_overrides_inner
//                 .borrow_mut()
//                 .push((ctx.evm.journaled_state.depth(), call_inputs.clone()));

//             // Sanity check that CREATE2 deployer exists.
//             let code_hash = ctx.evm.load_account(create2_deployer)?.info.code_hash;
//             if code_hash == KECCAK_EMPTY {
//                 return Ok(FrameOrResult::Result(FrameResult::Call(CallOutcome {
//                     result: InterpreterResult {
//                         result: InstructionResult::Revert,
//                         output: format!("missing CREATE2 deployer: {create2_deployer}").into(),
//                         gas: Gas::new(gas_limit),
//                     },
//                     memory_offset: 0..0,
//                 })))
//             } else if code_hash != DEFAULT_CREATE2_DEPLOYER_CODEHASH {
//                 return Ok(FrameOrResult::Result(FrameResult::Call(CallOutcome {
//                     result: InterpreterResult {
//                         result: InstructionResult::Revert,
//                         output: "invalid CREATE2 deployer bytecode".into(),
//                         gas: Gas::new(gas_limit),
//                     },
//                     memory_offset: 0..0,
//                 })))
//             }

//             // Handle potential inspector override.
//             if let Some(outcome) = outcome {
//                 return Ok(FrameOrResult::Result(FrameResult::Call(outcome)));
//             }

//             // Create CALL frame for CREATE2 factory invocation.
//             let mut frame_or_result = ctx.evm.make_call_frame(&call_inputs);

//             if let Ok(FrameOrResult::Item(frame)) = &mut frame_or_result {
//                 ctx.external
//                     .initialize_interp(&mut frame.frame_data_mut().interpreter, &mut ctx.evm)
//             }
//             frame_or_result
//         });

//     let create2_overrides_inner = create2_overrides;
//     let old_handle = handler.execution.insert_call_outcome.clone();
//     handler.execution.insert_call_outcome =
//         Arc::new(move |ctx, frame, shared_memory, mut outcome| {
//             // If we are on the depth of the latest override, handle the outcome.
//             if create2_overrides_inner
//                 .borrow()
//                 .last()
//                 .is_some_and(|(depth, _)| *depth == ctx.evm.journaled_state.depth())
//             {
//                 let (_, call_inputs) = create2_overrides_inner.borrow_mut().pop().unwrap();
//                 outcome = ctx.external.call_end(&mut ctx.evm, &call_inputs, outcome);

//                 // Decode address from output.
//                 let address = match outcome.instruction_result() {
//                     return_ok!() => Address::try_from(outcome.output().as_ref())
//                         .map_err(|_| {
//                             outcome.result = InterpreterResult {
//                                 result: InstructionResult::Revert,
//                                 output: "invalid CREATE2 factory output".into(),
//                                 gas: Gas::new(call_inputs.gas_limit),
//                             };
//                         })
//                         .ok(),
//                     _ => None,
//                 };
//                 frame
//                     .frame_data_mut()
//                     .interpreter
//                     .insert_create_outcome(CreateOutcome { address, result: outcome.result });

//                 Ok(())
//             } else {
//                 old_handle(ctx, frame, shared_memory, outcome)
//             }
//         });
// }
