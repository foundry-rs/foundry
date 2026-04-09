use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::{
    FoundryBlock, FoundryContextExt, FoundryInspectorExt, FoundryTransaction,
    backend::{DatabaseExt, JournaledState},
    constants::{CALLER, DEFAULT_CREATE2_DEPLOYER_CODEHASH, TEST_CONTRACT_ADDRESS},
    tempo::{TEMPO_PRECOMPILE_ADDRESSES, TEMPO_TIP20_TOKENS, initialize_tempo_genesis_inner},
};
use alloy_consensus::{
    SignableTransaction, Signed, constants::KECCAK_EMPTY, transaction::SignerRecoverable,
};
use alloy_evm::{
    EthEvmFactory, Evm, EvmEnv, EvmFactory, FromRecoveredTx, eth::EthEvmContext,
    precompiles::PrecompilesMap,
};
use alloy_network::{Ethereum, Network};
use alloy_primitives::{Address, Bytes, Signature, U256};
use alloy_rlp::Decodable;
use foundry_common::{FoundryReceiptResponse, FoundryTransactionBuilder, fmt::UIfmt};
use foundry_config::FromEvmVersion;
use foundry_fork_db::{DatabaseError, ForkBlockEnv};
use op_alloy_network::Optimism;
use op_revm::OpHaltReason;
use revm::{
    Context,
    context::{
        BlockEnv, ContextTr, CreateScheme, Evm as RevmEvm, JournalTr, LocalContextTr, TxEnv,
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
    state::Bytecode,
};
use serde::{Deserialize, Serialize};
use tempo_alloy::TempoNetwork;
use tempo_chainspec::hardfork::TempoHardfork;
use tempo_evm::evm::TempoEvmFactory;
use tempo_precompiles::storage::StorageCtx;
use tempo_revm::{
    TempoBlockEnv, TempoHaltReason, TempoInvalidTransaction, TempoTxEnv, evm::TempoContext,
    gas_params::tempo_gas_params, handler::TempoEvmHandler,
};

/// Converts a network-specific halt reason into an [`InstructionResult`].
pub trait IntoInstructionResult {
    fn into_instruction_result(self) -> InstructionResult;
}

impl IntoInstructionResult for HaltReason {
    fn into_instruction_result(self) -> InstructionResult {
        self.into()
    }
}

impl IntoInstructionResult for OpHaltReason {
    fn into_instruction_result(self) -> InstructionResult {
        match self {
            Self::Base(eth) => eth.into(),
            Self::FailedDeposit => InstructionResult::Stop,
        }
    }
}

impl IntoInstructionResult for TempoHaltReason {
    fn into_instruction_result(self) -> InstructionResult {
        match self {
            Self::Ethereum(eth) => eth.into(),
            _ => InstructionResult::PrecompileError,
        }
    }
}

/// Foundry's supertrait associating [Network] with [FoundryEvmFactory]
pub trait FoundryEvmNetwork: Copy + Debug + Default + 'static {
    type Network: Network<
            TxEnvelope: Decodable
                            + SignerRecoverable
                            + From<Signed<<Self::Network as Network>::UnsignedTx>>
                            + for<'d> Deserialize<'d>
                            + Serialize
                            + UIfmt,
            UnsignedTx: SignableTransaction<Signature>,
            TransactionRequest: FoundryTransactionBuilder<Self::Network>
                                    + for<'d> Deserialize<'d>
                                    + Serialize,
            ReceiptResponse: FoundryReceiptResponse,
        >;
    type EvmFactory: FoundryEvmFactory<Tx: FromRecoveredTx<<Self::Network as Network>::TxEnvelope>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct EthEvmNetwork;
impl FoundryEvmNetwork for EthEvmNetwork {
    type Network = Ethereum;
    type EvmFactory = EthEvmFactory;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TempoEvmNetwork;
impl FoundryEvmNetwork for TempoEvmNetwork {
    type Network = TempoNetwork;
    type EvmFactory = TempoEvmFactory;
}

// TODO: use `OpEvmFactory` once the FoundryEvmFactory impl is available for it.
#[derive(Clone, Copy, Debug, Default)]
pub struct OpEvmNetwork;
impl FoundryEvmNetwork for OpEvmNetwork {
    type Network = Optimism;
    type EvmFactory = EthEvmFactory;
}

/// Convenience type aliases for accessing associated types through [`FoundryEvmNetwork`].
pub type EvmFactoryFor<FEN> = <FEN as FoundryEvmNetwork>::EvmFactory;
pub type FoundryContextFor<'db, FEN> =
    <EvmFactoryFor<FEN> as FoundryEvmFactory>::FoundryContext<'db>;
pub type TxEnvFor<FEN> = <EvmFactoryFor<FEN> as EvmFactory>::Tx;
pub type HaltReasonFor<FEN> = <EvmFactoryFor<FEN> as EvmFactory>::HaltReason;
pub type SpecFor<FEN> = <EvmFactoryFor<FEN> as EvmFactory>::Spec;
pub type BlockEnvFor<FEN> = <EvmFactoryFor<FEN> as EvmFactory>::BlockEnv;
pub type PrecompilesFor<FEN> = <EvmFactoryFor<FEN> as EvmFactory>::Precompiles;
pub type EvmEnvFor<FEN> = EvmEnv<SpecFor<FEN>, BlockEnvFor<FEN>>;

pub type NetworkFor<FEN> = <FEN as FoundryEvmNetwork>::Network;
pub type TxEnvelopeFor<FEN> = <NetworkFor<FEN> as Network>::TxEnvelope;
pub type TransactionRequestFor<FEN> = <NetworkFor<FEN> as Network>::TransactionRequest;
pub type TransactionResponseFor<FEN> = <NetworkFor<FEN> as Network>::TransactionResponse;
pub type BlockResponseFor<FEN> = <NetworkFor<FEN> as Network>::BlockResponse;

pub trait FoundryEvmFactory:
    EvmFactory<
        Spec: Into<SpecId> + FromEvmVersion + Default + Display + Copy + Unpin + Send + 'static,
        BlockEnv: FoundryBlock + ForkBlockEnv + Default + Unpin,
        Tx: Clone + Debug + FoundryTransaction + Default + Send + Sync,
        HaltReason: IntoInstructionResult,
        Precompiles = PrecompilesMap,
    > + Clone
    + Debug
    + Default
    + 'static
{
    /// Foundry Context abstraction
    type FoundryContext<'db>: FoundryContextExt<
            Block = Self::BlockEnv,
            Tx = Self::Tx,
            Spec = Self::Spec,
            Db: DatabaseExt<Self>,
        >
    where
        Self: 'db;

    /// The Foundry-wrapped EVM type produced by this factory.
    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>: Evm<
            DB = &'db mut dyn DatabaseExt<Self>,
            Tx = Self::Tx,
            BlockEnv = Self::BlockEnv,
            Spec = Self::Spec,
            HaltReason = Self::HaltReason,
        > + Deref<Target = Self::FoundryContext<'db>>
        + IntoNestedEvm<Self::Spec, Self::BlockEnv, Self::Tx>
    where
        Self: 'db;

    /// Creates a Foundry-wrapped EVM with the given inspector.
    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I>;

    /// Creates a Foundry-wrapped EVM with a dynamic inspector, returning a boxed [`NestedEvm`].
    ///
    /// This helper exists because `&mut dyn FoundryInspectorExt<FoundryContext>` cannot satisfy
    /// the generic `I: FoundryInspectorExt<Self::FoundryContext<'db>>` bound when the context
    /// type is only known through an associated type.  Each concrete factory implements this
    /// directly, side-stepping the higher-kinded lifetime issue.
    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> Box<dyn NestedEvm<Spec = Self::Spec, Block = Self::BlockEnv, Tx = Self::Tx> + 'db>;
}

impl FoundryEvmFactory for EthEvmFactory {
    type FoundryContext<'db> = EthEvmContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> = EthFoundryEvm<'db, I>;

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let eth_evm = Self::default().create_evm_with_inspector(db, evm_env, inspector);
        let mut inner = eth_evm.into_inner();
        inner.ctx.cfg.tx_chain_id_check = true;

        let mut evm = EthFoundryEvm { inner };
        evm.inspector().get_networks().inject_precompiles(evm.precompiles_mut());
        evm
    }

    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> Box<dyn NestedEvm<Spec = SpecId, Block = BlockEnv, Tx = TxEnv> + 'db> {
        Box::new(self.create_foundry_evm_with_inspector(db, evm_env, inspector).into_nested_evm())
    }
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
    EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>,
    I,
    EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>,
    PrecompilesMap,
    EthFrame<EthInterpreter>,
>;

pub struct EthFoundryEvm<
    'db,
    I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>,
> {
    pub inner: EthRevmEvm<'db, I>,
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>> Evm
    for EthFoundryEvm<'db, I>
{
    type Precompiles = PrecompilesMap;
    type Inspector = I;
    type DB = &'db mut dyn DatabaseExt<EthEvmFactory>;
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

        let mut handler = EthFoundryHandler::<I>::default();
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

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>> Deref
    for EthFoundryEvm<'db, I>
{
    type Target = EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>;

    fn deref(&self) -> &Self::Target {
        &self.inner.ctx
    }
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>> DerefMut
    for EthFoundryEvm<'db, I>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner.ctx
    }
}

/// Trait for converting a Foundry EVM wrapper into its inner `NestedEvm` implementation.
///
/// Both [`EthFoundryEvm`] and [`TempoFoundryEvm`] wrap an inner revm EVM that implements
/// [`NestedEvm`]. This trait provides a uniform way to unwrap them.
pub trait IntoNestedEvm<SPEC, BLOCK, TX> {
    /// The inner type that implements [`NestedEvm`].
    type Inner: NestedEvm<Spec = SPEC, Block = BLOCK, Tx = TX>;

    /// Consumes the wrapper, returning the inner revm EVM.
    fn into_nested_evm(self) -> Self::Inner;
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>>
    IntoNestedEvm<SpecId, BlockEnv, TxEnv> for EthFoundryEvm<'db, I>
{
    type Inner = EthRevmEvm<'db, I>;

    fn into_nested_evm(self) -> Self::Inner {
        self.inner
    }
}

/// Object-safe trait exposing the operations that cheatcode nested EVM closures need.
///
/// This abstracts over the concrete EVM type (`FoundryEvm`, future `TempoEvm`, etc.)
/// so that cheatcode impls can build and run nested EVMs without knowing the concrete type.
pub trait NestedEvm {
    /// The spec type.
    type Spec;
    /// The block environment type.
    type Block;
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

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block>;
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>> NestedEvm
    for EthRevmEvm<'db, I>
{
    type Spec = SpecId;
    type Block = BlockEnv;
    type Tx = TxEnv;

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx_mut().journaled_state.inner
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        let mut handler = EthFoundryHandler::<I>::default();

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

        let mut handler = EthFoundryHandler::<I>::default();
        let result = handler.inspect_run(self)?;

        Ok(ResultAndState::new(result, self.ctx.journaled_state.inner.state.clone()))
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block> {
        self.ctx_ref().evm_clone()
    }
}

/// Closure type used by `CheatcodesExecutor` methods that run nested EVM operations.
pub type NestedEvmClosure<'a, Spec, Block, Tx> =
    &'a mut dyn FnMut(
        &mut dyn NestedEvm<Spec = Spec, Block = Block, Tx = Tx>,
    ) -> Result<(), EVMError<DatabaseError>>;

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

pub struct EthFoundryHandler<
    'db,
    I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>,
> {
    create2_overrides: Vec<(usize, CallInputs)>,
    _phantom: PhantomData<(&'db mut dyn DatabaseExt<EthEvmFactory>, I)>,
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>> Default
    for EthFoundryHandler<'db, I>
{
    fn default() -> Self {
        Self { create2_overrides: Vec::new(), _phantom: PhantomData }
    }
}

// Blanket Handler implementation for FoundryHandler, needed for implementing the InspectorHandler
// trait.
impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>> Handler
    for EthFoundryHandler<'db, I>
{
    type Evm = RevmEvm<
        EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>,
        I,
        EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>,
        PrecompilesMap,
        EthFrame<EthInterpreter>,
    >;
    type Error = EVMError<DatabaseError>;
    type HaltReason = HaltReason;
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>>
    EthFoundryHandler<'db, I>
{
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

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>>
    InspectorHandler for EthFoundryHandler<'db, I>
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

// Will be removed when the next revm release includes bluealloy/revm#3518.
type TempoRevmEvm<'db, I> = tempo_revm::TempoEvm<&'db mut dyn DatabaseExt<TempoEvmFactory>, I>;

/// Tempo counterpart of [`EthFoundryEvm`]. Wraps `tempo_revm::TempoEvm` and routes execution
/// through [`TempoFoundryHandler`] which composes [`TempoEvmHandler`] with CREATE2 factory
/// redirect logic.
///
/// Uses [`TempoEvmFactory`] for construction to reuse factory setup logic, then unwraps to the
/// raw revm EVM via `into_inner()` since the handler operates at the revm level.
pub struct TempoFoundryEvm<
    'db,
    I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>,
> {
    pub inner: TempoRevmEvm<'db, I>,
}

/// Initialize Tempo precompiles and contracts for a newly created [`TempoFoundryEvm`].
///
/// In non-fork mode, runs full genesis initialization (precompile sentinel bytecode,
/// TIP20 fee tokens, standard contracts) via [`StorageCtx::enter_evm`].
///
/// In fork mode, warms up precompile and TIP20 token addresses with sentinel bytecode
/// to prevent repeated RPC round-trips for addresses that are Rust-native precompiles
/// on Tempo nodes (no real EVM bytecode on-chain).
fn initialize_tempo_evm<
    'db,
    I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>,
>(
    evm: &mut TempoFoundryEvm<'db, I>,
    is_forked: bool,
) {
    let ctx = &mut evm.inner.inner.ctx;
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
        TempoFoundryEvm<'db, I>;

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let is_forked = db.is_forked_mode();
        let spec = *evm_env.spec_id();
        let tempo_evm = Self::default().create_evm_with_inspector(db, evm_env, inspector);
        let mut inner = tempo_evm.into_inner();
        inner.ctx.cfg.gas_params = tempo_gas_params(spec);
        inner.ctx.cfg.tx_chain_id_check = true;

        let mut evm = TempoFoundryEvm { inner };
        let networks = Evm::inspector(&evm).get_networks();
        networks.inject_precompiles(evm.precompiles_mut());

        initialize_tempo_evm(&mut evm, is_forked);
        evm
    }

    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> Box<dyn NestedEvm<Spec = TempoHardfork, Block = TempoBlockEnv, Tx = TempoTxEnv> + 'db>
    {
        Box::new(self.create_foundry_evm_with_inspector(db, evm_env, inspector).into_nested_evm())
    }
}

impl<'db, I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>> Evm
    for TempoFoundryEvm<'db, I>
{
    type Precompiles = PrecompilesMap;
    type Inspector = I;
    type DB = &'db mut dyn DatabaseExt<TempoEvmFactory>;
    type Error = EVMError<DatabaseError, TempoInvalidTransaction>;
    type HaltReason = TempoHaltReason;
    type Spec = TempoHardfork;
    type Tx = TempoTxEnv;
    type BlockEnv = TempoBlockEnv;

    fn block(&self) -> &TempoBlockEnv {
        &self.inner.block
    }

    fn chain_id(&self) -> u64 {
        self.inner.ctx.cfg.chain_id
    }

    fn components(&self) -> (&Self::DB, &Self::Inspector, &Self::Precompiles) {
        let evm = &self.inner.inner;
        (&evm.ctx.journaled_state.database, &evm.inspector, &evm.precompiles)
    }

    fn components_mut(&mut self) -> (&mut Self::DB, &mut Self::Inspector, &mut Self::Precompiles) {
        let evm = &mut self.inner.inner;
        (&mut evm.ctx.journaled_state.database, &mut evm.inspector, &mut evm.precompiles)
    }

    fn set_inspector_enabled(&mut self, _enabled: bool) {
        unimplemented!("TempoFoundryEvm is always inspecting")
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        self.inner.set_tx(tx);

        let mut handler = TempoFoundryHandler::<I>::default();
        let result = handler.inspect_run(&mut self.inner)?;

        Ok(ResultAndState::new(result, self.inner.inner.ctx.journaled_state.inner.state.clone()))
    }

    fn transact_system_call(
        &mut self,
        _caller: Address,
        _contract: Address,
        _data: Bytes,
    ) -> Result<ExecResultAndState<ExecutionResult<Self::HaltReason>>, Self::Error> {
        unimplemented!()
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec, Self::BlockEnv>)
    where
        Self: Sized,
    {
        let revm_evm = self.inner.inner;
        let Context { block: block_env, cfg: cfg_env, journaled_state, .. } = revm_evm.ctx;
        (journaled_state.database, EvmEnv { block_env, cfg_env })
    }
}

impl<'db, I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>> Deref
    for TempoFoundryEvm<'db, I>
{
    type Target = TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>;

    fn deref(&self) -> &Self::Target {
        &self.inner.ctx
    }
}

impl<'db, I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>> DerefMut
    for TempoFoundryEvm<'db, I>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner.ctx
    }
}

impl<'db, I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>>
    IntoNestedEvm<TempoHardfork, TempoBlockEnv, TempoTxEnv> for TempoFoundryEvm<'db, I>
{
    type Inner = TempoRevmEvm<'db, I>;

    fn into_nested_evm(self) -> Self::Inner {
        self.inner
    }
}

/// Maps a Tempo [`EVMError`] to the common `EVMError<DatabaseError>` used by [`NestedEvm`].
///
/// This exists because [`NestedEvm`] currently uses Eth-typed errors. When `NestedEvm` gains
/// an associated `Error` type, this mapping can be removed.
fn map_tempo_error(e: EVMError<DatabaseError, TempoInvalidTransaction>) -> EVMError<DatabaseError> {
    match e {
        EVMError::Database(db) => EVMError::Database(db),
        EVMError::Header(h) => EVMError::Header(h),
        EVMError::Custom(s) => EVMError::Custom(s),
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
        let mut handler = TempoFoundryHandler::<I>::default();

        let memory =
            SharedMemory::new_with_buffer(self.ctx().local().shared_memory_buffer().clone());
        let first_frame_input = FrameInit { depth: 0, memory, frame_input: frame };

        let mut frame_result =
            handler.inspect_run_exec_loop(self, first_frame_input).map_err(map_tempo_error)?;

        handler.last_frame_result(self, &mut frame_result).map_err(map_tempo_error)?;

        Ok(frame_result)
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<HaltReason>, EVMError<DatabaseError>> {
        self.set_tx(tx);

        let mut handler = TempoFoundryHandler::<I>::default();
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

/// Tempo counterpart of [`EthFoundryHandler`]. Wraps [`TempoEvmHandler`] and injects CREATE2
/// factory redirect logic into the execution loop. Delegates all [`Handler`] methods to
/// [`TempoEvmHandler`] for proper Tempo validation, fee collection, AA dispatch, and gas
/// handling.
///
/// Will be removed when the next revm release includes bluealloy/revm#3518.
pub struct TempoFoundryHandler<
    'db,
    I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>,
> {
    inner: TempoEvmHandler<&'db mut dyn DatabaseExt<TempoEvmFactory>, I>,
    create2_overrides: Vec<(usize, CallInputs)>,
}

impl<'db, I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>> Default
    for TempoFoundryHandler<'db, I>
{
    fn default() -> Self {
        Self { inner: TempoEvmHandler::new(), create2_overrides: Vec::new() }
    }
}

impl<'db, I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>> Handler
    for TempoFoundryHandler<'db, I>
{
    type Evm = TempoRevmEvm<'db, I>;
    type Error = EVMError<DatabaseError, TempoInvalidTransaction>;
    type HaltReason = TempoHaltReason;

    #[inline]
    fn run(
        &mut self,
        evm: &mut Self::Evm,
    ) -> Result<ExecutionResult<Self::HaltReason>, Self::Error> {
        self.inner.run(evm)
    }

    #[inline]
    fn execution(
        &mut self,
        evm: &mut Self::Evm,
        init_and_floor_gas: &revm::interpreter::InitialAndFloorGas,
    ) -> Result<FrameResult, Self::Error> {
        self.inner.execution(evm, init_and_floor_gas)
    }

    #[inline]
    fn validate_env(&self, evm: &mut Self::Evm) -> Result<(), Self::Error> {
        self.inner.validate_env(evm)
    }

    #[inline]
    fn validate_against_state_and_deduct_caller(
        &self,
        evm: &mut Self::Evm,
    ) -> Result<(), Self::Error> {
        self.inner.validate_against_state_and_deduct_caller(evm)
    }

    #[inline]
    fn reimburse_caller(
        &self,
        evm: &mut Self::Evm,
        exec_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<(), Self::Error> {
        self.inner.reimburse_caller(evm, exec_result)
    }

    #[inline]
    fn reward_beneficiary(
        &self,
        evm: &mut Self::Evm,
        exec_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<(), Self::Error> {
        self.inner.reward_beneficiary(evm, exec_result)
    }

    #[inline]
    fn validate_initial_tx_gas(
        &self,
        evm: &mut Self::Evm,
    ) -> Result<revm::interpreter::InitialAndFloorGas, Self::Error> {
        self.inner.validate_initial_tx_gas(evm)
    }

    #[inline]
    fn execution_result(
        &mut self,
        evm: &mut Self::Evm,
        result: <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
        result_gas: revm::context::result::ResultGas,
    ) -> Result<ExecutionResult<Self::HaltReason>, Self::Error> {
        self.inner.execution_result(evm, result, result_gas)
    }

    #[inline]
    fn catch_error(
        &self,
        evm: &mut Self::Evm,
        error: Self::Error,
    ) -> Result<ExecutionResult<Self::HaltReason>, Self::Error> {
        self.inner.catch_error(evm, error)
    }
}

/// CREATE2 factory redirect execution loop for Tempo.
fn create2_exec_loop<
    'db,
    I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>,
>(
    create2_overrides: &mut Vec<(usize, CallInputs)>,
    evm: &mut TempoRevmEvm<'db, I>,
    first_frame_input: FrameInit,
) -> Result<FrameResult, EVMError<DatabaseError, TempoInvalidTransaction>> {
    let res = evm.inspect_frame_init(first_frame_input)?;

    if let ItemOrResult::Result(frame_result) = res {
        return Ok(frame_result);
    }

    loop {
        let call_or_result = evm.inspect_frame_run()?;

        let result = match call_or_result {
            ItemOrResult::Item(mut init) => {
                if let Some(frame_result) = handle_create2_frame(create2_overrides, evm, &mut init)?
                {
                    return Ok(frame_result);
                }

                match evm.inspect_frame_init(init)? {
                    ItemOrResult::Item(_) => continue,
                    ItemOrResult::Result(result) => result,
                }
            }
            ItemOrResult::Result(result) => result,
        };

        let result = handle_create2_result(create2_overrides, evm, result);

        if let Some(result) = evm.frame_return_result(result)? {
            return Ok(result);
        }
    }
}

/// Handles CREATE2 frame initialization, potentially transforming it to use the CREATE2 factory.
fn handle_create2_frame<
    'db,
    I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>,
>(
    create2_overrides: &mut Vec<(usize, CallInputs)>,
    evm: &mut TempoRevmEvm<'db, I>,
    init: &mut FrameInit,
) -> Result<Option<FrameResult>, EVMError<DatabaseError, TempoInvalidTransaction>> {
    if let FrameInput::Create(inputs) = &init.frame_input
        && let CreateScheme::Create2 { salt } = inputs.scheme()
    {
        let (ctx, inspector) = evm.ctx_inspector();

        if inspector.should_use_create2_factory(ctx.journal().depth(), inputs) {
            let gas_limit = inputs.gas_limit();
            let create2_deployer = evm.inspector().create2_deployer();
            let call_inputs = get_create2_factory_call_inputs(salt, inputs, create2_deployer);

            create2_overrides.push((evm.journal().depth(), call_inputs.clone()));

            let code_hash = evm.journal_mut().load_account(create2_deployer)?.info.code_hash;
            if code_hash == KECCAK_EMPTY {
                return Ok(Some(FrameResult::Call(CallOutcome {
                    result: InterpreterResult {
                        result: InstructionResult::Revert,
                        output: Bytes::from(
                            format!("missing CREATE2 deployer: {create2_deployer}").into_bytes(),
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

            init.frame_input = FrameInput::Call(Box::new(call_inputs));
        }
    }
    Ok(None)
}

/// Transforms CREATE2 factory call results back into CREATE outcomes.
fn handle_create2_result<
    'db,
    I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>,
>(
    create2_overrides: &mut Vec<(usize, CallInputs)>,
    evm: &mut TempoRevmEvm<'db, I>,
    result: FrameResult,
) -> FrameResult {
    if create2_overrides.last().is_some_and(|(depth, _)| *depth == evm.journal().depth()) {
        let (_, call_inputs) = create2_overrides.pop().unwrap();
        let FrameResult::Call(mut call) = result else {
            unreachable!("create2 override should be a call frame");
        };

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

impl<'db, I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>>
    InspectorHandler for TempoFoundryHandler<'db, I>
{
    type IT = EthInterpreter;

    /// Delegates to [`TempoEvmHandler::inspect_execution_with`], injecting the CREATE2 factory
    /// redirect exec loop. AA multi-call dispatch and gas adjustments are handled by the inner
    /// Tempo handler.
    #[inline]
    fn inspect_execution(
        &mut self,
        evm: &mut Self::Evm,
        init_and_floor_gas: &revm::interpreter::InitialAndFloorGas,
    ) -> Result<FrameResult, Self::Error> {
        let overrides = &mut self.create2_overrides;
        self.inner.inspect_execution_with(evm, init_and_floor_gas, |_handler, evm, init| {
            create2_exec_loop(overrides, evm, init)
        })
    }

    fn inspect_run_exec_loop(
        &mut self,
        evm: &mut Self::Evm,
        first_frame_input: <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameInit,
    ) -> Result<FrameResult, Self::Error> {
        create2_exec_loop(&mut self.create2_overrides, evm, first_frame_input)
    }
}
