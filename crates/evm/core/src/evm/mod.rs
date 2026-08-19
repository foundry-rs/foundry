use std::{fmt::Debug, ops::Deref};

use crate::{
    FoundryBlock, FoundryChain, FoundryContextExt, FoundryInspectorExt, FoundryJournal,
    FoundryTransaction, FromAnyRpcTransaction,
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
#[cfg(feature = "monad")]
use revm::inspector::Inspector;
use revm::{
    Database,
    context::{
        ContextTr, JournalTr,
        result::{EVMError, HaltReason, ResultAndState},
    },
    handler::FrameResult,
    inspector::NoOpInspector,
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
    type Chain: FoundryChain;

    /// Additional network-specific cheatcode contract addresses.
    const EXTRA_CHEATCODE_ADDRESSES: &'static [Address] = &[];

    /// Maximum initcode size enforced during nested transaction execution.
    const CONTRACT_INITCODE_SIZE_LIMIT: usize = MAX_INITCODE_SIZE;

    /// Whether transaction execution needs metadata from surrounding blocks.
    const NEEDS_BLOCK_CONTEXT: bool = false;

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
        > + Deref<Target = Self::FoundryContext<'db>>
    where
        Self: 'db;

    /// Creates a Foundry-wrapped EVM with the given inspector.
    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        chain_context: Self::Chain,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I>;

    /// Builds chain context for a standalone synthetic transaction.
    fn chain_context_for_transaction(&self, _tx: &Self::Tx) -> Self::Chain {
        Self::Chain::default()
    }

    /// Builds chain context for a transaction at an exact block position.
    fn chain_context_for_block(
        &self,
        _grandparent: &[Self::Tx],
        _parent: &[Self::Tx],
        _current: &[Self::Tx],
        _current_tx_index: usize,
    ) -> Self::Chain {
        Self::Chain::default()
    }

    /// Tries to execute a canonical system transaction on a regular Alloy EVM during replay.
    ///
    /// Returning `Ok(None)` means the transaction was not recognized. Implementations must not
    /// mutate the EVM, its database, or inspector before returning `Ok(None)`, because callers may
    /// fall back to ordinary execution using the same EVM instance.
    ///
    /// Implementations that recognize a transaction here must provide equivalent recognition in
    /// [`Self::try_transact_foundry_system_replay`].
    #[cfg(feature = "monad")]
    fn try_transact_system_replay<DB, I>(
        &self,
        _evm: &mut Self::Evm<DB, I>,
        _tx: &Self::Tx,
    ) -> eyre::Result<Option<ResultAndState<Self::HaltReason>>>
    where
        DB: alloy_evm::Database,
        I: Inspector<Self::Context<DB>>,
    {
        Ok(None)
    }

    /// Tries to execute a canonical system transaction on a Foundry-wrapped EVM with an inspector.
    ///
    /// Returning `Ok(None)` means the transaction was not recognized. Implementations must not
    /// mutate the EVM, its database, or inspector before returning `Ok(None)`, because callers may
    /// fall back to ordinary execution using the same EVM instance.
    ///
    /// Implementations that recognize a transaction here must provide equivalent recognition in
    /// [`Self::try_transact_system_replay`].
    #[cfg(feature = "monad")]
    fn try_transact_foundry_system_replay<
        'db,
        I: FoundryInspectorExt<Self::FoundryContext<'db>>,
    >(
        &self,
        _evm: &mut Self::FoundryEvm<'db, I>,
        _tx: &Self::Tx,
    ) -> eyre::Result<Option<ResultAndState<Self::HaltReason>>> {
        Ok(None)
    }

    /// Creates an uninspected EVM with explicit transaction-position context.
    fn create_evm_with_context<DB: alloy_evm::Database>(
        &self,
        db: DB,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        chain_context: Self::Chain,
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
        chain_context: Self::Chain,
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
    type Tx: FoundryTransaction;
    /// Chain context identifying the active transaction position.
    type Chain: FoundryChain;
    /// The Journal type, which may own Monad's reserve-balance-tracker state.
    type Journal: FoundryJournal;
    /// Returns a mutable reference to the journal inner state (`JournaledState`).
    fn journal_inner_mut(&mut self) -> &mut JournaledState;

    /// Returns a mutable reference to the transaction environment.
    fn tx_mut(&mut self) -> &mut Self::Tx;

    /// Returns a mutable reference to the chain-position context.
    fn chain_mut(&mut self) -> &mut Self::Chain;

    /// Returns a mutable reference to the Journal.
    fn journal_mut(&mut self) -> &mut Self::Journal;

    /// Resyncs Monad's reserve-balance-tracker state that depends on the current chain-position
    /// context.
    ///
    /// See [`FoundryContextExt::refresh_chain_dependent_state`]; this is the equivalent hook
    /// for the object-safe nested-EVM boundary, which doesn't implement `FoundryContextExt`.
    fn refresh_chain_dependent_state(&mut self) {}

    /// Runs a single execution frame (create or call) through the EVM handler loop.
    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>>;

    /// Executes a full transaction with the given tx env.
    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<HaltReason>, EVMError<DatabaseError>>;

    /// Executes a canonical replay transaction.
    #[cfg(feature = "monad")]
    fn transact_replay(&mut self, tx: Self::Tx) -> eyre::Result<ResultAndState<HaltReason>> {
        self.transact_raw(tx).map_err(Into::into)
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block>;
}

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
