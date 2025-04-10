use std::{
    cell::RefCell,
    ops::{Deref, DerefMut},
    rc::Rc,
};

use crate::{
    backend::DatabaseExt, constants::DEFAULT_CREATE2_DEPLOYER_CODEHASH, Env, EnvMut, EnvRef,
    InspectorExt,
};
use alloy_evm::{eth::EthEvmContext, EvmEnv};
use alloy_primitives::{Address, U256};
use revm::{
    context::{
        result::{EVMError, HaltReason},
        ContextTr, CreateScheme, Evm, EvmData, JournalTr,
    },
    handler::{
        instructions::{EthInstructions, InstructionProvider},
        EthFrame, EthPrecompiles, EvmTr, Frame, FrameOrResult, FrameResult, Handler,
        PrecompileProvider,
    },
    inspector::InspectorHandler,
    interpreter::{
        interpreter::EthInterpreter, return_ok, CallInputs, CallOutcome, CallScheme, CallValue,
        CreateInputs, CreateOutcome, FrameInput, Gas, Host, InputsImpl, InstructionResult,
        InterpreterResult, EMPTY_SHARED_MEMORY,
    },
    primitives::{HashMap, KECCAK_EMPTY},
    Database, Journal,
};

pub type FoundryEvmContext<'db> = EthEvmContext<&'db mut dyn DatabaseExt>;

pub type FoundryEvm<'db, I, P = FoundryPrecompiles> =
    Evm<FoundryEvmContext<'db>, I, EthInstructions<EthInterpreter, FoundryEvmContext<'db>>, P>;

/// A list of features that can be enabled or disabled in the [`FoundryHandler`].
/// This is used to conditionally override certain execution paths in the EVM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Features {
    /// Enables routing certain `CREATE2` invocations through the `CREATE2_DEPLOYER`.
    ///
    /// If [`InspectorExt::should_use_create2_factory`] returns `true`, the standard `CREATE2`
    /// hook is overridden with a `CALL` frame targeting the deployer. The handler tracks
    /// these overridden frames and, in the `insert_call_outcome` hook, inserts the decoded
    /// contract address directly into the EVM interpreter.
    Create2Factory,
}

pub struct FoundryHandler<'db, I: InspectorExt> {
    pub inner: FoundryEvm<'db, I>,

    /// A map of enabled features.
    pub enabled: HashMap<Features, bool>,

    /// A list of overridden `CREATE2` frames.
    pub create2_overrides: Rc<RefCell<Vec<(usize, CallInputs)>>>,
}

impl<'db, I> FoundryHandler<'db, I>
where
    I: InspectorExt,
{
    /// Creates a new [`FoundryHandler`] with the given EVM and inspector.
    pub fn new(ctx: FoundryEvmContext<'db>, inspector: I) -> Self {
        // By default we enable the `CREATE2` handler.
        let mut enabled = HashMap::default();
        enabled.insert(Features::Create2Factory, true);

        FoundryHandler {
            inner: FoundryEvm::new_with_inspector(
                ctx,
                inspector,
                EthInstructions::default(),
                FoundryPrecompiles::new(),
            ),
            enabled,
            create2_overrides: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Set the enabled state of a handler feature.
    pub fn set_enabled(&mut self, name: Features, enabled: bool) {
        self.enabled.insert(name, enabled);
    }

    /// Whether a handler feature is enabled or not.
    pub fn is_enabled(&self, name: Features) -> bool {
        self.enabled.get(&name).copied().unwrap_or(false)
    }

    /// Returns a reference to the environment of the EVM.
    /// This is used to access the block, transaction, and configuration data.
    pub fn env(&self) -> EnvRef<'_> {
        EnvRef {
            block: &self.inner.data.ctx.block,
            cfg: &self.inner.data.ctx.cfg,
            tx: &self.inner.data.ctx.tx,
        }
    }

    /// Returns a mutable reference to the environment of the EVM.
    /// This is used to access the block, transaction, and configuration data.
    pub fn env_mut(&mut self) -> EnvMut<'_> {
        EnvMut {
            block: &mut self.inner.data.ctx.block,
            cfg: &mut self.inner.data.ctx.cfg,
            tx: &mut self.inner.data.ctx.tx,
        }
    }

    /// Returns a reference to the inner EVM instance.
    pub fn evm(&self) -> &FoundryEvm<'db, I> {
        &self.inner
    }

    /// Returns a mutable reference to the inner EVM instance.
    pub fn evm_mut(&mut self) -> &mut FoundryEvm<'db, I> {
        &mut self.inner
    }

    /// Returns a reference to the inner DB instance.
    pub fn db(&self) -> &dyn DatabaseExt {
        &self.inner.data.ctx.journaled_state.database
    }

    /// Returns a mutable reference to the inner DB instance.
    pub fn db_mut(&mut self) -> &mut dyn DatabaseExt {
        &mut self.inner.data.ctx.journaled_state.database
    }

    /// Returns a reference to the inner inspector instance.
    /// This is used to access the inspector's methods and properties.
    pub fn inspector(&mut self) -> &mut I {
        &mut self.inner.data.inspector
    }
}

impl<'db, I> Deref for FoundryHandler<'db, I>
where
    I: InspectorExt,
{
    type Target = FoundryEvmContext<'db>;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<I> DerefMut for FoundryHandler<'_, I>
where
    I: InspectorExt,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<'db, I> Handler for FoundryHandler<'db, I>
where
    I: InspectorExt,
    FoundryEvm<'db, I>: EvmTr<
        Context = FoundryEvmContext<'db>,
        Precompiles: PrecompileProvider<FoundryEvmContext<'db>, Output = InterpreterResult>,
        Instructions: InstructionProvider<
            Context = FoundryEvmContext<'db>,
            InterpreterTypes = EthInterpreter,
        >,
    >,
{
    type Evm = FoundryEvm<'db, I>;
    type Error = EVMError<<<FoundryEvmContext<'db> as ContextTr>::Db as Database>::Error>;
    type Frame = EthFrame<
        Self::Evm,
        Self::Error,
        <<Self::Evm as EvmTr>::Instructions as InstructionProvider>::InterpreterTypes,
    >;
    type HaltReason = HaltReason;

    #[inline]
    fn first_frame_init(
        &mut self,
        evm: &mut Self::Evm,
        frame_input: <Self::Frame as Frame>::FrameInit,
    ) -> Result<FrameOrResult<Self::Frame>, Self::Error> {
        if self.is_enabled(Features::Create2Factory) {
            if let FrameInput::Create(inputs) = &frame_input {
                // Early return if we are not using CREATE2.
                let CreateScheme::Create2 { salt } = inputs.scheme else {
                    return Self::Frame::init_first(evm, frame_input);
                };

                // Early return if we should not use the CREATE2 factory.
                if !self.inspector().should_use_create2_factory(evm.ctx(), inputs) {
                    return Self::Frame::init_first(evm, frame_input);
                }

                let gas_limit = inputs.gas_limit;
                let create2_deployer = self.inspector().create2_deployer();
                let mut call_inputs: CallInputs =
                    get_create2_factory_call_inputs(salt, inputs, create2_deployer);
                let outcome = self.inspector().call(evm.ctx(), &mut call_inputs);

                self.create2_overrides
                    .borrow_mut()
                    .push((evm.journaled_state.depth, call_inputs.clone()));

                if let Some(code_hash) = evm.ctx().load_account_code_hash(create2_deployer) {
                    if code_hash.data == KECCAK_EMPTY {
                        return Ok(FrameOrResult::Result(FrameResult::Call(CallOutcome {
                            result: InterpreterResult {
                                result: InstructionResult::Revert,
                                output: format!("missing CREATE2 deployer: {create2_deployer}")
                                    .into(),
                                gas: Gas::new(gas_limit),
                            },
                            memory_offset: 0..0,
                        })))
                    } else if code_hash.data != DEFAULT_CREATE2_DEPLOYER_CODEHASH {
                        return Ok(FrameOrResult::Result(FrameResult::Call(CallOutcome {
                            result: InterpreterResult {
                                result: InstructionResult::Revert,
                                output: "invalid CREATE2 deployer bytecode".into(),
                                gas: Gas::new(gas_limit),
                            },
                            memory_offset: 0..0,
                        })))
                    }
                } else {
                    return Ok(FrameOrResult::Result(FrameResult::Call(CallOutcome {
                        result: InterpreterResult {
                            result: InstructionResult::Revert,
                            output: format!("missing CREATE2 bytecode for: {create2_deployer}")
                                .into(),
                            gas: Gas::new(gas_limit),
                        },
                        memory_offset: 0..0,
                    })))
                }

                // Handle potential inspector override.
                if let Some(outcome) = outcome {
                    return Ok(FrameOrResult::Result(FrameResult::Call(outcome)));
                }

                // Create the `CALL` frame for the `CREATE2` factory.
                let mut frame_or_result = Self::Frame::make_call_frame(
                    evm,
                    evm.journaled_state.depth,
                    Rc::new(RefCell::new(EMPTY_SHARED_MEMORY)), // TODO: this seems wrong
                    Box::new(call_inputs),
                );

                if let Ok(FrameOrResult::Item(frame)) = &mut frame_or_result {
                    self.inspector().initialize_interp(&mut frame.interpreter, evm.ctx());
                    return frame_or_result;
                }
            }
        }

        Self::Frame::init_first(evm, frame_input)
    }

    #[inline]
    fn frame_return_result(
        &mut self,
        frame: &mut Self::Frame,
        evm: &mut Self::Evm,
        result: <Self::Frame as Frame>::FrameResult,
    ) -> Result<(), Self::Error> {
        if self.is_enabled(Features::Create2Factory) &&
            self.create2_overrides
                .borrow()
                .last()
                .is_some_and(|(depth, _)| *depth == evm.ctx().journaled_state.depth)
        {
            let (_, call_inputs) = self.create2_overrides.borrow_mut().pop().unwrap();

            if let FrameResult::Call(outcome) = &result {
                let mut outcome = outcome.clone();
                self.inspector().call_end(evm.ctx(), &call_inputs, &mut outcome);
                let address = match outcome.instruction_result() {
                    return_ok!() => Address::try_from(outcome.output().as_ref())
                        .map_err(|_| {
                            outcome.result = InterpreterResult {
                                result: InstructionResult::Revert,
                                output: "invalid CREATE2 factory output".into(),
                                gas: Gas::new(call_inputs.gas_limit),
                            };
                        })
                        .ok(),
                    _ => None,
                };

                return Self::Frame::return_result(
                    frame,
                    evm,
                    FrameResult::Create(CreateOutcome { result: outcome.result, address }),
                )
            }
        }

        Self::Frame::return_result(frame, evm, result)
    }
}

/// Creates the call inputs for the CREATE2 factory call.
/// This is used to deploy a contract using the CREATE2 factory.
fn get_create2_factory_call_inputs(
    salt: U256,
    inputs: &CreateInputs,
    deployer: Address,
) -> CallInputs {
    let calldata = [&salt.to_be_bytes::<32>()[..], &inputs.init_code[..]].concat();
    CallInputs {
        input: calldata.into(),
        return_memory_offset: 0..0,
        gas_limit: inputs.gas_limit,
        bytecode_address: deployer,
        target_address: deployer,
        caller: inputs.caller,
        value: CallValue::Transfer(inputs.value),
        scheme: CallScheme::Call,
        is_static: false,
        is_eof: false,
    }
}

impl<'db, I> InspectorHandler for FoundryHandler<'db, I>
where
    I: InspectorExt,
    FoundryEvm<'db, I>: EvmTr<
        Context = FoundryEvmContext<'db>,
        Precompiles: PrecompileProvider<FoundryEvmContext<'db>, Output = InterpreterResult>,
        Instructions: InstructionProvider<
            Context = FoundryEvmContext<'db>,
            InterpreterTypes = EthInterpreter,
        >,
    >,
{
    type IT = EthInterpreter;
}

pub struct FoundryPrecompiles {
    inner: EthPrecompiles,
}

impl FoundryPrecompiles {
    pub fn new() -> Self {
        Self { inner: EthPrecompiles::default() }
    }
}

impl Default for FoundryPrecompiles {
    fn default() -> Self {
        Self::new()
    }
}

impl<CTX: ContextTr> PrecompileProvider<CTX> for FoundryPrecompiles {
    type Output = InterpreterResult;

    /// Set the spec for the precompiles.
    fn set_spec(&mut self, spec: <<CTX as ContextTr>::Cfg as revm::context::Cfg>::Spec) -> bool {
        PrecompileProvider::<CTX>::set_spec(&mut self.inner, spec)
    }

    /// Run the precompile.
    fn run(
        &mut self,
        context: &mut CTX,
        address: &Address,
        inputs: &InputsImpl,
        is_static: bool,
        gas_limit: u64,
    ) -> Result<Option<Self::Output>, String> {
        self.inner.run(context, address, inputs, is_static, gas_limit)
    }

    /// Get the warm addresses.
    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        self.inner.warm_addresses()
    }

    /// Check if the address is a precompile.
    fn contains(&self, address: &Address) -> bool {
        self.inner.contains(address)
    }
}

pub fn new_evm_with_inspector<'i, 'db, I: InspectorExt + ?Sized>(
    db: &'db mut dyn DatabaseExt,
    env: &Env,
    inspector: &'i mut I,
) -> FoundryEvm<'db, &'i mut I> {
    new_evm_with_context(
        FoundryEvmContext {
            journaled_state: {
                let mut journal = Journal::new(db);
                journal.set_spec_id(env.evm_env.cfg_env.spec);
                journal
            },
            block: env.evm_env.block_env.clone(),
            cfg: env.evm_env.cfg_env.clone(),
            tx: env.tx.clone(),
            chain: (),
            error: Ok(()),
        },
        inspector,
    )
}

pub fn new_evm_with_context<'db, 'i, I: InspectorExt + ?Sized>(
    ctx: FoundryEvmContext<'db>,
    inspector: &'i mut I,
) -> FoundryEvm<'db, &'i mut I> {
    Evm {
        data: EvmData { ctx, inspector },
        instruction: EthInstructions::default(),
        precompiles: FoundryPrecompiles::new(),
    }
}
