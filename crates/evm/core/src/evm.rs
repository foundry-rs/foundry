use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::{
    Env, InspectorExt, backend::DatabaseExt, constants::DEFAULT_CREATE2_DEPLOYER_CODEHASH,
};
use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_evm::{
    Evm, EvmEnv,
    eth::EthEvmContext,
    precompiles::{DynPrecompile, PrecompileInput, PrecompilesMap},
};
use alloy_primitives::{Address, Bytes, U256};
use foundry_fork_db::DatabaseError;
use revm::{
    Context, Journal,
    context::{
        BlockEnv, CfgEnv, ContextTr, CreateScheme, Evm as RevmEvm, JournalTr, LocalContext,
        LocalContextTr, TxEnv,
        result::{EVMError, ExecResultAndState, ExecutionResult, HaltReason, ResultAndState},
    },
    handler::{
        EthFrame, EthPrecompiles, EvmTr, FrameResult, FrameTr, Handler, ItemOrResult,
        instructions::EthInstructions,
    },
    inspector::{InspectorEvmTr, InspectorHandler},
    interpreter::{
        CallInput, CallInputs, CallOutcome, CallScheme, CallValue, CreateInputs, CreateOutcome,
        FrameInput, Gas, InstructionResult, InterpreterResult, SharedMemory,
        interpreter::EthInterpreter, interpreter_action::FrameInit, return_ok,
    },
    precompile::{
        PrecompileSpecId, Precompiles,
        secp256r1::{P256VERIFY, P256VERIFY_BASE_GAS_FEE},
    },
    primitives::hardfork::SpecId,
};

pub fn new_evm_with_inspector<'db, I: InspectorExt>(
    db: &'db mut dyn DatabaseExt,
    env: Env,
    inspector: I,
) -> FoundryEvm<'db, I> {
    let mut ctx = EthEvmContext {
        journaled_state: {
            let mut journal = Journal::new(db);
            journal.set_spec_id(env.evm_env.cfg_env.spec);
            journal
        },
        block: env.evm_env.block_env,
        cfg: env.evm_env.cfg_env,
        tx: env.tx,
        chain: (),
        local: LocalContext::default(),
        error: Ok(()),
    };
    ctx.cfg.tx_chain_id_check = true;
    let spec = ctx.cfg.spec;

    let mut evm = FoundryEvm {
        inner: RevmEvm::new_with_inspector(
            ctx,
            inspector,
            EthInstructions::default(),
            get_precompiles(spec),
        ),
    };

    inject_precompiles(&mut evm);

    evm
}

pub fn new_evm_with_existing_context<'a>(
    ctx: EthEvmContext<&'a mut dyn DatabaseExt>,
    inspector: &'a mut dyn InspectorExt,
) -> FoundryEvm<'a, &'a mut dyn InspectorExt> {
    let spec = ctx.cfg.spec;

    let mut evm = FoundryEvm {
        inner: RevmEvm::new_with_inspector(
            ctx,
            inspector,
            EthInstructions::default(),
            get_precompiles(spec),
        ),
    };

    inject_precompiles(&mut evm);

    evm
}

/// Conditionally inject additional precompiles into the EVM context.
fn inject_precompiles(evm: &mut FoundryEvm<'_, impl InspectorExt>) {
    if evm.inspector().is_odyssey() {
        evm.precompiles_mut().apply_precompile(P256VERIFY.address(), |_| {
            // Create a wrapper function that adapts the new API
            let precompile_fn = |input: PrecompileInput<'_>| -> Result<_, _> {
                P256VERIFY.precompile()(input.data, P256VERIFY_BASE_GAS_FEE)
            };
            Some(DynPrecompile::from(precompile_fn))
        });
    }
}

/// Get the precompiles for the given spec.
fn get_precompiles(spec: SpecId) -> PrecompilesMap {
    PrecompilesMap::from_static(
        EthPrecompiles {
            precompiles: Precompiles::new(PrecompileSpecId::from_spec_id(spec)),
            spec,
        }
        .precompiles,
    )
}

/// Get the call inputs for the CREATE2 factory.
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
        input: CallInput::Bytes(calldata.into()),
        gas_limit: inputs.gas_limit,
        is_static: false,
        return_memory_offset: 0..0,
    }
}

pub struct FoundryEvm<'db, I: InspectorExt> {
    #[allow(clippy::type_complexity)]
    pub inner: RevmEvm<
        EthEvmContext<&'db mut dyn DatabaseExt>,
        I,
        EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt>>,
        PrecompilesMap,
        EthFrame<EthInterpreter>,
    >,
}
impl<I: InspectorExt> FoundryEvm<'_, I> {
    pub fn run_execution(
        &mut self,
        frame: FrameInput,
    ) -> Result<FrameResult, EVMError<DatabaseError>> {
        let mut handler = FoundryHandler::<I>::default();

        // Create first frame
        let memory =
            SharedMemory::new_with_buffer(self.inner.ctx().local().shared_memory_buffer().clone());
        let first_frame_input = FrameInit { depth: 0, memory, frame_input: frame };

        // Run execution loop
        let mut frame_result = handler.inspect_run_exec_loop(&mut self.inner, first_frame_input)?;

        // Handle last frame result
        handler.last_frame_result(&mut self.inner, &mut frame_result)?;

        Ok(frame_result)
    }
}

impl<'db, I: InspectorExt> Evm for FoundryEvm<'db, I> {
    type Precompiles = PrecompilesMap;
    type Inspector = I;
    type DB = &'db mut dyn DatabaseExt;
    type Error = EVMError<DatabaseError>;
    type HaltReason = HaltReason;
    type Spec = SpecId;
    type Tx = TxEnv;

    fn block(&self) -> &BlockEnv {
        &self.inner.block
    }

    fn chain_id(&self) -> u64 {
        self.inner.ctx.cfg.chain_id
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

    fn db_mut(&mut self) -> &mut Self::DB {
        &mut self.inner.ctx.journaled_state.database
    }

    fn precompiles(&self) -> &Self::Precompiles {
        &self.inner.precompiles
    }

    fn precompiles_mut(&mut self) -> &mut Self::Precompiles {
        &mut self.inner.precompiles
    }

    fn inspector(&self) -> &Self::Inspector {
        &self.inner.inspector
    }

    fn inspector_mut(&mut self) -> &mut Self::Inspector {
        &mut self.inner.inspector
    }

    fn set_inspector_enabled(&mut self, _enabled: bool) {
        unimplemented!("FoundryEvm is always inspecting")
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        self.inner.ctx.tx = tx;

        let mut handler = FoundryHandler::<I>::default();
        let result = handler.inspect_run(&mut self.inner)?;

        Ok(ResultAndState::new(result, self.inner.ctx.journaled_state.inner.state.clone()))
    }

    fn transact_system_call(
        &mut self,
        _caller: Address,
        _contract: Address,
        _data: Bytes,
    ) -> Result<ExecResultAndState<ExecutionResult>, Self::Error> {
        unimplemented!()
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec>)
    where
        Self: Sized,
    {
        let Context { block: block_env, cfg: cfg_env, journaled_state, .. } = self.inner.ctx;

        (journaled_state.database, EvmEnv { block_env, cfg_env })
    }
}

impl<'db, I: InspectorExt> Deref for FoundryEvm<'db, I> {
    type Target = Context<BlockEnv, TxEnv, CfgEnv, &'db mut dyn DatabaseExt>;

    fn deref(&self) -> &Self::Target {
        &self.inner.ctx
    }
}

impl<I: InspectorExt> DerefMut for FoundryEvm<'_, I> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner.ctx
    }
}

pub struct FoundryHandler<'db, I: InspectorExt> {
    create2_overrides: Vec<(usize, CallInputs)>,
    _phantom: PhantomData<(&'db mut dyn DatabaseExt, I)>,
}

impl<I: InspectorExt> Default for FoundryHandler<'_, I> {
    fn default() -> Self {
        Self { create2_overrides: Vec::new(), _phantom: PhantomData }
    }
}

// Blanket Handler implementation for FoundryHandler, needed for implementing the InspectorHandler
// trait.
impl<'db, I: InspectorExt> Handler for FoundryHandler<'db, I> {
    type Evm = RevmEvm<
        EthEvmContext<&'db mut dyn DatabaseExt>,
        I,
        EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt>>,
        PrecompilesMap,
        EthFrame<EthInterpreter>,
    >;
    type Error = EVMError<DatabaseError>;
    type HaltReason = HaltReason;
}

impl<'db, I: InspectorExt> FoundryHandler<'db, I> {
    /// Handles CREATE2 frame initialization, potentially transforming it to use the CREATE2
    /// factory.
    fn handle_create_frame(
        &mut self,
        evm: &mut <Self as Handler>::Evm,
        init: &mut FrameInit,
    ) -> Result<Option<FrameResult>, <Self as Handler>::Error> {
        if let FrameInput::Create(inputs) = &init.frame_input
            && let CreateScheme::Create2 { salt } = inputs.scheme
        {
            let (ctx, inspector) = evm.ctx_inspector();

            if inspector.should_use_create2_factory(ctx, inputs) {
                let gas_limit = inputs.gas_limit;

                // Get CREATE2 deployer.
                let create2_deployer = evm.inspector().create2_deployer();

                // Generate call inputs for CREATE2 factory.
                let call_inputs = get_create2_factory_call_inputs(salt, inputs, create2_deployer);

                // Push data about current override to the stack.
                self.create2_overrides.push((evm.journal().depth(), call_inputs.clone()));

                // Sanity check that CREATE2 deployer exists.
                let code_hash = evm.journal_mut().load_account(create2_deployer)?.info.code_hash;
                if code_hash == KECCAK_EMPTY {
                    return Ok(Some(FrameResult::Call(CallOutcome {
                        result: InterpreterResult {
                            result: InstructionResult::Revert,
                            output: Bytes::from(
                                format!("missing CREATE2 deployer: {create2_deployer}")
                                    .into_bytes(),
                            ),
                            gas: Gas::new(gas_limit),
                        },
                        memory_offset: 0..0,
                    })));
                } else if code_hash != DEFAULT_CREATE2_DEPLOYER_CODEHASH {
                    return Ok(Some(FrameResult::Call(CallOutcome {
                        result: InterpreterResult {
                            result: InstructionResult::Revert,
                            output: "invalid CREATE2 deployer bytecode".into(),
                            gas: Gas::new(gas_limit),
                        },
                        memory_offset: 0..0,
                    })));
                }

                // Rewrite the frame init
                init.frame_input = FrameInput::Call(Box::new(call_inputs));
            }
        }
        Ok(None)
    }

    /// Transforms CREATE2 factory call results back into CREATE outcomes.
    fn handle_create2_override(
        &mut self,
        evm: &mut <Self as Handler>::Evm,
        result: FrameResult,
    ) -> FrameResult {
        if self.create2_overrides.last().is_some_and(|(depth, _)| *depth == evm.journal().depth()) {
            let (_, call_inputs) = self.create2_overrides.pop().unwrap();
            let FrameResult::Call(mut call) = result else {
                unreachable!("create2 override should be a call frame");
            };

            // Decode address from output.
            let address = match call.instruction_result() {
                return_ok!() => Address::try_from(call.output().as_ref())
                    .map_err(|_| {
                        call.result = InterpreterResult {
                            result: InstructionResult::Revert,
                            output: "invalid CREATE2 factory output".into(),
                            gas: Gas::new(call_inputs.gas_limit),
                        };
                    })
                    .ok(),
                _ => None,
            };

            FrameResult::Create(CreateOutcome { result: call.result, address })
        } else {
            result
        }
    }
}

impl<I: InspectorExt> InspectorHandler for FoundryHandler<'_, I> {
    type IT = EthInterpreter;

    fn inspect_run_exec_loop(
        &mut self,
        evm: &mut Self::Evm,
        first_frame_input: <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameInit,
    ) -> Result<FrameResult, Self::Error> {
        let res = evm.inspect_frame_init(first_frame_input)?;

        if let ItemOrResult::Result(frame_result) = res {
            return Ok(frame_result);
        }

        loop {
            let call_or_result = evm.inspect_frame_run()?;

            let result = match call_or_result {
                ItemOrResult::Item(mut init) => {
                    // Handle CREATE/CREATE2 frame initialization
                    if let Some(frame_result) = self.handle_create_frame(evm, &mut init)? {
                        return Ok(frame_result);
                    }

                    match evm.inspect_frame_init(init)? {
                        ItemOrResult::Item(_) => continue,
                        ItemOrResult::Result(result) => result,
                    }
                }
                ItemOrResult::Result(result) => result,
            };

            // Handle CREATE2 override transformation if needed
            let result = self.handle_create2_override(evm, result);

            if let Some(result) = evm.frame_return_result(result)? {
                return Ok(result);
            }
        }
    }
}
