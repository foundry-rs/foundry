use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::{
    Env, InspectorExt, backend::DatabaseExt, constants::DEFAULT_CREATE2_DEPLOYER_CODEHASH,
};
use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_evm::{Evm, EvmEnv, precompiles::PrecompilesMap};
use alloy_primitives::{Address, Bytes, U256};
use foundry_fork_db::DatabaseError;
use monad_revm::{
    MonadCfgEnv, MonadContext, MonadEvm as InnerMonadEvm, MonadSpecId,
    instructions::MonadInstructions, precompiles::MonadPrecompiles,
};
use revm::{
    Context, Journal,
    context::{
        BlockEnv, ContextTr, CreateScheme, JournalTr, LocalContext, LocalContextTr, TxEnv,
        result::{EVMError, ExecResultAndState, ExecutionResult, HaltReason, ResultAndState},
    },
    handler::{EvmTr, FrameResult, FrameTr, Handler, ItemOrResult},
    inspector::{InspectorEvmTr, InspectorHandler},
    interpreter::{
        CallInput, CallInputs, CallOutcome, CallScheme, CallValue, CreateInputs, CreateOutcome,
        FrameInput, Gas, InstructionResult, InterpreterResult, SharedMemory,
        interpreter::EthInterpreter, interpreter_action::FrameInit, return_ok,
    },
};

/// Creates a new Monad EVM with the given inspector.
///
/// This function builds a `MonadContext` and wraps it in a `FoundryEvm` with
/// Monad-specific gas costs, precompiles, and handler behavior.
pub fn new_evm_with_inspector<'db, I: InspectorExt>(
    db: &'db mut dyn DatabaseExt,
    env: Env,
    inspector: I,
) -> FoundryEvm<'db, I> {
    let spec = env.evm_env.cfg_env.spec;
    // Convert to MonadCfgEnv to apply Monad-specific defaults (128KB code size)
    let monad_cfg = MonadCfgEnv::from(env.evm_env.cfg_env);

    let mut ctx: MonadContext<&'db mut dyn DatabaseExt> = Context {
        journaled_state: {
            let mut journal = Journal::new(db);
            journal.set_spec_id(spec.into_eth_spec());
            journal
        },
        block: env.evm_env.block_env,
        cfg: monad_cfg,
        tx: env.tx,
        chain: (),
        local: LocalContext::default(),
        error: Ok(()),
    };
    ctx.cfg.tx_chain_id_check = true;

    let mut evm = FoundryEvm {
        inner: InnerMonadEvm::new(ctx, inspector).with_precompiles(get_precompiles(spec)),
    };

    evm.inspector().get_networks().inject_precompiles(evm.precompiles_mut());
    evm
}

/// Creates a new Monad EVM with an existing context.
///
/// Used for nested execution (e.g., cheatcode execution).
pub fn new_evm_with_existing_context<'a>(
    ctx: MonadContext<&'a mut dyn DatabaseExt>,
    inspector: &'a mut dyn InspectorExt,
) -> FoundryEvm<'a, &'a mut dyn InspectorExt> {
    use revm::context::Cfg;
    let spec = ctx.cfg.spec();

    let mut evm = FoundryEvm {
        inner: InnerMonadEvm::new(ctx, inspector).with_precompiles(get_precompiles(spec)),
    };

    evm.inspector().get_networks().inject_precompiles(evm.precompiles_mut());
    evm
}

/// Get the Monad precompiles for the given spec.
fn get_precompiles(spec: MonadSpecId) -> PrecompilesMap {
    PrecompilesMap::from_static(MonadPrecompiles::new_with_spec(spec).precompiles())
}

/// Get the call inputs for the CREATE2 factory.
fn get_create2_factory_call_inputs(
    salt: U256,
    inputs: &CreateInputs,
    deployer: Address,
) -> CallInputs {
    let calldata = [&salt.to_be_bytes::<32>()[..], &inputs.init_code()[..]].concat();
    CallInputs {
        caller: inputs.caller(),
        bytecode_address: deployer,
        known_bytecode: None,
        target_address: deployer,
        scheme: CallScheme::Call,
        value: CallValue::Transfer(inputs.value()),
        input: CallInput::Bytes(calldata.into()),
        gas_limit: inputs.gas_limit(),
        is_static: false,
        return_memory_offset: 0..0,
    }
}

/// Foundry EVM wrapper around Monad EVM.
///
/// This provides Foundry-specific functionality on top of the Monad EVM,
/// including CREATE2 factory support and custom execution handling.
pub struct FoundryEvm<'db, I: InspectorExt> {
    #[allow(clippy::type_complexity)]
    inner: InnerMonadEvm<
        MonadContext<&'db mut dyn DatabaseExt>,
        I,
        MonadInstructions<MonadContext<&'db mut dyn DatabaseExt>>,
        PrecompilesMap,
    >,
}

impl<'db, I: InspectorExt> FoundryEvm<'db, I> {
    /// Consumes the EVM and returns the inner context.
    pub fn into_context(self) -> MonadContext<&'db mut dyn DatabaseExt> {
        self.inner.0.ctx
    }

    /// Returns a copy of the current environment.
    pub fn env(&self) -> Env {
        Env {
            evm_env: EvmEnv {
                cfg_env: self.inner.0.ctx.cfg.clone().into_inner(),
                block_env: self.inner.0.ctx.block.clone(),
            },
            tx: self.inner.0.ctx.tx.clone(),
        }
    }

    pub fn run_execution(
        &mut self,
        frame: FrameInput,
    ) -> Result<FrameResult, EVMError<DatabaseError>> {
        let mut handler = FoundryHandler::<I>::default();

        // Create first frame
        let memory = SharedMemory::new_with_buffer(
            self.inner.0.ctx().local().shared_memory_buffer().clone(),
        );
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
    type Spec = MonadSpecId;
    type Tx = TxEnv;
    type BlockEnv = BlockEnv;

    fn block(&self) -> &BlockEnv {
        &self.inner.0.ctx.block
    }

    fn chain_id(&self) -> u64 {
        self.inner.0.ctx.cfg.chain_id
    }

    fn components(&self) -> (&Self::DB, &Self::Inspector, &Self::Precompiles) {
        (
            &self.inner.0.ctx.journaled_state.database,
            &self.inner.0.inspector,
            &self.inner.0.precompiles,
        )
    }

    fn components_mut(&mut self) -> (&mut Self::DB, &mut Self::Inspector, &mut Self::Precompiles) {
        (
            &mut self.inner.0.ctx.journaled_state.database,
            &mut self.inner.0.inspector,
            &mut self.inner.0.precompiles,
        )
    }

    fn db_mut(&mut self) -> &mut Self::DB {
        &mut self.inner.0.ctx.journaled_state.database
    }

    fn precompiles(&self) -> &Self::Precompiles {
        &self.inner.0.precompiles
    }

    fn precompiles_mut(&mut self) -> &mut Self::Precompiles {
        &mut self.inner.0.precompiles
    }

    fn inspector(&self) -> &Self::Inspector {
        &self.inner.0.inspector
    }

    fn inspector_mut(&mut self) -> &mut Self::Inspector {
        &mut self.inner.0.inspector
    }

    fn set_inspector_enabled(&mut self, _enabled: bool) {
        unimplemented!("FoundryEvm is always inspecting")
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        self.inner.0.ctx.tx = tx;

        let mut handler = FoundryHandler::<I>::default();
        let result = handler.inspect_run(&mut self.inner)?;

        Ok(ResultAndState::new(result, self.inner.0.ctx.journaled_state.inner.state.clone()))
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
        let Context { block: block_env, cfg: monad_cfg, journaled_state, .. } = self.inner.0.ctx;
        // Convert MonadCfgEnv back to CfgEnv<MonadSpecId> for EvmEnv
        let cfg_env = monad_cfg.into_inner();

        (journaled_state.database, EvmEnv { block_env, cfg_env })
    }
}

impl<'db, I: InspectorExt> Deref for FoundryEvm<'db, I> {
    type Target = MonadContext<&'db mut dyn DatabaseExt>;

    fn deref(&self) -> &Self::Target {
        &self.inner.0.ctx
    }
}

impl<I: InspectorExt> DerefMut for FoundryEvm<'_, I> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner.0.ctx
    }
}

/// Foundry handler for Monad EVM execution.
///
/// This handler provides Foundry-specific behavior (CREATE2 factory support)
/// on top of the Monad EVM's gas-limit charging model.
pub struct FoundryHandler<'db, I: InspectorExt> {
    create2_overrides: Vec<(usize, CallInputs)>,
    _phantom: PhantomData<(&'db mut dyn DatabaseExt, I)>,
}

impl<I: InspectorExt> Default for FoundryHandler<'_, I> {
    fn default() -> Self {
        Self { create2_overrides: Vec::new(), _phantom: PhantomData }
    }
}

// Handler implementation for FoundryHandler with Monad EVM.
impl<'db, I: InspectorExt> Handler for FoundryHandler<'db, I> {
    type Evm = InnerMonadEvm<
        MonadContext<&'db mut dyn DatabaseExt>,
        I,
        MonadInstructions<MonadContext<&'db mut dyn DatabaseExt>>,
        PrecompilesMap,
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
            && let CreateScheme::Create2 { salt } = inputs.scheme()
        {
            // Access the inner Evm's context and inspector
            let (ctx, inspector) = (&mut evm.0.ctx, &mut evm.0.inspector);

            if inspector.should_use_create2_factory(ctx, inputs) {
                let gas_limit = inputs.gas_limit();

                // Get CREATE2 deployer.
                let create2_deployer = evm.0.inspector.create2_deployer();

                // Generate call inputs for CREATE2 factory.
                let call_inputs = get_create2_factory_call_inputs(salt, inputs, create2_deployer);

                // Push data about current override to the stack.
                // Access journal depth through the context's journaled_state
                self.create2_overrides
                    .push((evm.0.ctx.journaled_state.depth(), call_inputs.clone()));

                // Sanity check that CREATE2 deployer exists.
                let code_hash =
                    evm.0.ctx.journaled_state.load_account(create2_deployer)?.info.code_hash;
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
                        was_precompile_called: false,
                        precompile_call_logs: vec![],
                    })));
                } else if code_hash != DEFAULT_CREATE2_DEPLOYER_CODEHASH {
                    return Ok(Some(FrameResult::Call(CallOutcome {
                        result: InterpreterResult {
                            result: InstructionResult::Revert,
                            output: "invalid CREATE2 deployer bytecode".into(),
                            gas: Gas::new(gas_limit),
                        },
                        memory_offset: 0..0,
                        was_precompile_called: false,
                        precompile_call_logs: vec![],
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
        // Access journal depth through the context's journaled_state
        if self
            .create2_overrides
            .last()
            .is_some_and(|(depth, _)| *depth == evm.0.ctx.journaled_state.depth())
        {
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
