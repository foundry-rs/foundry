use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::{
    FoundryContextExt, FoundryInspectorExt,
    backend::{DatabaseExt, JournaledState},
    constants::DEFAULT_CREATE2_DEPLOYER_CODEHASH,
};
use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_evm::{Evm, EvmEnv, EvmFactory, eth::EthEvmContext, precompiles::PrecompilesMap};
use alloy_primitives::{Address, Bytes, U256};
use foundry_fork_db::DatabaseError;
use revm::{
    Context,
    context::{
        BlockEnv, CfgEnv, ContextTr, CreateScheme, Evm as RevmEvm, JournalTr, LocalContextTr,
        TxEnv,
        result::{EVMError, ExecResultAndState, ExecutionResult, HaltReason, ResultAndState},
    },
    handler::{
        EthFrame, EvmTr, FrameResult, FrameTr, Handler, ItemOrResult, instructions::EthInstructions,
    },
    inspector::{InspectorEvmTr, InspectorHandler},
    interpreter::{
        CallInput, CallInputs, CallOutcome, CallScheme, CallValue, CreateInputs, CreateOutcome,
        FrameInput, Gas, InstructionResult, InterpreterResult, SharedMemory,
        interpreter::EthInterpreter, interpreter_action::FrameInit, return_ok,
    },
    primitives::hardfork::SpecId,
};

pub fn new_revm_with_inspector<
    'db,
    I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt>>,
>(
    db: &'db mut dyn DatabaseExt,
    evm_env: EvmEnv,
    inspector: I,
) -> EthRevmEvm<'db, I> {
    let mut revm = alloy_evm::EthEvmFactory::default()
        .create_evm_with_inspector(db, evm_env, inspector)
        .into_inner();
    revm.ctx.cfg.tx_chain_id_check = true;
    revm.inspector.get_networks().inject_precompiles(&mut revm.precompiles);
    revm
}

pub fn new_eth_evm_with_inspector<
    'db,
    I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt>>,
>(
    db: &'db mut dyn DatabaseExt,
    evm_env: EvmEnv,
    inspector: I,
) -> FoundryEvm<'db, I> {
    let eth_evm =
        alloy_evm::EthEvmFactory::default().create_evm_with_inspector(db, evm_env, inspector);
    let mut inner = eth_evm.into_inner();
    inner.ctx.cfg.tx_chain_id_check = true;

    let mut evm = FoundryEvm { inner };
    evm.inspector().get_networks().inject_precompiles(evm.precompiles_mut());
    evm
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

type EthRevmEvm<'db, I> = RevmEvm<
    EthEvmContext<&'db mut dyn DatabaseExt>,
    I,
    EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt>>,
    PrecompilesMap,
    EthFrame<EthInterpreter>,
>;

pub struct FoundryEvm<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt>>> {
    inner: EthRevmEvm<'db, I>,
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt>>> Evm
    for FoundryEvm<'db, I>
{
    type Precompiles = PrecompilesMap;
    type Inspector = I;
    type DB = &'db mut dyn DatabaseExt;
    type Error = EVMError<DatabaseError>;
    type HaltReason = HaltReason;
    type Spec = SpecId;
    type Tx = TxEnv;
    type BlockEnv = BlockEnv;

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

    fn set_inspector_enabled(&mut self, _enabled: bool) {
        unimplemented!("FoundryEvm is always inspecting")
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        self.inner.set_tx(tx);

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

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt>>> Deref
    for FoundryEvm<'db, I>
{
    type Target = Context<BlockEnv, TxEnv, CfgEnv, &'db mut dyn DatabaseExt>;

    fn deref(&self) -> &Self::Target {
        &self.inner.ctx
    }
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt>>> DerefMut
    for FoundryEvm<'db, I>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner.ctx
    }
}

/// Object-safe trait exposing the operations that cheatcode nested EVM closures need.
///
/// This abstracts over the concrete EVM type (`FoundryEvm`, future `TempoEvm`, etc.)
/// so that cheatcode impls can build and run nested EVMs without knowing the concrete type.
pub trait NestedEvm {
    /// The transaction environment type.
    type Tx;

    /// Returns a mutable reference to the journal inner state (`JournaledState`).
    fn journal_inner_mut(&mut self) -> &mut JournaledState;

    /// Runs a single execution frame (create or call) through the EVM handler loop.
    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>>;

    /// Executes a full transaction with the given tx env.
    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<HaltReason>, EVMError<DatabaseError>>;
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt>>> NestedEvm
    for EthRevmEvm<'db, I>
{
    type Tx = TxEnv;

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx_mut().journaled_state.inner
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        let mut handler = FoundryHandler::<I>::default();

        // Create first frame
        let memory =
            SharedMemory::new_with_buffer(self.ctx().local().shared_memory_buffer().clone());
        let first_frame_input = FrameInit { depth: 0, memory, frame_input: frame };

        // Run execution loop
        let mut frame_result = handler.inspect_run_exec_loop(self, first_frame_input)?;

        // Handle last frame result
        handler.last_frame_result(self, &mut frame_result)?;

        Ok(frame_result)
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<HaltReason>, EVMError<DatabaseError>> {
        self.set_tx(tx);

        let mut handler = FoundryHandler::<I>::default();
        let result = handler.inspect_run(self)?;

        Ok(ResultAndState::new(result, self.ctx.journaled_state.inner.state.clone()))
    }
}

/// Closure type used by `CheatcodesExecutor` methods that run nested EVM operations.
pub type NestedEvmClosure<'a, Tx> =
    &'a mut dyn FnMut(&mut dyn NestedEvm<Tx = Tx>) -> Result<(), EVMError<DatabaseError>>;

/// Clones the current context (env + journal), passes the database, cloned env,
/// and cloned journal inner to the callback. The callback builds whatever EVM it
/// needs, runs its operations, and returns `(result, modified_env, modified_journal)`.
/// Modified state is written back after the callback returns.
pub fn with_cloned_context<CTX: FoundryContextExt>(
    ecx: &mut CTX,
    f: impl FnOnce(
        &mut CTX::Db,
        EvmEnv<CTX::Spec, CTX::Block>,
        JournaledState,
    )
        -> Result<(EvmEnv<CTX::Spec, CTX::Block>, JournaledState), EVMError<DatabaseError>>,
) -> Result<(), EVMError<DatabaseError>> {
    let evm_env = ecx.evm_clone();

    let (db, journal_inner) = ecx.db_journal_inner_mut();
    let journal_inner_clone = journal_inner.clone();

    let (sub_evm_env, sub_inner) = f(db, evm_env, journal_inner_clone)?;

    // Write back modified state. The db borrow was released when f returned.
    ecx.set_journal_inner(sub_inner);
    ecx.set_evm(sub_evm_env);

    Ok(())
}

pub struct FoundryHandler<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt>>> {
    create2_overrides: Vec<(usize, CallInputs)>,
    _phantom: PhantomData<(&'db mut dyn DatabaseExt, I)>,
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt>>> Default
    for FoundryHandler<'db, I>
{
    fn default() -> Self {
        Self { create2_overrides: Vec::new(), _phantom: PhantomData }
    }
}

// Blanket Handler implementation for FoundryHandler, needed for implementing the InspectorHandler
// trait.
impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt>>> Handler
    for FoundryHandler<'db, I>
{
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

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt>>> FoundryHandler<'db, I> {
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
            let (ctx, inspector) = evm.ctx_inspector();

            if inspector.should_use_create2_factory(ctx.journal().depth(), inputs) {
                let gas_limit = inputs.gas_limit();

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

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt>>> InspectorHandler
    for FoundryHandler<'db, I>
{
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
