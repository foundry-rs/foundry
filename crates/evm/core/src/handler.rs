use crate::{
    constants::DEFAULT_CREATE2_DEPLOYER_CODEHASH,
    evm::{FoundryEvm, FoundryEvmCtx},
    InspectorExt,
};
use alloy_primitives::{map::foldhash::HashMap, Address, U256};
use revm::{
    context::{
        result::{EVMError, HaltReason},
        ContextTr, CreateScheme,
    },
    handler::{
        instructions::{EthInstructions, InstructionProvider},
        EthFrame, EvmTr, Frame, FrameOrResult, FrameResult, Handler,
    },
    inspector::{InspectorEvmTr, NoOpInspector},
    interpreter::{
        interpreter::EthInterpreter, return_ok, CallInputs, CallOutcome, CallScheme, CallValue,
        CreateInputs, CreateOutcome, FrameInput, Gas, Host, InstructionResult, InterpreterResult,
        EMPTY_SHARED_MEMORY,
    },
    primitives::KECCAK_EMPTY,
    Database,
};
use std::{cell::RefCell, rc::Rc};

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

/// A [`Handler`] registry for the Foundry EVM.
/// This is a wrapper around the EVM that allows us to conditionally override certain
/// execution paths based on the enabled features.
pub struct FoundryHandler<'db, INSP: InspectorExt = NoOpInspector> {
    /// The inner EVM instance.
    pub inner: FoundryEvm<'db, INSP>,

    /// A map of enabled features.
    pub enabled: HashMap<Features, bool>,
    /// A list of overridden `CREATE2` frames.
    pub create2_overrides: Rc<RefCell<Vec<(usize, CallInputs)>>>,
}

impl<'db, INSP> FoundryHandler<'db, INSP>
where
    INSP: InspectorExt,
{
    /// Creates a new [`FoundryHandler`] with the given context and inspector.
    pub fn new(ctx: FoundryEvmCtx<'db>, inspector: INSP) -> Self {
        // By default we enable the `CREATE2` handler.
        let mut enabled = HashMap::default();
        enabled.insert(Features::Create2Factory, true);

        FoundryHandler {
            inner: FoundryEvm::new(ctx, inspector),
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
}

impl<'db, INSP> Handler for FoundryHandler<'db, INSP>
where
    INSP: InspectorExt,
{
    type Evm = FoundryEvm<'db, INSP>;
    type Error = EVMError<<<FoundryEvmCtx<'db> as ContextTr>::Db as Database>::Error>;
    type Frame = EthFrame<
    Self::Evm,
    Self::Error,
    <EthInstructions<
        EthInterpreter,
        FoundryEvmCtx<'db>,
    > as InstructionProvider>::InterpreterTypes,
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
                if !self.inner.inspector().should_use_create2_factory(evm.ctx(), inputs) {
                    return Self::Frame::init_first(evm, frame_input);
                }

                let gas_limit = inputs.gas_limit;
                let create2_deployer = self.inner.inspector().create2_deployer();
                let mut call_inputs: CallInputs =
                    get_create2_factory_call_inputs(salt, inputs, create2_deployer);
                let outcome = self.inner.inspector().call(evm.ctx(), &mut call_inputs);

                self.create2_overrides
                    .borrow_mut()
                    .push((evm.inner.journaled_state.depth, call_inputs.clone()));

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
                    evm.inner.journaled_state.depth,
                    Rc::new(RefCell::new(EMPTY_SHARED_MEMORY)), // TODO: this seems wrong
                    Box::new(call_inputs),
                );

                if let Ok(FrameOrResult::Item(frame)) = &mut frame_or_result {
                    self.inner.inspector().initialize_interp(&mut frame.interpreter, evm.ctx());
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
                self.inner.inspector().call_end(evm.ctx(), &call_inputs, &mut outcome);
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
