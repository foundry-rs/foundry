use std::ops::{Deref, DerefMut};

use crate::{
    backend::DatabaseExt, constants::DEFAULT_CREATE2_DEPLOYER_CODEHASH, Env, InspectorExt,
};
use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_evm::{eth::EthEvmContext, Evm, EvmEnv};
use alloy_primitives::{Address, Bytes, U256};
use foundry_fork_db::DatabaseError;
use revm::{
    context::{
        result::{EVMError, HaltReason, ResultAndState},
        BlockEnv, CfgEnv, ContextTr, CreateScheme, Evm as RevmEvm, JournalTr, TxEnv,
    },
    handler::{
        instructions::EthInstructions, EthFrame, EthPrecompiles, FrameInitOrResult, FrameResult,
        Handler, ItemOrResult, MainnetHandler, PrecompileProvider,
    },
    inspector::InspectorHandler,
    interpreter::{
        interpreter::EthInterpreter, return_ok, CallInputs, CallOutcome, CallScheme, CallValue,
        CreateInputs, CreateOutcome, FrameInput, Gas, InputsImpl, InstructionResult,
        InterpreterResult,
    },
    primitives::hardfork::SpecId,
    Context, ExecuteEvm, Journal,
};

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
    env: Env,
    inspector: &'i mut I,
) -> FoundryEvm<'db, &'i mut I> {
    let ctx = EthEvmContext {
        journaled_state: {
            let mut journal = Journal::new(db);
            journal.set_spec_id(env.evm_env.cfg_env.spec);
            journal
        },
        block: env.evm_env.block_env,
        cfg: env.evm_env.cfg_env,
        tx: env.tx,
        chain: (),
        error: Ok(()),
    };

    FoundryEvm {
        inner: RevmEvm::new_with_inspector(
            ctx,
            inspector,
            EthInstructions::default(),
            FoundryPrecompiles::default(),
        ),
    }
}

pub fn new_evm_with_existing_context<'a>(
    ctx: EthEvmContext<&'a mut dyn DatabaseExt>,
    inspector: &'a mut dyn InspectorExt,
) -> FoundryEvm<'a, &'a mut dyn InspectorExt> {
    FoundryEvm {
        inner: RevmEvm::new_with_inspector(
            ctx,
            inspector,
            EthInstructions::default(),
            FoundryPrecompiles::default(),
        ),
    }
}

fn get_create2_factory_call_inputs(
    salt: U256,
    inputs: &CreateInputs,
    deployer: Address,
) -> CallInputs {
    let calldata = [&salt.to_be_bytes::<32>()[..], &inputs.init_code[..]].concat();
    CallInputs {
        caller: inputs.caller,
        bytecode_address: deployer,
        target_address: deployer,
        scheme: CallScheme::Call,
        value: CallValue::Transfer(inputs.value),
        input: calldata.into(),
        gas_limit: inputs.gas_limit,
        is_static: false,
        return_memory_offset: 0..0,
        is_eof: false,
    }
}

pub struct FoundryEvm<'db, I: InspectorExt> {
    #[allow(clippy::type_complexity)]
    pub inner: RevmEvm<
        EthEvmContext<&'db mut dyn DatabaseExt>,
        I,
        EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt>>,
        FoundryPrecompiles,
    >,
}

impl<I: InspectorExt> FoundryEvm<'_, I> {
    pub fn run_execution(
        &mut self,
        frame: FrameInput,
    ) -> Result<FrameResult, EVMError<DatabaseError>> {
        let mut handler = FoundryHandler::<_>::default();

        // Create first frame action
        let frame = handler.inspect_first_frame_init(&mut self.inner, frame)?;
        let frame_result = match frame {
            ItemOrResult::Item(frame) => handler.inspect_run_exec_loop(&mut self.inner, frame)?,
            ItemOrResult::Result(result) => result,
        };

        Ok(frame_result)
    }
}

impl<'db, I: InspectorExt> Evm for FoundryEvm<'db, I> {
    type DB = &'db mut dyn DatabaseExt;
    type Error = EVMError<DatabaseError>;
    type HaltReason = HaltReason;
    type Spec = SpecId;
    type Tx = TxEnv;

    fn block(&self) -> &BlockEnv {
        &self.inner.block
    }

    fn db_mut(&mut self) -> &mut Self::DB {
        self.inner.db()
    }

    fn set_inspector_enabled(&mut self, _enabled: bool) {
        unimplemented!("FoundryEvm is always inspecting")
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        let mut handler = FoundryHandler::<_>::default();
        self.inner.set_tx(tx);
        handler.inspect_run(&mut self.inner)
    }

    fn transact_system_call(
        &mut self,
        _caller: Address,
        _contract: Address,
        _data: Bytes,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        unimplemented!()
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec>)
    where
        Self: Sized,
    {
        let Context { block: block_env, cfg: cfg_env, journaled_state, .. } = self.inner.data.ctx;

        (journaled_state.database, EvmEnv { block_env, cfg_env })
    }
}

impl<'db, I: InspectorExt> Deref for FoundryEvm<'db, I> {
    type Target = Context<BlockEnv, TxEnv, CfgEnv, &'db mut dyn DatabaseExt>;

    fn deref(&self) -> &Self::Target {
        &self.inner.data.ctx
    }
}

impl<I: InspectorExt> DerefMut for FoundryEvm<'_, I> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner.data.ctx
    }
}

pub struct FoundryHandler<'db, I: InspectorExt> {
    #[allow(clippy::type_complexity)]
    inner: MainnetHandler<
        RevmEvm<
            EthEvmContext<&'db mut dyn DatabaseExt>,
            I,
            EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt>>,
            FoundryPrecompiles,
        >,
        EVMError<DatabaseError>,
        EthFrame<
            RevmEvm<
                EthEvmContext<&'db mut dyn DatabaseExt>,
                I,
                EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt>>,
                FoundryPrecompiles,
            >,
            EVMError<DatabaseError>,
            EthInterpreter,
        >,
    >,
    create2_overrides: Vec<(usize, CallInputs)>,
}

impl<I: InspectorExt> Default for FoundryHandler<'_, I> {
    fn default() -> Self {
        Self { inner: MainnetHandler::default(), create2_overrides: Vec::new() }
    }
}

impl<'db, I: InspectorExt> Handler for FoundryHandler<'db, I> {
    type Evm = RevmEvm<
        EthEvmContext<&'db mut dyn DatabaseExt>,
        I,
        EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt>>,
        FoundryPrecompiles,
    >;
    type Error = EVMError<DatabaseError>;
    type Frame = EthFrame<
        RevmEvm<
            EthEvmContext<&'db mut dyn DatabaseExt>,
            I,
            EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt>>,
            FoundryPrecompiles,
        >,
        EVMError<DatabaseError>,
        EthInterpreter,
    >;
    type HaltReason = HaltReason;

    fn frame_return_result(
        &mut self,
        frame: &mut Self::Frame,
        evm: &mut Self::Evm,
        result: <Self::Frame as revm::handler::Frame>::FrameResult,
    ) -> Result<(), Self::Error> {
        let result = if self
            .create2_overrides
            .last()
            .is_some_and(|(depth, _)| *depth == evm.journal().depth)
        {
            let (_, call_inputs) = self.create2_overrides.pop().unwrap();
            let FrameResult::Call(mut result) = result else {
                unreachable!("create2 override should be a call frame");
            };

            // Decode address from output.
            let address = match result.instruction_result() {
                return_ok!() => Address::try_from(result.output().as_ref())
                    .map_err(|_| {
                        result.result = InterpreterResult {
                            result: InstructionResult::Revert,
                            output: "invalid CREATE2 factory output".into(),
                            gas: Gas::new(call_inputs.gas_limit),
                        };
                    })
                    .ok(),
                _ => None,
            };

            FrameResult::Create(CreateOutcome { result: result.result, address })
        } else {
            result
        };

        self.inner.frame_return_result(frame, evm, result)
    }
}

impl<I: InspectorExt> InspectorHandler for FoundryHandler<'_, I> {
    type IT = EthInterpreter;

    fn inspect_frame_call(
        &mut self,
        frame: &mut Self::Frame,
        evm: &mut Self::Evm,
    ) -> Result<FrameInitOrResult<Self::Frame>, Self::Error> {
        let frame_or_result = self.inner.inspect_frame_call(frame, evm)?;

        let ItemOrResult::Item(FrameInput::Create(inputs)) = &frame_or_result else {
            return Ok(frame_or_result)
        };

        let CreateScheme::Create2 { salt } = inputs.scheme else { return Ok(frame_or_result) };

        if !evm.data.inspector.should_use_create2_factory(&mut evm.data.ctx, inputs) {
            return Ok(frame_or_result)
        }

        let gas_limit = inputs.gas_limit;

        // Get CREATE2 deployer.
        let create2_deployer = evm.data.inspector.create2_deployer();
        // Generate call inputs for CREATE2 factory.
        let call_inputs = get_create2_factory_call_inputs(salt, inputs, create2_deployer);

        // Push data about current override to the stack.
        self.create2_overrides.push((evm.journal().depth(), call_inputs.clone()));

        // Sanity check that CREATE2 deployer exists.
        let code_hash = evm.journal().load_account(create2_deployer)?.info.code_hash;
        if code_hash == KECCAK_EMPTY {
            return Ok(ItemOrResult::Result(FrameResult::Call(CallOutcome {
                result: InterpreterResult {
                    result: InstructionResult::Revert,
                    output: format!("missing CREATE2 deployer: {create2_deployer}").into(),
                    gas: Gas::new(gas_limit),
                },
                memory_offset: 0..0,
            })))
        } else if code_hash != DEFAULT_CREATE2_DEPLOYER_CODEHASH {
            return Ok(ItemOrResult::Result(FrameResult::Call(CallOutcome {
                result: InterpreterResult {
                    result: InstructionResult::Revert,
                    output: "invalid CREATE2 deployer bytecode".into(),
                    gas: Gas::new(gas_limit),
                },
                memory_offset: 0..0,
            })))
        }

        // Return the created CALL frame instead
        Ok(ItemOrResult::Item(FrameInput::Call(Box::new(call_inputs))))
    }
}
