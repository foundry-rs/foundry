use std::{fmt::Debug, ops::Deref};

use crate::{
    FoundryBlock, FoundryContextExt, FoundryInspectorExt, FoundryTransaction,
    FromAnyRpcTransaction,
    backend::{DatabaseExt, JournaledState},
};
use alloy_consensus::{SignableTransaction, Signed, transaction::SignerRecoverable};
use alloy_evm::{
    EthEvmFactory, Evm, EvmEnv, EvmFactory, FromRecoveredTx, precompiles::PrecompilesMap,
};
use alloy_network::{Ethereum, Network};
use alloy_op_evm::OpEvmFactory;
use alloy_primitives::{Address, Signature, U256};
use alloy_rlp::Decodable;
use foundry_common::{FoundryReceiptResponse, FoundryTransactionBuilder, fmt::UIfmt};
use foundry_config::FromEvmVersion;
use foundry_fork_db::{DatabaseError, ForkBlockEnv};
use op_alloy_network::Optimism;
use op_revm::OpHaltReason;
use revm::{
    Database,
    context::{
        JournalTr,
        result::{EVMError, HaltReason, ResultAndState},
    },
    handler::FrameResult,
    interpreter::{
        CallInput, CallInputs, CallScheme, CallValue, CreateInputs, FrameInput, InstructionResult,
    },
    primitives::hardfork::SpecId,
};
use serde::{Deserialize, Serialize};
use tempo_alloy::TempoNetwork;
use tempo_evm::evm::TempoEvmFactory;
use tempo_revm::TempoHaltReason;

pub mod eth;
pub mod op;
pub mod tempo;

pub use eth::*;
pub use op::*;
pub use tempo::*;

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

#[derive(Clone, Copy, Debug, Default)]
pub struct OpEvmNetwork;
impl FoundryEvmNetwork for OpEvmNetwork {
    type Network = Optimism;
    type EvmFactory = OpEvmFactory;
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
        Spec: Into<SpecId> + FromEvmVersion + Default + Copy + Unpin + Send + 'static,
        BlockEnv: FoundryBlock + ForkBlockEnv + Default + Unpin,
        Tx: Clone + Debug + FoundryTransaction + FromAnyRpcTransaction + Default + Send + Sync,
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
    })
}

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
