//! Shared EVM traits, associated types, and execution helpers.
//!
//! Each network module owns its network marker and concrete EVM implementations.

use crate::{
    FoundryBlock, FoundryChain, FoundryContextExt, FoundryInspectorExt, FoundryJournal,
    FoundryTransaction, FromAnyRpcTransaction,
    backend::{DatabaseExt, JournaledState},
    refresh_chain_journal,
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
    state::{AccountStatus, EvmState},
};
use serde::{Deserialize, Serialize};
use std::{fmt::Debug, ops::DerefMut};

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
            Precompiles = PrecompilesMap,
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

/// Runs a child operation with the parent's environment, journal, and native chain state.
///
/// Publishes child environment, journal, and chain changes only when the operation returns `Ok`.
/// This does not roll back database or inspector effects: callers retain their existing ownership
/// of those effects. The outer transaction environment is not replaced.
///
/// Both inspector adapters use this operation so inheritance and write-back remain paired. The
/// existing Monad journal bridge is retained here until native journal lifecycle ownership
/// migrates.
pub fn with_inherited_evm<F, I>(
    ecx: &mut F::FoundryContext<'_>,
    inspector: I,
    f: NestedEvmClosure<'_, F>,
) -> Result<(), EVMError<DatabaseError>>
where
    F: FoundryEvmFactory,
    I: for<'db> FoundryInspectorExt<F::FoundryContext<'db>>,
{
    let evm_env = ecx.evm_clone();
    let chain_context = ecx.chain().clone();
    #[cfg(feature = "monad")]
    let mut reserve_balance = FoundryJournal::capture_reserve_balance(ecx.journal());
    let (evm_env, journaled_state, chain_context) = {
        let (db, journaled_state) = ecx.db_journal_inner_mut();
        let journaled_state = journaled_state.clone();
        let mut evm = F::default().create_nested_evm_with_inspector(db, evm_env, inspector);
        *evm.chain_mut() = chain_context;
        *evm.journal_inner_mut() = journaled_state;
        #[cfg(feature = "monad")]
        {
            FoundryJournal::restore_reserve_balance(evm.journal_mut(), reserve_balance);
            refresh_nested_chain_journal(&mut *evm);
        }
        f(&mut *evm)?;
        #[cfg(feature = "monad")]
        {
            reserve_balance = FoundryJournal::capture_reserve_balance(evm.journal_mut());
        }
        (evm.to_evm_env(), evm.journal_inner_mut().clone(), evm.chain_mut().clone())
    };
    ecx.set_journal_inner(journaled_state);
    ecx.set_evm(evm_env);
    *ecx.chain_mut() = chain_context;
    #[cfg(feature = "monad")]
    FoundryJournal::restore_reserve_balance(ecx.journal_mut(), reserve_balance);
    refresh_chain_journal(ecx);
    Ok(())
}

/// Prepares account state for execution across a synthetic transaction boundary.
///
/// Preserve account flags, including local creation, while making accounts outside the protocol
/// warm-address set cold. All storage starts cold with its current value as the child's original
/// value. The parent's state is unchanged.
pub fn prepare_child_state(journal: &JournaledState) -> EvmState {
    let mut state = journal.state.clone();
    for (address, account) in &mut state {
        if journal.warm_addresses.is_cold(address) {
            account.mark_cold();
        }
        for slot in account.storage.values_mut() {
            slot.is_cold = true;
            slot.original_value = slot.present_value;
        }
    }
    state
}

/// Merges a child's returned account state into its suspended parent.
///
/// Preserve parent warmth and original storage values, import child account flags and current
/// values, and optionally remove parent accounts and slots absent from the child. Newly loaded
/// accounts and slots keep their child metadata. This operates on the EVM's returned state, not on
/// an unfiltered write set; the caller retains responsibility for execution errors and
/// family-specific reconciliation.
pub fn merge_child_state(parent: &mut EvmState, child: EvmState, remove_absent: bool) {
    if remove_absent {
        parent.retain(|address, parent_account| {
            let Some(child_account) = child.get(address) else { return false };
            parent_account.storage.retain(|key, _| child_account.storage.contains_key(key));
            true
        });
    }

    for (address, mut account) in child {
        let Some(parent_account) = parent.get_mut(&address) else {
            parent.insert(address, account);
            continue;
        };
        if account.status.contains(AccountStatus::Cold)
            && !parent_account.status.contains(AccountStatus::Cold)
        {
            account.status -= AccountStatus::Cold;
        }
        parent_account.info = account.info;
        parent_account.status |= account.status;
        for (key, slot) in account.storage {
            let Some(parent_slot) = parent_account.storage.get_mut(&key) else {
                parent_account.storage.insert(key, slot);
                continue;
            };
            parent_slot.present_value = slot.present_value;
            parent_slot.is_cold &= slot.is_cold;
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    use alloy_evm::EthEvmFactory;
    use revm::{
        context::Transaction,
        state::{Account, AccountInfo, EvmStorageSlot, TransactionId},
    };

    #[cfg(feature = "monad")]
    use alloy_monad_evm::MonadEvmFactory;
    #[cfg(feature = "monad")]
    use monad_revm::{MonadHardfork, MonadJournalTr, reserve_balance::tracker::ReserveBalanceInit};
    #[cfg(feature = "monad")]
    use revm::context::{BlockEnv, CfgEnv};

    #[test]
    fn inherited_journal_publishes_only_after_success() {
        let address = Address::with_last_byte(0x42);
        for succeeds in [false, true] {
            let mut db = Backend::<EthEvmNetwork>::spawn(None).unwrap();
            let mut parent = EthEvmFactory::default().create_foundry_evm_with_inspector(
                &mut db,
                EvmEnvFor::<EthEvmNetwork>::default(),
                NoOpInspector,
            );
            parent.journaled_state.inner.depth = 3;
            parent
                .journaled_state
                .inner
                .state
                .insert(address, Account::from(AccountInfo::from_balance(U256::from(7))));
            let caller = parent.tx().caller();
            let result =
                with_inherited_evm::<EthEvmFactory, _>(&mut parent, NoOpInspector, &mut |child| {
                    assert_eq!(child.journal_inner_mut().depth, 3);
                    assert_eq!(
                        child.journal_inner_mut().state[&address].info.balance,
                        U256::from(7)
                    );
                    child.journal_inner_mut().depth = 4;
                    child.journal_inner_mut().state.get_mut(&address).unwrap().info.balance =
                        U256::from(9);
                    child.tx_mut().caller = address;
                    if succeeds { Ok(()) } else { Err(EVMError::Custom("abort child".into())) }
                });
            assert_eq!(result.is_ok(), succeeds);
            assert_eq!(parent.journaled_state.inner.depth, if succeeds { 4 } else { 3 });
            assert_eq!(
                parent.journaled_state.inner.state[&address].info.balance,
                U256::from(if succeeds { 9 } else { 7 })
            );
            assert_eq!(parent.tx().caller(), caller);
        }
    }

    #[cfg(feature = "monad")]
    #[test]
    fn inherited_monad_tracker_and_chain_publish_together() {
        let sender = Address::with_last_byte(0x42);
        for succeeds in [false, true] {
            let mut db = Backend::<MonadEvmNetwork>::spawn(None).unwrap();
            let mut parent = MonadEvmFactory::default().create_foundry_evm_with_inspector(
                &mut db,
                EvmEnv::new(CfgEnv::new_with_spec(MonadHardfork::MonadNine), BlockEnv::default()),
                NoOpInspector,
            );
            let account = Account::from(AccountInfo::from_balance(U256::from(12)));
            let chain = parent.chain().clone();
            parent.journaled_state.reserve_balance_mut().init(ReserveBalanceInit {
                chain: &chain,
                spec: MonadHardfork::MonadNine,
                sender,
                effective_gas_price: 0,
                gas_limit: 0,
                sender_is_delegated: false,
                sender_account: Some(&account),
            });
            parent.journaled_state.inner.state.insert(sender, account);
            let tracker = parent.journaled_state.reserve_balance().clone();
            let result = with_inherited_evm::<MonadEvmFactory, _>(
                &mut parent,
                NoOpInspector,
                &mut |child| {
                    assert_eq!(child.journal_mut().reserve_balance(), &tracker);
                    child.chain_mut().parent_senders_and_authorities.insert(sender);
                    child.journal_inner_mut().state.get_mut(&sender).unwrap().info.balance =
                        U256::from(9);
                    if succeeds { Ok(()) } else { Err(EVMError::Custom("abort child".into())) }
                },
            );
            assert_eq!(result.is_ok(), succeeds);
            if succeeds {
                assert!(parent.chain().parent_senders_and_authorities.contains(&sender));
                assert!(parent.journaled_state.reserve_balance().has_violation());
            } else {
                assert_eq!(parent.chain(), &chain);
                assert_eq!(parent.journaled_state.reserve_balance(), &tracker);
                assert_eq!(
                    parent.journaled_state.inner.state[&sender].info.balance,
                    U256::from(12)
                );
            }
        }
    }

    #[test]
    fn preparation_preserves_creation_and_protocol_warmth() {
        let address = Address::with_last_byte(0x42);
        let protocol_address = Address::with_last_byte(0x43);
        let key = U256::ONE;
        let mut account = Account::from(AccountInfo::default());
        account.mark_created_locally();
        account.mark_touch();
        account.storage.insert(
            key,
            EvmStorageSlot::new_changed(U256::from(3), U256::from(7), TransactionId::ZERO),
        );
        let mut journal = JournaledState::default();
        journal.state.insert(address, account.clone());
        journal.state.insert(protocol_address, account);
        journal.warm_addresses.set_coinbase(protocol_address);
        let before = journal.state.clone();

        let child = prepare_child_state(&journal);

        assert_eq!(journal.state, before);
        assert!(child[&address].is_created_locally());
        assert!(child[&address].is_touched());
        assert!(child[&address].status.contains(AccountStatus::Cold));
        assert!(!child[&protocol_address].status.contains(AccountStatus::Cold));
        for account in child.values() {
            assert_eq!(account.storage[&key].original_value, U256::from(7));
            assert_eq!(account.storage[&key].present_value, U256::from(7));
            assert!(account.storage[&key].is_cold);
        }
    }

    #[test]
    fn settlement_preserves_parent_original_values_and_combines_warmth() {
        let address = Address::with_last_byte(0x42);
        let key = U256::ONE;
        for parent_cold in [false, true] {
            for child_cold in [false, true] {
                let mut account = Account::from(AccountInfo::default());
                account.status.set(AccountStatus::Cold, parent_cold);
                account.mark_created_locally();
                let mut slot =
                    EvmStorageSlot::new_changed(U256::from(3), U256::from(7), TransactionId::ZERO);
                slot.is_cold = parent_cold;
                account.storage.insert(key, slot);
                let mut parent = EvmState::from_iter([(address, account)]);
                let mut account = Account::from(AccountInfo::from_balance(U256::from(9)));
                account.status.set(AccountStatus::Cold, child_cold);
                account.mark_touch();
                let mut slot =
                    EvmStorageSlot::new_changed(U256::from(7), U256::from(11), TransactionId::ZERO);
                slot.is_cold = child_cold;
                account.storage.insert(key, slot);

                merge_child_state(&mut parent, EvmState::from_iter([(address, account)]), false);

                let account = &parent[&address];
                assert!(account.is_created_locally());
                assert!(account.is_touched());
                assert_eq!(account.info.balance, U256::from(9));
                // Account flags are unioned; a cold parent retains its flag until journal access.
                assert_eq!(account.status.contains(AccountStatus::Cold), parent_cold);
                assert_eq!(account.storage[&key].original_value, U256::from(3));
                assert_eq!(account.storage[&key].present_value, U256::from(11));
                assert_eq!(account.storage[&key].is_cold, parent_cold && child_cold);
            }
        }
    }

    #[test]
    fn settlement_only_removes_state_absent_from_child_when_requested() {
        let retained_address = Address::with_last_byte(0x42);
        let removed_address = Address::with_last_byte(0x43);
        let retained_key = U256::ONE;
        let removed_key = U256::from(2);
        let mut retained_account = Account::from(AccountInfo::default());
        retained_account
            .storage
            .insert(retained_key, EvmStorageSlot::new(U256::ONE, TransactionId::ZERO));
        retained_account
            .storage
            .insert(removed_key, EvmStorageSlot::new(U256::from(2), TransactionId::ZERO));
        let mut parent = EvmState::from_iter([
            (retained_address, retained_account),
            (removed_address, Account::from(AccountInfo::default())),
        ]);
        let mut child_account = Account::from(AccountInfo::default());
        child_account
            .storage
            .insert(retained_key, EvmStorageSlot::new(U256::ONE, TransactionId::ZERO));

        let child = EvmState::from_iter([(retained_address, child_account)]);

        let mut retained = parent.clone();
        merge_child_state(&mut retained, child.clone(), false);

        assert!(retained.contains_key(&removed_address));
        assert!(retained[&retained_address].storage.contains_key(&removed_key));

        merge_child_state(&mut parent, child, true);

        assert!(!parent.contains_key(&removed_address));
        assert!(parent[&retained_address].storage.contains_key(&retained_key));
        assert!(!parent[&retained_address].storage.contains_key(&removed_key));
    }
}
