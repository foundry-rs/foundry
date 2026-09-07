use alloy_evm::{Evm, EvmEnv, EvmFactory};
use alloy_monad_evm::{MonadEvm, MonadEvmFactory, MonadPrecompilesMap};
use alloy_sol_types::SolCall;
use eyre::WrapErr;
use foundry_fork_db::DatabaseError;
use monad_revm::{
    MonadBuilder, MonadCfgEnv, MonadChainContext, MonadContext, MonadEvm as RevmMonadEvm,
    MonadHardfork, MonadJournal, MonadJournalTr,
    api::block::{
        syscall_on_epoch_change_calldata, syscall_reward_calldata, syscall_snapshot_calldata,
    },
    handler::MonadHandler,
    instructions::MonadInstructions,
    monad_context_with_db,
    staking::{
        STAKING_ADDRESS,
        constants::SYSTEM_ADDRESS,
        interface::IMonadStaking::{
            syscallOnEpochChangeCall, syscallRewardCall, syscallSnapshotCall,
        },
    },
};
use revm::{
    context::{
        BlockEnv, ContextTr, Transaction, TransactionType, TxEnv,
        journaled_state::account::JournaledAccountTr,
        result::{EVMError, ResultAndState},
    },
    context_interface::{Cfg, ContextSetters, transaction::AuthorizationTr},
    handler::{EthFrame, EvmTr, FrameResult},
    inspector::{InspectSystemCallEvm, Inspector, InspectorHandler},
    interpreter::FrameInput,
    primitives::{Address, Bytes, HashSet, U256},
};

use crate::{
    FoundryChain, FoundryContextExt, FoundryInspectorExt, FoundryJournal,
    backend::{DatabaseExt, JournaledState},
    evm::{FoundryEvmFactory, NestedEvm, NestedEvmFor, run_inspected_frame},
};

impl FoundryChain<TxEnv> for MonadChainContext {
    fn for_transaction(tx: &TxEnv) -> Self {
        monad_context_from_participants(
            Default::default(),
            Default::default(),
            std::slice::from_ref(tx),
            0,
        )
    }

    fn for_block(
        grandparent: &[TxEnv],
        parent: &[TxEnv],
        current: &[TxEnv],
        current_tx_index: usize,
    ) -> Self {
        monad_context_from_participants(
            monad_block_participants(grandparent),
            monad_block_participants(parent),
            current,
            current_tx_index,
        )
    }

    fn refresh_journal<J: FoundryJournal>(&self, journal: &mut J) {
        let mut tracker = journal.capture_reserve_balance();
        tracker.rebase(self, journal.evm_state());
        journal.restore_reserve_balance(tracker);
    }
}

/// Refreshes journal state derived from a nested EVM's active Monad chain position.
pub fn refresh_nested_chain_journal<E: NestedEvm + ?Sized>(evm: &mut E) {
    let chain = evm.chain_mut().clone();
    chain.refresh_journal(evm.journal_mut());
}

type MonadEvmHandler<'db, I> =
    MonadHandler<MonadRevmEvm<'db, I>, EVMError<DatabaseError>, EthFrame>;

pub type MonadRevmEvm<'db, I> = RevmMonadEvm<
    MonadContext<&'db mut dyn DatabaseExt<MonadEvmFactory>>,
    I,
    MonadInstructions<MonadContext<&'db mut dyn DatabaseExt<MonadEvmFactory>>>,
    MonadPrecompilesMap,
>;

/// Senders and EIP-7702 authorities that participated in one Monad block.
pub type MonadBlockParticipants = HashSet<Address>;

/// Collects all senders and EIP-7702 authorities from a block's transactions.
pub fn monad_block_participants(transactions: &[TxEnv]) -> MonadBlockParticipants {
    transactions
        .iter()
        .flat_map(|tx| {
            std::iter::once(tx.caller())
                .chain(tx.authorization_list().filter_map(|auth| auth.authority()))
        })
        .collect()
}

/// Builds Monad context from cached ancestor participants and the current block transactions.
pub fn monad_context_from_participants(
    grandparent_senders_and_authorities: MonadBlockParticipants,
    parent_senders_and_authorities: MonadBlockParticipants,
    current: &[TxEnv],
    current_tx_index: usize,
) -> MonadChainContext {
    MonadChainContext {
        grandparent_senders_and_authorities,
        parent_senders_and_authorities,
        current_block_senders: current.iter().map(Transaction::caller).collect(),
        current_block_authorities: current
            .iter()
            .map(|tx| tx.authorization_list().filter_map(|auth| auth.authority()).collect())
            .collect(),
        current_tx_index,
        ..Default::default()
    }
}

/// A canonical Monad protocol system transaction.
#[derive(Clone, Debug)]
pub struct ProtocolSystemCall {
    /// Reserved caller used by the protocol.
    pub caller: Address,
    /// Native system contract or precompile being called.
    pub contract: Address,
    /// Calldata passed to the dedicated system-call entry point.
    pub data: Bytes,
    /// Sender nonce encoded by the canonical envelope.
    pub nonce: u64,
    /// Optional EIP-155 chain ID encoded by the canonical envelope.
    pub chain_id: Option<u64>,
    /// Optional protocol mint applied before system-call execution.
    pub balance_increment: Option<(Address, U256)>,
}

impl ProtocolSystemCall {
    fn validate_chain_id(&self, chain_id: u64) -> eyre::Result<()> {
        if let Some(envelope_chain_id) = self.chain_id
            && envelope_chain_id != chain_id
        {
            eyre::bail!(
                "protocol system transaction chain ID mismatch: envelope {envelope_chain_id}, \
                 environment {chain_id}"
            );
        }
        Ok(())
    }

    fn apply_prestate<DB: alloy_evm::Database>(
        &self,
        db: &mut DB,
        journal: &mut JournaledState,
    ) -> eyre::Result<()> {
        let next_nonce = self
            .nonce
            .checked_add(1)
            .ok_or_else(|| eyre::eyre!("protocol system transaction nonce overflow"))?;
        let caller_nonce = journal.load_account(db, self.caller)?.data.info.nonce;
        if caller_nonce != self.nonce {
            eyre::bail!(
                "protocol system transaction nonce mismatch: envelope {}, state {}",
                self.nonce,
                caller_nonce
            );
        }

        let balance = if let Some((address, amount)) = self.balance_increment {
            let balance = journal
                .load_account(db, address)?
                .data
                .info
                .balance
                .checked_add(amount)
                .ok_or_else(|| eyre::eyre!("protocol system transaction balance overflow"))?;
            Some((address, balance))
        } else {
            None
        };

        journal.load_account_mut(db, self.caller)?.data.set_nonce(next_nonce);
        if let Some((address, balance)) = balance {
            journal.load_account_mut(db, address)?.data.set_balance(balance);
        }

        Ok(())
    }
}

/// Converts a canonical Monad envelope into its dedicated system call.
///
/// Returns an error when the transaction uses Monad's reserved protocol sender but does not
/// satisfy the canonical envelope rules.
pub fn protocol_system_call<T: Transaction>(tx: &T) -> eyre::Result<Option<ProtocolSystemCall>> {
    if tx.caller() != SYSTEM_ADDRESS {
        return Ok(None);
    }

    eyre::ensure!(
        tx.tx_type() == TransactionType::Legacy as u8,
        "invalid Monad protocol system transaction: transaction type must be legacy"
    );
    eyre::ensure!(
        tx.kind() == revm::primitives::TxKind::Call(STAKING_ADDRESS),
        "invalid Monad protocol system transaction: target must be the staking contract"
    );
    eyre::ensure!(
        tx.gas_limit() == 0,
        "invalid Monad protocol system transaction: gas limit must be zero"
    );
    eyre::ensure!(
        tx.gas_price() == 0,
        "invalid Monad protocol system transaction: gas price must be zero"
    );
    eyre::ensure!(
        tx.max_priority_fee_per_gas().is_none(),
        "invalid Monad protocol system transaction: priority fee must be absent"
    );
    eyre::ensure!(
        tx.access_list().is_none_or(|mut list| list.next().is_none()),
        "invalid Monad protocol system transaction: access list must be empty"
    );
    eyre::ensure!(
        tx.blob_versioned_hashes().is_empty(),
        "invalid Monad protocol system transaction: blob hashes must be empty"
    );
    eyre::ensure!(
        tx.max_fee_per_blob_gas() == 0,
        "invalid Monad protocol system transaction: blob gas fee must be zero"
    );
    eyre::ensure!(
        tx.authorization_list_len() == 0,
        "invalid Monad protocol system transaction: authorization list must be empty"
    );

    let selector: [u8; 4] = tx
        .input()
        .get(..4)
        .ok_or_else(|| {
            eyre::eyre!(
                "invalid Monad protocol system transaction: calldata is shorter than a selector"
            )
        })?
        .try_into()
        .expect("slice has exactly four bytes");
    let (data, balance_increment) = match selector {
        syscallRewardCall::SELECTOR => {
            eyre::ensure!(
                tx.input().len() == 36,
                "invalid Monad protocol system transaction: reward calldata must be 36 bytes"
            );
            let call = syscallRewardCall::abi_decode_raw(&tx.input()[4..])
                .wrap_err("invalid Monad protocol system reward calldata")?;
            eyre::ensure!(
                call.abi_encode().as_slice() == tx.input(),
                "invalid Monad protocol system reward calldata"
            );
            (
                syscall_reward_calldata(call.blockAuthor, tx.value()),
                Some((STAKING_ADDRESS, tx.value())),
            )
        }
        syscallSnapshotCall::SELECTOR => {
            eyre::ensure!(
                tx.input().len() == 4,
                "invalid Monad protocol system transaction: snapshot calldata must be 4 bytes"
            );
            eyre::ensure!(
                tx.value().is_zero(),
                "invalid Monad protocol system transaction: snapshot value must be zero"
            );
            syscallSnapshotCall::abi_decode_raw(&tx.input()[4..])
                .wrap_err("invalid Monad protocol system snapshot calldata")?;
            (syscall_snapshot_calldata(), None)
        }
        syscallOnEpochChangeCall::SELECTOR => {
            eyre::ensure!(
                tx.input().len() == 36,
                "invalid Monad protocol system transaction: epoch calldata must be 36 bytes"
            );
            eyre::ensure!(
                tx.value().is_zero(),
                "invalid Monad protocol system transaction: epoch value must be zero"
            );
            let call = syscallOnEpochChangeCall::abi_decode_raw(&tx.input()[4..])
                .wrap_err("invalid Monad protocol system epoch calldata")?;
            eyre::ensure!(
                call.abi_encode().as_slice() == tx.input(),
                "invalid Monad protocol system epoch calldata"
            );
            (syscall_on_epoch_change_calldata(call.epoch), None)
        }
        _ => {
            return Err(eyre::eyre!(
                "invalid Monad protocol system transaction: unknown staking syscall selector"
            ));
        }
    };

    Ok(Some(ProtocolSystemCall {
        caller: SYSTEM_ADDRESS,
        contract: STAKING_ADDRESS,
        data,
        nonce: tx.nonce(),
        chain_id: tx.chain_id(),
        balance_increment,
    }))
}

fn finish_protocol_system_call<H>(
    mut result: ResultAndState<H>,
) -> eyre::Result<ResultAndState<H>> {
    if !result.result.is_success() {
        eyre::bail!("protocol system transaction reverted or halted");
    }

    if let revm::context_interface::result::ExecutionResult::Success { gas, .. } =
        &mut result.result
    {
        *gas = Default::default();
    }

    Ok(result)
}

/// Tries to execute a canonical Monad system transaction on an existing Monad EVM.
pub fn try_transact_monad_system_replay<DB, I>(
    evm: &mut MonadEvm<DB, I>,
    tx: &TxEnv,
) -> eyre::Result<Option<ResultAndState>>
where
    DB: alloy_evm::Database,
    I: Inspector<MonadContext<DB>>,
{
    let Some(system_call) = protocol_system_call(tx)? else {
        return Ok(None);
    };

    system_call.validate_chain_id(evm.chain_id())?;
    let journal = evm.ctx().journal_inner().clone();
    let chain = evm.ctx().chain.clone();
    let reserve_balance = evm.ctx().journaled_state.reserve_balance().clone();
    let result = (|| {
        let (db, journal) = evm.ctx_mut().db_journal_inner_mut();
        system_call.apply_prestate(db, journal)?;
        let result = evm
            .transact_system_call(system_call.caller, system_call.contract, system_call.data)
            .wrap_err("failed to execute protocol system transaction")?;
        finish_protocol_system_call(result)
    })();
    if result.is_err() {
        evm.ctx_mut().set_journal_inner(journal);
        evm.ctx_mut().chain = chain;
        *evm.ctx_mut().journaled_state.reserve_balance_mut() = reserve_balance;
    }
    result.map(Some)
}

impl FoundryEvmFactory for MonadEvmFactory {
    type Chain = MonadChainContext;

    type FoundryContext<'db> = MonadContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> =
        MonadEvm<&'db mut dyn DatabaseExt<Self>, I>;

    fn create_evm_with_context<DB: alloy_evm::Database>(
        &self,
        db: DB,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        chain_context: Self::Chain,
    ) -> Self::Evm<DB, revm::inspector::NoOpInspector> {
        let mut evm = self.create_evm(db, evm_env);
        evm.ctx_mut().chain = chain_context;
        evm
    }

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        chain_context: Self::Chain,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let mut monad_evm = self.create_evm_with_inspector(db, evm_env, inspector);
        monad_evm.ctx_mut().chain = chain_context;
        monad_evm.cfg.tx_chain_id_check = true;
        monad_evm
    }

    fn try_transact_system_replay<DB, I>(
        &self,
        evm: &mut Self::Evm<DB, I>,
        tx: &Self::Tx,
    ) -> eyre::Result<Option<ResultAndState<Self::HaltReason>>>
    where
        DB: alloy_evm::Database,
        I: Inspector<Self::Context<DB>>,
    {
        try_transact_monad_system_replay(evm, tx)
    }

    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        chain_context: Self::Chain,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> NestedEvmFor<'db, Self> {
        let spec = evm_env.cfg_env.spec;
        let monad_cfg = MonadCfgEnv::from(evm_env.cfg_env);
        let mut evm = monad_context_with_db(db)
            .with_block(evm_env.block_env)
            .with_cfg(monad_cfg)
            .build_monad_with_inspector(inspector)
            .with_precompiles(MonadPrecompilesMap::new_with_spec(spec));

        evm.0.ctx.chain = chain_context;
        evm.0.ctx.cfg.tx_chain_id_check = true;
        Box::new(evm)
    }
}

impl<'db, I: FoundryInspectorExt<MonadContext<&'db mut dyn DatabaseExt<MonadEvmFactory>>>> NestedEvm
    for MonadRevmEvm<'db, I>
{
    type Spec = MonadHardfork;
    type Block = BlockEnv;
    type Tx = TxEnv;
    type Chain = MonadChainContext;
    type Journal = MonadJournal<&'db mut dyn DatabaseExt<MonadEvmFactory>>;

    fn tx_mut(&mut self) -> &mut Self::Tx {
        self.ctx_mut().tx_mut()
    }

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx_mut().journaled_state.inner
    }

    fn chain_mut(&mut self) -> &mut Self::Chain {
        &mut self.ctx_mut().chain
    }

    fn journal_mut(&mut self) -> &mut Self::Journal {
        &mut self.ctx_mut().journaled_state
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        run_inspected_frame(self, MonadEvmHandler::<I>::new(), frame)
    }

    fn transact_raw(&mut self, tx: Self::Tx) -> eyre::Result<ResultAndState> {
        let Some(system_call) = protocol_system_call(&tx)? else {
            ContextSetters::set_tx(&mut self.0.ctx, tx);

            let mut handler = MonadEvmHandler::<I>::new();
            let result = handler.inspect_run(self)?;

            return Ok(ResultAndState::new(
                result,
                self.ctx_ref().journaled_state.inner.state.clone(),
            ));
        };

        system_call.validate_chain_id(self.ctx_ref().cfg().chain_id())?;
        let journal = self.ctx_ref().journal_inner().clone();
        let chain = self.ctx_ref().chain.clone();
        let reserve_balance = self.ctx_ref().journaled_state.reserve_balance().clone();
        let result = (|| {
            let (db, journal) = self.0.ctx.db_journal_inner_mut();
            system_call.apply_prestate(db, journal)?;
            let result = self
                .inspect_system_call_with_caller(
                    system_call.caller,
                    system_call.contract,
                    system_call.data,
                )
                .wrap_err("failed to execute protocol system transaction")?;
            finish_protocol_system_call(result)
        })();
        if result.is_err() {
            self.ctx_mut().set_journal_inner(journal);
            self.ctx_mut().chain = chain;
            *self.ctx_mut().journaled_state.reserve_balance_mut() = reserve_balance;
        }
        result
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block> {
        self.ctx_ref().evm_clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evm::{BlockContext, MonadEvmNetwork};
    use alloy_sol_types::SolEvent;
    use monad_revm::{
        reserve_balance::tracker::ReserveBalanceInit,
        staking::{
            constants::MON,
            interface::IMonadStaking::ValidatorRewarded,
            storage::{
                consensus_view_key, global_slots, val_id_secp_key, validator_key, validator_offsets,
            },
        },
    };
    use revm::{
        Database, DatabaseCommit,
        context::CfgEnv,
        context_interface::{
            either::Either,
            transaction::{
                AccessListItem, Authorization, RecoveredAuthority, RecoveredAuthorization,
            },
        },
        database::InMemoryDB,
        interpreter::{CallInputs, CallOutcome},
        primitives::{B256, TxKind, address},
        state::{Account, AccountInfo, EvmState},
    };

    #[derive(Default)]
    struct ProtocolPrestateInspector {
        call_count: usize,
        staking_balance: Option<U256>,
    }

    impl Inspector<MonadContext<InMemoryDB>> for ProtocolPrestateInspector {
        fn call(
            &mut self,
            context: &mut MonadContext<InMemoryDB>,
            _inputs: &mut CallInputs,
        ) -> Option<CallOutcome> {
            self.call_count += 1;
            self.staking_balance = context
                .journaled_state
                .inner
                .state
                .get(&STAKING_ADDRESS)
                .map(|account| account.info.balance);
            None
        }
    }

    fn transaction(caller: Address, authority: Address) -> TxEnv {
        let authorization = RecoveredAuthorization::new_unchecked(
            Authorization { chain_id: U256::from(1), address: Address::ZERO, nonce: 0 },
            RecoveredAuthority::Valid(authority),
        );
        TxEnv {
            caller,
            authorization_list: vec![Either::Right(authorization)],
            ..Default::default()
        }
    }

    fn system_transaction(data: Vec<u8>, value: U256) -> TxEnv {
        TxEnv {
            tx_type: TransactionType::Legacy as u8,
            caller: SYSTEM_ADDRESS,
            gas_limit: 0,
            kind: revm::primitives::TxKind::Call(STAKING_ADDRESS),
            data: data.into(),
            value,
            nonce: 3,
            chain_id: None,
            ..Default::default()
        }
    }

    fn assert_invalid_system_transaction(tx: TxEnv, expected: &str) {
        let err = protocol_system_call(&tx).unwrap_err();
        assert!(err.to_string().contains(expected), "expected {expected:?} in error, got {err:?}");
    }

    #[test]
    fn monad_evm_factory_implements_foundry_evm_factory() {
        fn assert_foundry_factory<F: FoundryEvmFactory>() {}

        assert_foundry_factory::<MonadEvmFactory>();
    }

    #[test]
    fn monad_context_transition_rebases_live_tracker() {
        let sender = Address::with_last_byte(1);
        let old_chain = MonadChainContext::default();
        let new_chain = MonadChainContext {
            parent_senders_and_authorities: [sender].into_iter().collect(),
            ..Default::default()
        };
        let mut account =
            Account::from(AccountInfo { balance: U256::from(12), ..Default::default() });
        account.info.balance = U256::from(9);

        let factory = MonadEvmFactory::default();
        let mut evm = factory.create_evm(
            revm::database::EmptyDB::default(),
            EvmEnv::new(
                revm::context::CfgEnv::new_with_spec(MonadHardfork::MonadNine),
                BlockEnv::default(),
            ),
        );
        evm.ctx_mut().chain = old_chain.clone();
        evm.ctx_mut().journaled_state.reserve_balance_mut().init(ReserveBalanceInit {
            chain: &old_chain,
            spec: MonadHardfork::MonadNine,
            sender,
            effective_gas_price: 0,
            gas_limit: 0,
            sender_is_delegated: false,
            sender_account: Some(&account),
        });
        assert!(!evm.ctx().journaled_state.reserve_balance().has_violation());

        evm.ctx_mut().chain = new_chain.clone();
        evm.ctx_mut().journaled_state.inner.state = EvmState::from_iter([(sender, account)]);
        crate::refresh_chain_journal(evm.ctx_mut());

        assert_eq!(evm.ctx().chain, new_chain);
        assert!(evm.ctx().journaled_state.reserve_balance().has_violation());
    }

    #[test]
    fn monad_factory_classifies_canonical_system_envelopes() {
        let reward = U256::from(25);
        let reward_tx = system_transaction(
            syscallRewardCall { blockAuthor: Address::with_last_byte(1) }.abi_encode(),
            reward,
        );
        let reward_call = protocol_system_call(&reward_tx).unwrap().unwrap();
        assert_eq!(reward_call.data.len(), 68);
        assert_eq!(reward_call.balance_increment, Some((STAKING_ADDRESS, reward)));

        let snapshot_tx = system_transaction(syscallSnapshotCall {}.abi_encode(), U256::ZERO);
        assert!(protocol_system_call(&snapshot_tx).unwrap().is_some());

        let epoch_tx =
            system_transaction(syscallOnEpochChangeCall { epoch: 9 }.abi_encode(), U256::ZERO);
        assert!(protocol_system_call(&epoch_tx).unwrap().is_some());

        let mut unrelated = snapshot_tx;
        unrelated.caller = Address::with_last_byte(2);
        unrelated.tx_type = TransactionType::Eip1559 as u8;
        assert!(protocol_system_call(&unrelated).unwrap().is_none());
    }

    #[test]
    fn monad_replay_decline_leaves_evm_untouched() {
        let tx = TxEnv {
            caller: foundry_common::OPTIMISM_SYSTEM_ADDRESS,
            kind: TxKind::Call(Address::with_last_byte(1)),
            ..Default::default()
        };
        let factory = MonadEvmFactory::default();
        let evm_env =
            EvmEnv::new(CfgEnv::new_with_spec(MonadHardfork::MonadNine), BlockEnv::default());
        let mut evm = factory.create_evm(InMemoryDB::default(), evm_env);
        let tx_before = evm.tx().clone();
        let journal_before = evm.ctx().journal_inner().clone();
        let chain_before = evm.ctx().chain.clone();
        let tracker_before = evm.ctx().journaled_state.reserve_balance().clone();

        assert!(factory.try_transact_system_replay(&mut evm, &tx).unwrap().is_none());
        assert_eq!(evm.tx(), &tx_before);
        assert_eq!(evm.ctx().journal_inner().state, journal_before.state);
        assert_eq!(evm.ctx().chain, chain_before);
        assert_eq!(evm.ctx().journaled_state.reserve_balance(), &tracker_before);
    }

    #[test]
    fn monad_factory_rejects_noncanonical_system_envelope_fields() {
        let canonical = system_transaction(syscallSnapshotCall {}.abi_encode(), U256::ZERO);

        let mut tx = canonical.clone();
        tx.tx_type = TransactionType::Eip1559 as u8;
        assert_invalid_system_transaction(tx, "transaction type must be legacy");

        let mut tx = canonical.clone();
        tx.kind = revm::primitives::TxKind::Call(Address::ZERO);
        assert_invalid_system_transaction(tx, "target must be the staking contract");

        let mut tx = canonical.clone();
        tx.gas_limit = 1;
        assert_invalid_system_transaction(tx, "gas limit must be zero");

        let mut tx = canonical.clone();
        tx.gas_price = 1;
        assert_invalid_system_transaction(tx, "gas price must be zero");

        let mut tx = canonical.clone();
        tx.gas_priority_fee = Some(0);
        assert_invalid_system_transaction(tx, "priority fee must be absent");

        let mut tx = canonical.clone();
        tx.access_list.0.push(AccessListItem::default());
        assert_invalid_system_transaction(tx, "access list must be empty");

        let mut tx = canonical.clone();
        tx.blob_hashes.push(B256::ZERO);
        assert_invalid_system_transaction(tx, "blob hashes must be empty");

        let mut tx = canonical.clone();
        tx.max_fee_per_blob_gas = 1;
        assert_invalid_system_transaction(tx, "blob gas fee must be zero");

        let mut tx = canonical;
        tx.authorization_list =
            transaction(Address::ZERO, Address::with_last_byte(1)).authorization_list;
        assert_invalid_system_transaction(tx, "authorization list must be empty");
    }

    #[test]
    fn monad_factory_rejects_noncanonical_system_call_data_and_value() {
        assert_invalid_system_transaction(
            system_transaction(Vec::new(), U256::ZERO),
            "calldata is shorter than a selector",
        );
        assert_invalid_system_transaction(
            system_transaction(vec![0xff; 4], U256::ZERO),
            "unknown staking syscall selector",
        );

        let mut reward = syscallRewardCall { blockAuthor: Address::with_last_byte(1) }.abi_encode();
        reward.push(0);
        assert_invalid_system_transaction(
            system_transaction(reward, U256::ZERO),
            "reward calldata must be 36 bytes",
        );

        let mut malformed_reward =
            syscallRewardCall { blockAuthor: Address::with_last_byte(1) }.abi_encode();
        malformed_reward[4] = 1;
        assert_invalid_system_transaction(
            system_transaction(malformed_reward, U256::ZERO),
            "invalid Monad protocol system reward calldata",
        );

        let mut snapshot = syscallSnapshotCall {}.abi_encode();
        snapshot.push(0);
        assert_invalid_system_transaction(
            system_transaction(snapshot, U256::ZERO),
            "snapshot calldata must be 4 bytes",
        );
        assert_invalid_system_transaction(
            system_transaction(syscallSnapshotCall {}.abi_encode(), U256::from(1)),
            "snapshot value must be zero",
        );

        let mut epoch = syscallOnEpochChangeCall { epoch: 9 }.abi_encode();
        epoch.push(0);
        assert_invalid_system_transaction(
            system_transaction(epoch, U256::ZERO),
            "epoch calldata must be 36 bytes",
        );
        let mut malformed_epoch = syscallOnEpochChangeCall { epoch: 9 }.abi_encode();
        malformed_epoch[4] = 1;
        assert_invalid_system_transaction(
            system_transaction(malformed_epoch, U256::ZERO),
            "invalid Monad protocol system epoch calldata",
        );
        assert_invalid_system_transaction(
            system_transaction(syscallOnEpochChangeCall { epoch: 9 }.abi_encode(), U256::from(1)),
            "epoch value must be zero",
        );
    }

    #[test]
    fn monad_factory_validates_system_envelope_chain_id_at_execution() {
        let mut tx = system_transaction(syscallSnapshotCall {}.abi_encode(), U256::ZERO);
        tx.chain_id = Some(143);
        let system_call = protocol_system_call(&tx).unwrap().unwrap();

        system_call.validate_chain_id(143).unwrap();
        assert!(
            system_call.validate_chain_id(1).unwrap_err().to_string().contains("chain ID mismatch")
        );
    }

    #[test]
    fn protocol_prestate_updates_nonce_and_balance() {
        let caller = address!("00000000000000000000000000000000000000fe");
        let recipient = address!("0000000000000000000000000000000000001000");
        let mut db = InMemoryDB::default();
        db.insert_account_info(caller, AccountInfo { nonce: 7, ..Default::default() });
        db.insert_account_info(
            recipient,
            AccountInfo { balance: U256::from(10), ..Default::default() },
        );
        let call = ProtocolSystemCall {
            caller,
            contract: recipient,
            data: Bytes::new(),
            nonce: 7,
            chain_id: None,
            balance_increment: Some((recipient, U256::from(25))),
        };
        let mut journal = JournaledState::default();

        call.apply_prestate(&mut db, &mut journal).unwrap();

        assert_eq!(journal.state[&caller].info.nonce, 8);
        assert_eq!(journal.state[&recipient].info.balance, U256::from(35));
        assert_eq!(db.basic(caller).unwrap().unwrap().nonce, 7);
        assert_eq!(db.basic(recipient).unwrap().unwrap().balance, U256::from(10));
    }

    #[test]
    fn protocol_prestate_rejects_nonce_mismatch() {
        let caller = address!("00000000000000000000000000000000000000fe");
        let mut db = InMemoryDB::default();
        db.insert_account_info(caller, AccountInfo { nonce: 3, ..Default::default() });
        let call = ProtocolSystemCall {
            caller,
            contract: Address::ZERO,
            data: Bytes::new(),
            nonce: 4,
            chain_id: None,
            balance_increment: None,
        };
        let mut journal = JournaledState::default();

        let err = call.apply_prestate(&mut db, &mut journal).unwrap_err();

        assert!(err.to_string().contains("nonce mismatch"));
        assert_eq!(db.basic(caller).unwrap().unwrap().nonce, 3);
    }

    #[test]
    fn protocol_prestate_rejects_nonce_overflow() {
        let caller = address!("00000000000000000000000000000000000000fe");
        let mut db = InMemoryDB::default();
        db.insert_account_info(caller, AccountInfo { nonce: u64::MAX, ..Default::default() });
        let call = ProtocolSystemCall {
            caller,
            contract: Address::ZERO,
            data: Bytes::new(),
            nonce: u64::MAX,
            chain_id: None,
            balance_increment: None,
        };
        let mut journal = JournaledState::default();

        let err = call.apply_prestate(&mut db, &mut journal).unwrap_err();

        assert!(err.to_string().contains("nonce overflow"));
        assert_eq!(db.basic(caller).unwrap().unwrap().nonce, u64::MAX);
    }

    #[test]
    fn reward_envelope_replays_mint_nonce_storage_and_log() {
        let block_author = address!("1111111111111111111111111111111111111111");
        let validator_auth = address!("2222222222222222222222222222222222222222");
        let validator_id = 7;
        let reward = U256::from(25) * MON;
        let initial_staking_balance = U256::from(3) * MON;
        // Monad stores validator IDs and packed address/flags values left-aligned.
        let validator_id_slot = U256::from(validator_id) << 192;
        let address_flags_slot = U256::from_be_slice(validator_auth.as_slice()) << 96;
        let mut db = InMemoryDB::default();
        db.insert_account_info(SYSTEM_ADDRESS, AccountInfo { nonce: 11, ..Default::default() });
        db.insert_account_info(
            STAKING_ADDRESS,
            AccountInfo { balance: initial_staking_balance, ..Default::default() },
        );
        db.insert_account_storage(
            STAKING_ADDRESS,
            val_id_secp_key(&block_author),
            validator_id_slot,
        )
        .unwrap();
        db.insert_account_storage(
            STAKING_ADDRESS,
            consensus_view_key(validator_id, 0),
            U256::from(100) * MON,
        )
        .unwrap();
        db.insert_account_storage(STAKING_ADDRESS, consensus_view_key(validator_id, 1), U256::ZERO)
            .unwrap();
        db.insert_account_storage(
            STAKING_ADDRESS,
            validator_key(validator_id, validator_offsets::ADDRESS_FLAGS),
            address_flags_slot,
        )
        .unwrap();

        let tx = TxEnv {
            tx_type: 0,
            caller: SYSTEM_ADDRESS,
            gas_limit: 0,
            kind: TxKind::Call(STAKING_ADDRESS),
            value: reward,
            data: syscallRewardCall { blockAuthor: block_author }.abi_encode().into(),
            nonce: 11,
            ..Default::default()
        };
        let factory = MonadEvmFactory::default();
        let evm_env =
            EvmEnv::new(CfgEnv::new_with_spec(MonadHardfork::MonadNine), BlockEnv::default());
        let mut evm =
            factory.create_evm_with_inspector(db, evm_env, ProtocolPrestateInspector::default());

        let result = factory.try_transact_system_replay(&mut evm, &tx).unwrap().unwrap();

        assert!(result.result.is_success());
        assert_eq!(result.result.tx_gas_used(), 0);
        assert!(evm.inspector().call_count > 0);
        assert_eq!(evm.inspector().staking_balance, Some(initial_staking_balance + reward));
        assert_eq!(result.result.logs().len(), 1);
        assert_eq!(result.result.logs()[0].address, STAKING_ADDRESS);
        assert_eq!(result.result.logs()[0].topics()[0], ValidatorRewarded::SIGNATURE_HASH);
        evm.db_mut().commit(result.state);
        let mut db = evm.into_db();
        assert_eq!(db.basic(SYSTEM_ADDRESS).unwrap().unwrap().nonce, 12);
        assert_eq!(
            db.basic(STAKING_ADDRESS).unwrap().unwrap().balance,
            initial_staking_balance + reward
        );
        assert_eq!(
            db.storage(STAKING_ADDRESS, global_slots::PROPOSER_VAL_ID).unwrap(),
            validator_id_slot
        );
        assert_eq!(
            db.storage(
                STAKING_ADDRESS,
                validator_key(validator_id, validator_offsets::UNCLAIMED_REWARDS),
            )
            .unwrap(),
            reward
        );
    }

    #[test]
    fn failed_reward_envelope_does_not_commit_prestate() {
        let unknown_author = address!("1111111111111111111111111111111111111111");
        let reward = U256::from(25) * MON;
        let initial_staking_balance = U256::from(3) * MON;
        let mut db = InMemoryDB::default();
        db.insert_account_info(SYSTEM_ADDRESS, AccountInfo { nonce: 11, ..Default::default() });
        db.insert_account_info(
            STAKING_ADDRESS,
            AccountInfo { balance: initial_staking_balance, ..Default::default() },
        );
        let tx = TxEnv {
            tx_type: 0,
            caller: SYSTEM_ADDRESS,
            gas_limit: 0,
            kind: TxKind::Call(STAKING_ADDRESS),
            value: reward,
            data: syscallRewardCall { blockAuthor: unknown_author }.abi_encode().into(),
            nonce: 11,
            ..Default::default()
        };
        let factory = MonadEvmFactory::default();
        let evm_env =
            EvmEnv::new(CfgEnv::new_with_spec(MonadHardfork::MonadNine), BlockEnv::default());
        let mut evm =
            factory.create_evm_with_inspector(db, evm_env, ProtocolPrestateInspector::default());
        let journal_before = evm.ctx().journal_inner().clone();
        let chain_before = evm.ctx().chain.clone();
        let tracker_before = evm.ctx().journaled_state.reserve_balance().clone();

        let error = factory.try_transact_system_replay(&mut evm, &tx).unwrap_err();

        assert!(error.to_string().contains("reverted or halted"));
        assert!(evm.inspector().call_count > 0);
        assert_eq!(evm.inspector().staking_balance, Some(initial_staking_balance + reward));
        assert_eq!(evm.ctx().journal_inner().state, journal_before.state);
        assert_eq!(evm.ctx().chain, chain_before);
        assert_eq!(evm.ctx().journaled_state.reserve_balance(), &tracker_before);
        assert_eq!(evm.db_mut().basic(SYSTEM_ADDRESS).unwrap().unwrap().nonce, 11);
        assert_eq!(
            evm.db_mut().basic(STAKING_ADDRESS).unwrap().unwrap().balance,
            initial_staking_balance
        );
        assert_eq!(
            evm.db_mut().storage(STAKING_ADDRESS, global_slots::PROPOSER_VAL_ID).unwrap(),
            U256::ZERO
        );
    }

    #[test]
    fn monad_context_tracks_senders_authorities_and_current_index() {
        let grandparent_sender = Address::from([1; 20]);
        let grandparent_authority = Address::from([2; 20]);
        let parent_sender = Address::from([3; 20]);
        let parent_authority = Address::from([4; 20]);
        let current_sender = Address::from([5; 20]);
        let current_authority = Address::from([6; 20]);
        let next_sender = Address::from([7; 20]);
        let next_authority = Address::from([8; 20]);

        let grandparent = [transaction(grandparent_sender, grandparent_authority)];
        let parent = [transaction(parent_sender, parent_authority)];
        let current = [
            transaction(current_sender, current_authority),
            transaction(next_sender, next_authority),
        ];

        let context = monad_context_from_participants(
            monad_block_participants(&grandparent),
            monad_block_participants(&parent),
            &current,
            1,
        );

        assert_eq!(context.current_tx_index, 1);
        assert_eq!(context.grandparent_senders_and_authorities.len(), 2);
        assert!(context.grandparent_senders_and_authorities.contains(&grandparent_sender));
        assert!(context.grandparent_senders_and_authorities.contains(&grandparent_authority));
        assert_eq!(context.parent_senders_and_authorities.len(), 2);
        assert!(context.parent_senders_and_authorities.contains(&parent_sender));
        assert!(context.parent_senders_and_authorities.contains(&parent_authority));
        assert_eq!(context.current_block_senders, vec![current_sender, next_sender]);
        assert_eq!(context.current_block_authorities.len(), 2);
        assert!(context.current_block_authorities[0].contains(&current_authority));
        assert!(context.current_block_authorities[1].contains(&next_authority));
    }

    #[test]
    fn child_context_advances_fork_ancestry() {
        let parent_sender = Address::from([1; 20]);
        let parent_authority = Address::from([2; 20]);
        let current_sender = Address::from([3; 20]);
        let current_authority = Address::from([4; 20]);
        let child_sender = Address::from([5; 20]);
        let child_authority = Address::from([6; 20]);

        let context = BlockContext::<MonadEvmNetwork>::new(
            Vec::new(),
            vec![transaction(parent_sender, parent_authority)],
            vec![transaction(current_sender, current_authority)],
        )
        .into_child()
        .next_transaction(&transaction(child_sender, child_authority));

        assert_eq!(context.current_tx_index, 0);
        assert_eq!(context.grandparent_senders_and_authorities.len(), 2);
        assert!(context.grandparent_senders_and_authorities.contains(&parent_sender));
        assert!(context.grandparent_senders_and_authorities.contains(&parent_authority));
        assert_eq!(context.parent_senders_and_authorities.len(), 2);
        assert!(context.parent_senders_and_authorities.contains(&current_sender));
        assert!(context.parent_senders_and_authorities.contains(&current_authority));
        assert_eq!(context.current_block_senders, vec![child_sender]);
        assert!(context.current_block_authorities[0].contains(&child_authority));
    }

    #[test]
    fn transaction_cursor_replaces_target_and_excludes_future_transactions() {
        let preceding_sender = Address::from([1; 20]);
        let target_sender = Address::from([2; 20]);
        let future_sender = Address::from([3; 20]);
        let synthetic_sender = Address::from([4; 20]);

        let cursor = BlockContext::<MonadEvmNetwork>::new(
            Vec::new(),
            Vec::new(),
            vec![
                transaction(preceding_sender, Address::ZERO),
                transaction(target_sender, Address::ZERO),
                transaction(future_sender, Address::ZERO),
            ],
        )
        .before_transaction(1)
        .unwrap();
        let context = cursor.next_transaction(&transaction(synthetic_sender, Address::ZERO));

        assert_eq!(context.current_tx_index, 1);
        assert_eq!(context.current_block_senders, vec![preceding_sender, synthetic_sender]);
        assert!(!context.current_block_senders.contains(&target_sender));
        assert!(!context.current_block_senders.contains(&future_sender));
    }

    #[test]
    fn transaction_cursor_accumulates_same_block_transactions() {
        let fork_sender = Address::from([1; 20]);
        let first_sender = Address::from([2; 20]);
        let second_sender = Address::from([3; 20]);
        let mut cursor = BlockContext::<MonadEvmNetwork>::new(
            Vec::new(),
            Vec::new(),
            vec![transaction(fork_sender, Address::ZERO)],
        )
        .into_child();

        cursor.record_transaction(transaction(first_sender, Address::ZERO));
        let context = cursor.next_transaction(&transaction(second_sender, Address::ZERO));

        assert_eq!(context.current_tx_index, 1);
        assert_eq!(context.current_block_senders, vec![first_sender, second_sender]);
        assert!(context.parent_senders_and_authorities.contains(&fork_sender));
    }

    #[test]
    fn transaction_cursor_rotates_separate_blocks() {
        let fork_parent_sender = Address::from([1; 20]);
        let fork_sender = Address::from([2; 20]);
        let first_sender = Address::from([3; 20]);
        let second_sender = Address::from([4; 20]);
        let mut cursor = BlockContext::<MonadEvmNetwork>::new(
            Vec::new(),
            vec![transaction(fork_parent_sender, Address::ZERO)],
            vec![transaction(fork_sender, Address::ZERO)],
        )
        .into_child();

        cursor.record_transaction(transaction(first_sender, Address::ZERO));
        cursor.advance_block();
        let context = cursor.next_transaction(&transaction(second_sender, Address::ZERO));

        assert_eq!(context.current_tx_index, 0);
        assert_eq!(context.current_block_senders, vec![second_sender]);
        assert!(context.parent_senders_and_authorities.contains(&first_sender));
        assert!(context.grandparent_senders_and_authorities.contains(&fork_sender));
        assert!(!context.grandparent_senders_and_authorities.contains(&fork_parent_sender));
    }
}
