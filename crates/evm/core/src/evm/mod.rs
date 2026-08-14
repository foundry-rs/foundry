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
#[cfg(feature = "monad")]
use alloy_monad_evm::MonadEvmFactory;
use alloy_network::{Ethereum, Network};
use alloy_primitives::{Address, Signature, U256};
use alloy_rlp::Decodable;
use foundry_common::{FoundryReceiptResponse, FoundryTransactionBuilder, fmt::UIfmt};
use foundry_config::ExecutionSpec;
use foundry_fork_db::{DatabaseError, ForkBlockEnv};
use revm::{
    Database,
    context::{
        JournalTr,
        result::{EVMError, HaltReason, ResultAndState},
    },
    handler::FrameResult,
    inspector::{Inspector, NoOpInspector},
    interpreter::{
        CallInput, CallInputs, CallScheme, CallValue, CreateInputs, FrameInput, InstructionResult,
    },
    primitives::{eip3860::MAX_INITCODE_SIZE, hardfork::SpecId},
};
use serde::{Deserialize, Serialize};
use tempo_alloy::TempoNetwork;
use tempo_evm::evm::TempoEvmFactory;
use tempo_revm::TempoHaltReason;

pub mod eth;
#[cfg(feature = "monad")]
pub mod monad;
#[cfg(feature = "optimism")]
pub mod op;
pub mod tempo;

mod block_context;
pub use block_context::*;

pub use eth::*;
#[cfg(feature = "monad")]
pub use monad::*;
#[cfg(feature = "optimism")]
pub use op::*;
pub use tempo::*;

mod replay;
pub use replay::*;

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
#[cfg(feature = "monad")]
pub struct MonadEvmNetwork;
#[cfg(feature = "monad")]
impl FoundryEvmNetwork for MonadEvmNetwork {
    type Network = Ethereum;
    type EvmFactory = MonadEvmFactory;
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
pub type ChainContextFor<FEN> = <EvmFactoryFor<FEN> as FoundryEvmFactory>::ChainContext;
pub type TransactionStateFor<FEN> = <EvmFactoryFor<FEN> as FoundryEvmFactory>::TransactionState;

/// Boxed nested EVM produced by a Foundry EVM factory.
pub type NestedEvmFor<'db, F> = Box<
    dyn NestedEvm<
            Spec = <F as EvmFactory>::Spec,
            Block = <F as EvmFactory>::BlockEnv,
            Tx = <F as EvmFactory>::Tx,
            ChainContext = <F as FoundryEvmFactory>::ChainContext,
            TransactionState = <F as FoundryEvmFactory>::TransactionState,
        > + 'db,
>;

pub type NetworkFor<FEN> = <FEN as FoundryEvmNetwork>::Network;
pub type TxEnvelopeFor<FEN> = <NetworkFor<FEN> as Network>::TxEnvelope;
pub type TransactionRequestFor<FEN> = <NetworkFor<FEN> as Network>::TransactionRequest;
pub type TransactionResponseFor<FEN> = <NetworkFor<FEN> as Network>::TransactionResponse;
pub type BlockResponseFor<FEN> = <NetworkFor<FEN> as Network>::BlockResponse;

/// Rebases network caches after state changes that retain the active chain cursor.
pub fn refresh_context_after_state_change<FEN: FoundryEvmNetwork>(
    ecx: &mut FoundryContextFor<'_, FEN>,
) {
    FEN::EvmFactory::default().apply_context_transition(ecx, None);
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
    /// Chain context required to execute at an exact transaction position.
    type ChainContext: Clone + Debug + Default + Send + Sync + 'static;

    /// Family-owned state scoped to the active transaction.
    type TransactionState: Clone + Debug + Default + Send + Sync + 'static;

    /// Additional network-specific cheatcode contract addresses.
    const EXTRA_CHEATCODE_ADDRESSES: &'static [Address] = &[];

    /// Maximum initcode size enforced during nested transaction execution.
    const CONTRACT_INITCODE_SIZE_LIMIT: usize = MAX_INITCODE_SIZE;

    /// Whether transaction execution needs metadata from surrounding blocks.
    const NEEDS_BLOCK_CONTEXT: bool = false;

    /// Whether canonical protocol system transactions must be included during fork replay.
    const REPLAYS_PROTOCOL_SYSTEM_TRANSACTIONS: bool = false;

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
        chain_context: Self::ChainContext,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I>;

    /// Builds chain context for a standalone synthetic transaction.
    fn chain_context_for_transaction(&self, _tx: &Self::Tx) -> Self::ChainContext {
        Self::ChainContext::default()
    }

    /// Builds chain context for a transaction at an exact block position.
    fn chain_context_for_block(
        &self,
        _grandparent: &[Self::Tx],
        _parent: &[Self::Tx],
        _current: &[Self::Tx],
        _current_tx_index: usize,
    ) -> Self::ChainContext {
        Self::ChainContext::default()
    }

    /// Captures the active transaction position from a live EVM context.
    fn capture_chain_context(&self, _ecx: &Self::FoundryContext<'_>) -> Self::ChainContext {
        Self::ChainContext::default()
    }

    /// Applies a new transaction position and refreshes family-owned state after journal changes.
    fn apply_context_transition<'db>(
        &self,
        _ecx: &mut Self::FoundryContext<'db>,
        _replacement: Option<&Self::ChainContext>,
    ) {
    }

    /// Captures family-owned state for the active transaction.
    fn capture_transaction_state(&self, _ecx: &Self::FoundryContext<'_>) -> Self::TransactionState {
        Self::TransactionState::default()
    }

    /// Restores family-owned state for the active transaction.
    fn restore_transaction_state(
        &self,
        _ecx: &mut Self::FoundryContext<'_>,
        _state: Self::TransactionState,
    ) {
    }

    /// Converts a canonical envelope into a family-specific protocol system call.
    ///
    /// Returns an error when the transaction uses a network's reserved protocol sender but does
    /// not satisfy that network's canonical envelope rules.
    fn protocol_system_call(&self, _tx: &Self::Tx) -> eyre::Result<Option<ProtocolSystemCall>> {
        Ok(None)
    }

    /// Executes a canonical replay transaction on a regular EVM created by this factory.
    ///
    /// Factories with protocol system envelopes override this hook to apply their protocol
    /// prestate through the concrete EVM context before entering the dedicated system-call path.
    fn transact_replay<DB, I>(
        &self,
        evm: &mut Self::Evm<DB, I>,
        tx: Self::Tx,
    ) -> eyre::Result<ResultAndState<Self::HaltReason>>
    where
        DB: alloy_evm::Database,
        I: Inspector<Self::Context<DB>>,
    {
        if self.protocol_system_call(&tx)?.is_some() {
            eyre::bail!("protocol system replay is not implemented for this EVM factory");
        }
        evm.transact(tx).map_err(Into::into)
    }

    /// Executes a canonical replay transaction on a Foundry EVM with an inspector.
    fn transact_foundry_replay<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        evm: &mut Self::FoundryEvm<'db, I>,
        tx: Self::Tx,
    ) -> eyre::Result<ResultAndState<Self::HaltReason>> {
        if self.protocol_system_call(&tx)?.is_some() {
            eyre::bail!("protocol system replay is not implemented for this EVM factory");
        }
        evm.transact(tx).map_err(Into::into)
    }

    /// Creates an uninspected EVM with explicit transaction-position context.
    fn create_evm_with_context<DB: alloy_evm::Database>(
        &self,
        db: DB,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        chain_context: Self::ChainContext,
    ) -> Self::Evm<DB, NoOpInspector>;

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
        chain_context: Self::ChainContext,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> NestedEvmFor<'db, Self>;
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
    /// Chain context identifying the active transaction position.
    type ChainContext: Clone + Debug + Default + Send + Sync + 'static;
    /// Family-owned state scoped to the active transaction.
    type TransactionState: Clone + Debug + Default + Send + Sync + 'static;
    /// Returns a mutable reference to the journal inner state (`JournaledState`).
    fn journal_inner_mut(&mut self) -> &mut JournaledState;

    /// Captures the active transaction position.
    fn capture_chain_context(&self) -> Self::ChainContext {
        Self::ChainContext::default()
    }

    /// Captures family-owned state for the active transaction.
    fn capture_transaction_state(&self) -> Self::TransactionState {
        Self::TransactionState::default()
    }

    /// Restores family-owned state for the active transaction.
    fn restore_transaction_state(&mut self, _state: Self::TransactionState) {}

    /// Preserves transaction-scoped state across the next transaction boundary.
    fn preserve_transaction_state_on_next_transaction(&mut self) {}

    /// Runs a single execution frame (create or call) through the EVM handler loop.
    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>>;

    /// Executes a full transaction with the given tx env.
    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<HaltReason>, EVMError<DatabaseError>>;

    /// Executes a canonical replay transaction.
    ///
    /// Networks with protocol system envelopes must override this method so replay can apply the
    /// protocol prestate and bypass ordinary transaction validation.
    fn transact_replay(&mut self, tx: Self::Tx) -> eyre::Result<ResultAndState<HaltReason>> {
        self.transact_raw(tx).map_err(Into::into)
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block>;
}

/// Closure type used by `CheatcodesExecutor` methods that run nested EVM operations.
pub type NestedEvmClosure<'a, Spec, Block, Tx, ChainContext, TransactionState> =
    &'a mut dyn FnMut(
        &mut dyn NestedEvm<
            Spec = Spec,
            Block = Block,
            Tx = Tx,
            ChainContext = ChainContext,
            TransactionState = TransactionState,
        >,
    ) -> Result<(), EVMError<DatabaseError>>;

/// Nested EVM closure for a Foundry EVM network.
pub type NestedEvmClosureFor<'a, FEN> = NestedEvmClosure<
    'a,
    SpecFor<FEN>,
    BlockEnvFor<FEN>,
    TxEnvFor<FEN>,
    ChainContextFor<FEN>,
    TransactionStateFor<FEN>,
>;

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

/// Converts a network-specific halt reason into an [`InstructionResult`].
pub trait IntoInstructionResult {
    fn into_instruction_result(self) -> InstructionResult;
}

impl IntoInstructionResult for HaltReason {
    fn into_instruction_result(self) -> InstructionResult {
        self.into()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factories_define_nested_initcode_size_limit() {
        assert_eq!(EthEvmFactory::CONTRACT_INITCODE_SIZE_LIMIT, MAX_INITCODE_SIZE);
        assert_eq!(TempoEvmFactory::CONTRACT_INITCODE_SIZE_LIMIT, MAX_INITCODE_SIZE);
        #[cfg(feature = "monad")]
        assert_eq!(
            MonadEvmFactory::CONTRACT_INITCODE_SIZE_LIMIT,
            monad_revm::MONAD_MAX_INITCODE_SIZE
        );
    }
}
