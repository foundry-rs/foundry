//! Shared EVM traits, associated types, and execution helpers.
//!
//! Each network module owns its network marker and concrete EVM implementations.

use std::{fmt::Debug, ops::DerefMut};

use crate::{
    FoundryBlock, FoundryChain, FoundryContextExt, FoundryInspectorExt, FoundryJournal,
    FoundryTransaction, FromAnyRpcTransaction,
    backend::{DatabaseExt, JournaledState},
};
use alloy_consensus::{SignableTransaction, Signed, transaction::SignerRecoverable};
use alloy_evm::{Evm, EvmEnv, EvmFactory, FromRecoveredTx, precompiles::PrecompilesMap};
use alloy_network::Network;
use alloy_primitives::{Address, Signature, U256};
use alloy_rlp::Decodable;
use foundry_common::{FoundryReceiptResponse, FoundryTransactionBuilder, fmt::UIfmt};
use foundry_config::ExecutionSpec;
use foundry_fork_db::{DatabaseError, ForkBlockEnv};
use revm::{
    Database,
    context::{
        ContextTr, JournalTr, LocalContextTr,
        result::{EVMError, HaltReason, ResultAndState},
    },
    handler::{EvmTr, FrameResult},
    inspector::{InspectorEvmTr, InspectorHandler, NoOpInspector},
    interpreter::{
        CallInput, CallInputs, CallScheme, CallValue, CreateInputs, FrameInput, GasTracker,
        InstructionResult, SharedMemory, interpreter::EthInterpreter,
        interpreter_action::FrameInit,
    },
    primitives::hardfork::SpecId,
};
use serde::{Deserialize, Serialize};

#[cfg(feature = "base")]
pub mod base;
pub mod eth;
#[cfg(feature = "monad")]
pub mod monad;
#[cfg(feature = "optimism")]
pub mod op;
pub mod tempo;

pub use eth::*;
pub use tempo::*;

#[cfg(feature = "base")]
pub use base::*;

#[cfg(feature = "monad")]
pub use monad::*;

#[cfg(feature = "optimism")]
pub use op::*;

/// Foundry's compatibility trait associating a [`Network`] with a [`FoundryEvmFactory`].
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

pub trait FoundryEvmFactory:
    EvmFactory<
        Spec: Into<SpecId> + ExecutionSpec + Default + Copy + Unpin + Send + 'static,
        BlockEnv: FoundryBlock + ForkBlockEnv + Default + Unpin,
        Tx: Clone + Debug + FoundryTransaction + FromAnyRpcTransaction + Default + Send + Sync,
        HaltReason: IntoInstructionResult,
        Precompiles = PrecompilesMap,
    > + Clone
    + Debug
    + Default
    + 'static
{
    /// Chain type for EVM's context created by this factory.
    type Chain: FoundryChain<Self::Tx>;

    /// Foundry Context abstraction
    type FoundryContext<'db>: FoundryContextExt<
            Block = Self::BlockEnv,
            Tx = Self::Tx,
            Spec = Self::Spec,
            Chain = Self::Chain,
            Journal: FoundryJournal,
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
        > + DerefMut<Target = Self::FoundryContext<'db>>
    where
        Self: 'db;

    /// Creates a Foundry-wrapped EVM with the given inspector.
    ///
    /// Callers carrying execution context must install it through the returned context's
    /// `chain_mut` before executing. This also preserves OP's block-derived L1 fee information.
    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I>;

    /// Creates a Foundry-wrapped nested EVM without an inspector.
    fn create_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
    ) -> NestedEvmFor<'db, Self> {
        self.create_nested_evm_with_inspector(db, evm_env, NoOpInspector)
    }

    /// Creates a Foundry-wrapped nested EVM with the given inspector.
    /// Install inherited chain state with [`NestedEvm::chain_mut`] before executing or restoring
    /// journal-derived state.
    fn create_nested_evm_with_inspector<'db, I>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: I,
    ) -> NestedEvmFor<'db, Self>
    where
        I: FoundryInspectorExt<Self::FoundryContext<'db>> + 'db;
}

/// Object-safe EVM operations used by nested execution and fork replay.
///
/// This abstracts over the concrete EVM type (`FoundryEvm`, future `TempoEvm`, etc.)
/// so that cheatcode impls can build and run nested EVMs without knowing the concrete type.
pub trait NestedEvm {
    /// The spec type.
    type Spec;
    /// The block environment type.
    type Block;
    /// The transaction environment type.
    type Tx: FoundryTransaction;
    /// Chain context identifying the active transaction position.
    type Chain: FoundryChain<Self::Tx>;
    /// The Journal type, which may own Monad's reserve-balance-tracker state.
    type Journal: FoundryJournal;
    /// Returns a mutable reference to the journal inner state (`JournaledState`).
    fn journal_inner_mut(&mut self) -> &mut JournaledState;

    /// Returns a mutable reference to the transaction environment.
    fn tx_mut(&mut self) -> &mut Self::Tx;

    /// Returns a mutable reference to the chain-position context.
    fn chain_mut(&mut self) -> &mut Self::Chain;

    /// Returns the precompile map.
    fn precompiles_mut(&mut self) -> &mut PrecompilesMap;

    /// Returns a mutable reference to the Journal.
    fn journal_mut(&mut self) -> &mut Self::Journal;

    /// Runs a single execution frame (create or call) through the EVM handler loop.
    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>>;

    /// Executes a full transaction with the given tx env.
    fn transact_raw(&mut self, tx: Self::Tx) -> eyre::Result<ResultAndState<HaltReason>>;

    /// Replays a transaction, skipping unsupported system envelopes.
    ///
    /// `is_system` preserves the RPC envelope classification that conversion to `Self::Tx` may
    /// discard. Returning `None` must not mutate the EVM, database, or inspector.
    fn transact_replay(
        &mut self,
        tx: Self::Tx,
        is_system: bool,
    ) -> eyre::Result<Option<ResultAndState<HaltReason>>> {
        if is_system {
            return Ok(None);
        }
        self.transact_raw(tx).map(Some)
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block>;
}

/// Converts a network-specific halt reason into an [`InstructionResult`].
pub trait IntoInstructionResult {
    fn into_instruction_result(self) -> InstructionResult;
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

pub type ChainFor<FEN> = <EvmFactoryFor<FEN> as FoundryEvmFactory>::Chain;

/// Boxed nested EVM produced by a Foundry EVM factory.
pub type NestedEvmFor<'db, F> = Box<
    dyn NestedEvm<
            Spec = <F as EvmFactory>::Spec,
            Block = <F as EvmFactory>::BlockEnv,
            Tx = <F as EvmFactory>::Tx,
            Chain = <F as FoundryEvmFactory>::Chain,
            Journal = <<F as FoundryEvmFactory>::FoundryContext<'db> as ContextTr>::Journal,
        > + 'db,
>;

/// Closure type used by `CheatcodesExecutor` methods that run nested EVM operations.
pub type NestedEvmClosure<'a, F> = &'a mut dyn for<'j> FnMut(
    &mut dyn NestedEvm<
        Spec = <F as EvmFactory>::Spec,
        Block = <F as EvmFactory>::BlockEnv,
        Tx = <F as EvmFactory>::Tx,
        Chain = <F as FoundryEvmFactory>::Chain,
        Journal = <<F as FoundryEvmFactory>::FoundryContext<'j> as ContextTr>::Journal,
    >,
)
    -> Result<(), EVMError<DatabaseError>>;

/// Nested EVM closure for a Foundry EVM network.
pub type NestedEvmClosureFor<'a, FEN> = NestedEvmClosure<'a, EvmFactoryFor<FEN>>;

/// Runs a nested frame with inspection and settles its gas into the parent frame.
pub(crate) fn run_inspected_frame<H>(
    evm: &mut H::Evm,
    mut handler: H,
    frame_input: FrameInput,
) -> Result<FrameResult, H::Error>
where
    H: InspectorHandler<IT = EthInterpreter>,
    H::Evm: InspectorEvmTr,
{
    let memory =
        SharedMemory::new_with_buffer(evm.ctx_ref().local().shared_memory_buffer().clone());
    let first_frame_input = FrameInit { depth: 0, memory, frame_input };
    let mut frame_result = handler.inspect_run_exec_loop(evm, first_frame_input)?;
    let mut parent_gas = GasTracker::new(
        frame_result.gas().limit(),
        frame_result.gas().remaining(),
        frame_result.gas().reservoir(),
    );
    handler.last_frame_result(evm, &mut frame_result, &mut parent_gas)?;
    Ok(frame_result)
}

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
    let journal_inner = journal_inner.clone();

    let (sub_evm_env, sub_inner) = f(db, evm_env, journal_inner)?;

    // Write back modified state. The db borrow was released when f returned.
    ecx.set_journal_inner(sub_inner);
    ecx.set_evm(sub_evm_env);

    Ok(())
}

/// Get the call inputs for the CREATE2 factory.
pub fn get_create2_factory_call_inputs<T: JournalTr>(
    salt: U256,
    inputs: &CreateInputs,
    deployer: Address,
    journal: &mut T,
) -> Result<CallInputs, <T::Database as Database>::Error> {
    let calldata = [&salt.to_be_bytes::<32>()[..], &inputs.init_code()[..]].concat();
    let account = journal.load_account_with_code(deployer)?;
    Ok(CallInputs {
        caller: inputs.caller(),
        bytecode_address: deployer,
        known_bytecode: (account.info.code_hash, account.info.code.clone().unwrap_or_default()),
        target_address: deployer,
        scheme: CallScheme::Call,
        value: CallValue::Transfer(inputs.value()),
        input: CallInput::Bytes(calldata.into()),
        gas_limit: inputs.gas_limit(),
        reservoir: inputs.reservoir(),
        is_static: false,
        return_memory_offset: 0..0,
        charged_new_account_state_gas: false,
    })
}
