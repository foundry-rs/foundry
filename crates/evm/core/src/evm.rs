use crate::{
    backend::DatabaseExt, handler::FoundryHandler, precompiles::ODYSSEY_P256, Env, InspectorExt,
};
use alloy_evm::eth::EthEvmContext;
use alloy_primitives::{Address, Bytes};
use revm::{
    context::{ContextTr, Evm, EvmData, JournalInner},
    handler::{
        instructions::{EthInstructions, InstructionProvider},
        EthPrecompiles, EvmTr, PrecompileProvider,
    },
    inspector::{inspect_instructions, InspectorEvmTr},
    interpreter::{
        interpreter::EthInterpreter, Gas, InstructionResult, Interpreter, InterpreterResult,
        InterpreterTypes,
    },
    precompile::PrecompileError,
    Journal,
};

/// [`revm::Context`] type used by Foundry.
pub type FoundryEvmCtx<'db> = EthEvmContext<&'db mut dyn DatabaseExt>;

/// Type alias for revm's EVM used by Foundry.
pub struct FoundryEvm<'db, INSP = ()> {
    pub inner: Evm<
        FoundryEvmCtx<'db>,
        INSP,
        EthInstructions<EthInterpreter, FoundryEvmCtx<'db>>,
        FoundryPrecompiles,
    >,
}

/// Implementation of revm's [`Evm`] for Foundry.
impl<'db, INSP: InspectorExt> FoundryEvm<'db, INSP> {
    pub fn new(ctx: FoundryEvmCtx<'db>, inspector: INSP) -> Self {
        let is_odyssey = inspector.is_odyssey();
        let evm = Evm {
            data: EvmData { ctx, inspector },
            instruction: EthInstructions::default(),
            precompiles: FoundryPrecompiles::new(is_odyssey),
        };
        FoundryEvm { inner: evm }
    }
}

/// Implementation of revm's [`EvmTr`] for FoundryEvm.
impl<'db, INSP: InspectorExt> EvmTr for FoundryEvm<'db, INSP> {
    type Context = FoundryEvmCtx<'db>;
    type Instructions = EthInstructions<EthInterpreter, FoundryEvmCtx<'db>>;
    type Precompiles = FoundryPrecompiles;

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
    FoundryHandler::new(
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
    FoundryHandler::new(ctx, inspector)
}

/// [`PrecompileProvider`] wrapper for Foundry's precompiles.
/// Adds support for:
/// - [`ODYSSEY_P256`], if `odyssey` is enabled.
pub struct FoundryPrecompiles {
    inner: EthPrecompiles,
    odyssey: bool,
}

impl FoundryPrecompiles {
    /// Creates a new instance of the [`FoundryPrecompiles`].
    pub fn new(odyssey: bool) -> Self {
        Self { inner: EthPrecompiles::default(), odyssey }
    }
}

impl<CTX: ContextTr> PrecompileProvider<CTX> for FoundryPrecompiles {
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: <<CTX as ContextTr>::Cfg as revm::context::Cfg>::Spec) {
        PrecompileProvider::<CTX>::set_spec(&mut self.inner, spec);
    }

    fn run(
        &mut self,
        context: &mut CTX,
        address: &Address,
        bytes: &Bytes,
        gas_limit: u64,
    ) -> Result<Option<Self::Output>, String> {
        if self.odyssey && address == ODYSSEY_P256.address() {
            let mut result = InterpreterResult {
                result: InstructionResult::Return,
                gas: Gas::new(gas_limit),
                output: Bytes::new(),
            };

            match ODYSSEY_P256.precompile()(bytes, gas_limit) {
                Ok(output) => {
                    let underflow = result.gas.record_cost(output.gas_used);
                    if underflow {
                        result.result = InstructionResult::PrecompileOOG;
                    } else {
                        result.result = InstructionResult::Return;
                        result.output = output.bytes;
                    }
                }
                Err(e) => {
                    if let PrecompileError::Fatal(_) = e {
                        return Err(e.to_string());
                    }
                    result.result = if e.is_oog() {
                        InstructionResult::PrecompileOOG
                    } else {
                        InstructionResult::PrecompileError
                    };
                }
            }
        }

        self.inner.run(context, address, bytes, gas_limit)
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        let warm_addresses = self.inner.warm_addresses() as Box<dyn Iterator<Item = Address>>;

        let iter = if self.odyssey {
            Box::new(warm_addresses.chain(core::iter::once(*ODYSSEY_P256.address())))
        } else {
            warm_addresses
        };

        Box::new(iter)
    }

    fn contains(&self, address: &Address) -> bool {
        if self.odyssey && address == ODYSSEY_P256.address() {
            true
        } else {
            self.inner.contains(address)
        }
    }
}
