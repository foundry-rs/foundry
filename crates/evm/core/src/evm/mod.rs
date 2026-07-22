use std::{fmt::Debug, ops::Deref};

#[cfg(feature = "monad")]
use crate::constants::MONAD_CHEATCODE_ADDRESS;
use crate::{
    FoundryBlock, FoundryContextExt, FoundryContextState, FoundryEvmAuxState, FoundryInspectorExt,
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
use revm::{
    Database,
    context::{
        JournalTr,
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

mod replay;
pub use replay::*;

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

    /// Additional network-specific cheatcode contract addresses.
    const EXTRA_CHEATCODE_ADDRESSES: &'static [Address] = &[];

    /// Maximum initcode size enforced when nested cheatcode execution simulates a raw deployment.
    const CONTRACT_INITCODE_SIZE_LIMIT: usize = MAX_INITCODE_SIZE;

    fn is_extra_cheatcode_address(address: Address) -> bool {
        Self::EXTRA_CHEATCODE_ADDRESSES.contains(&address)
    }
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

    const EXTRA_CHEATCODE_ADDRESSES: &'static [Address] = &[MONAD_CHEATCODE_ADDRESS];
    const CONTRACT_INITCODE_SIZE_LIMIT: usize = monad_revm::MONAD_MAX_INITCODE_SIZE;
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
pub type ContextAuxFor<FEN> = <EvmFactoryFor<FEN> as FoundryEvmFactory>::ContextAux;
pub type ContextStateFor<FEN> = FoundryContextState<ContextAuxFor<FEN>>;

pub type NetworkFor<FEN> = <FEN as FoundryEvmNetwork>::Network;
pub type TxEnvelopeFor<FEN> = <NetworkFor<FEN> as Network>::TxEnvelope;
pub type TransactionRequestFor<FEN> = <NetworkFor<FEN> as Network>::TransactionRequest;
pub type TransactionResponseFor<FEN> = <NetworkFor<FEN> as Network>::TransactionResponse;
pub type BlockResponseFor<FEN> = <NetworkFor<FEN> as Network>::BlockResponse;

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
    /// Whether transaction execution needs metadata from surrounding blocks.
    const NEEDS_BLOCK_CONTEXT: bool = false;

    /// Whether this EVM family executes the EIP-4788 beacon-roots system call.
    const USES_EIP4788_BEACON_ROOTS: bool = true;

    /// Network-specific state stored outside the standard REVM journal.
    type ContextAux: FoundryEvmAuxState;

    /// Foundry Context abstraction
    type FoundryContext<'db>: FoundryContextExt<
            Block = Self::BlockEnv,
            Tx = Self::Tx,
            Spec = Self::Spec,
            Aux = Self::ContextAux,
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
        context_aux: Self::ContextAux,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I>;

    /// Returns the auxiliary state for a standalone synthetic transaction.
    fn context_for_transaction(&self, _tx: &Self::Tx) -> Self::ContextAux {
        Self::ContextAux::default()
    }

    /// Returns the auxiliary state for a transaction at an exact position in a block.
    fn context_for_block(
        &self,
        _grandparent: &[Self::Tx],
        _parent: &[Self::Tx],
        _current: &[Self::Tx],
        _current_tx_index: usize,
    ) -> Self::ContextAux {
        Self::ContextAux::default()
    }

    /// Converts a canonical envelope into a family-specific protocol system call.
    fn protocol_system_call(&self, _tx: &Self::Tx) -> Option<ProtocolSystemCall> {
        None
    }

    /// Creates an uninspected EVM with explicit network-specific auxiliary state.
    fn create_evm_with_context<DB: alloy_evm::Database>(
        &self,
        db: DB,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        context_aux: Self::ContextAux,
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
        context_aux: Self::ContextAux,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> Box<
        dyn NestedEvm<
                Spec = Self::Spec,
                Block = Self::BlockEnv,
                Tx = Self::Tx,
                Aux = Self::ContextAux,
            > + 'db,
    >;
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
    /// Network-specific state stored outside the standard REVM journal.
    type Aux: FoundryEvmAuxState;

    /// Returns a mutable reference to the journal inner state (`JournaledState`).
    fn journal_inner_mut(&mut self) -> &mut JournaledState;

    /// Clones the complete context state.
    fn context_state(&self) -> FoundryContextState<Self::Aux>;

    /// Clones the network-specific auxiliary state.
    fn aux_state(&self) -> Self::Aux;

    /// Restores the complete context state.
    fn set_context_state(&mut self, state: FoundryContextState<Self::Aux>);

    /// Preserves auxiliary state across the next synthetic transaction boundary.
    fn preserve_aux_state_on_transaction(&mut self) {}

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
pub type NestedEvmClosure<'a, Spec, Block, Tx, Aux> =
    &'a mut dyn FnMut(
        &mut dyn NestedEvm<Spec = Spec, Block = Block, Tx = Tx, Aux = Aux>,
    ) -> Result<(), EVMError<DatabaseError>>;

/// Clones the current context (env + journal), passes the database, cloned env,
/// and cloned context state to the callback. The callback builds whatever EVM it
/// needs, runs its operations, and returns `(result, modified_env, modified_context)`.
/// Modified state is written back after the callback returns.
pub fn with_cloned_context<CTX: FoundryContextExt>(
    ecx: &mut CTX,
    f: impl FnOnce(
        &mut CTX::Db,
        EvmEnv<CTX::Spec, CTX::Block>,
        FoundryContextState<CTX::Aux>,
    ) -> Result<
        (EvmEnv<CTX::Spec, CTX::Block>, FoundryContextState<CTX::Aux>),
        EVMError<DatabaseError>,
    >,
) -> Result<(), EVMError<DatabaseError>> {
    let evm_env = ecx.evm_clone();
    let context_state = ecx.context_state();

    let db = ecx.db_mut();

    let (sub_evm_env, sub_state) = f(db, evm_env, context_state)?;

    // Write back modified state. The db borrow was released when f returned.
    ecx.set_context_state(sub_state);
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

#[cfg(all(test, feature = "monad"))]
mod tests {
    use super::*;

    #[test]
    fn monad_overrides_nested_initcode_size_limit() {
        assert_eq!(
            <EthEvmNetwork as FoundryEvmNetwork>::CONTRACT_INITCODE_SIZE_LIMIT,
            MAX_INITCODE_SIZE
        );
        assert_eq!(
            <TempoEvmNetwork as FoundryEvmNetwork>::CONTRACT_INITCODE_SIZE_LIMIT,
            MAX_INITCODE_SIZE
        );
        assert_eq!(
            <MonadEvmNetwork as FoundryEvmNetwork>::CONTRACT_INITCODE_SIZE_LIMIT,
            monad_revm::MONAD_MAX_INITCODE_SIZE
        );
    }
}
