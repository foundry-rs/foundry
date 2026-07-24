use alloy_evm::{Evm, EvmEnv, EvmFactory};
use alloy_monad_evm::{MonadEvm, MonadEvmFactory, MonadPrecompilesMap};
use alloy_sol_types::SolCall;
use eyre::WrapErr;
use foundry_fork_db::DatabaseError;
use monad_revm::{
    MonadBuilder, MonadCfgEnv, MonadContext, MonadEvm as RevmMonadEvm, MonadHardfork,
    MonadJournalTr,
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
        BlockEnv, ContextTr, LocalContextTr, Transaction, TransactionType, TxEnv,
        result::{EVMError, ResultAndState},
    },
    context_interface::{Cfg, ContextSetters, transaction::AuthorizationTr},
    handler::{EthFrame, EvmTr, FrameResult, Handler},
    inspector::{InspectSystemCallEvm, InspectorHandler},
    interpreter::{FrameInput, SharedMemory, interpreter_action::FrameInit},
    primitives::{Address, HashSet},
};

use crate::{
    FoundryContextExt, FoundryContextState, FoundryInspectorExt, MonadContextAux,
    backend::{DatabaseExt, JournaledState},
    evm::{FoundryEvmFactory, NestedEvm, ProtocolSystemCall, finish_protocol_system_call},
};

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
) -> MonadContextAux {
    MonadContextAux {
        chain: monad_revm::MonadChainContext {
            grandparent_senders_and_authorities,
            parent_senders_and_authorities,
            current_block_senders: current.iter().map(Transaction::caller).collect(),
            current_block_authorities: current
                .iter()
                .map(|tx| tx.authorization_list().filter_map(|auth| auth.authority()).collect())
                .collect(),
            current_tx_index,
            ..Default::default()
        },
        ..Default::default()
    }
}

impl FoundryEvmFactory for MonadEvmFactory {
    const NEEDS_BLOCK_CONTEXT: bool = true;
    const USES_EIP4788_BEACON_ROOTS: bool = false;

    type ContextAux = MonadContextAux;
    type FoundryContext<'db> = MonadContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> =
        MonadEvm<&'db mut dyn DatabaseExt<Self>, I>;

    fn create_evm_with_context<DB: alloy_evm::Database>(
        &self,
        db: DB,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        context_aux: Self::ContextAux,
    ) -> Self::Evm<DB, revm::inspector::NoOpInspector> {
        let mut evm = self.create_evm(db, evm_env);
        evm.ctx_mut().set_aux_state(context_aux);
        evm
    }

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        context_aux: Self::ContextAux,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let mut monad_evm = self.create_evm_with_inspector(db, evm_env, inspector);
        monad_evm.ctx_mut().set_aux_state(context_aux);
        monad_evm.cfg.tx_chain_id_check = true;
        monad_evm.inspector().get_networks().inject_precompiles(monad_evm.precompiles_mut());
        monad_evm
    }

    fn context_for_transaction(&self, tx: &Self::Tx) -> Self::ContextAux {
        self.context_for_block(&[], &[], std::slice::from_ref(tx), 0)
    }

    fn context_for_block(
        &self,
        grandparent: &[Self::Tx],
        parent: &[Self::Tx],
        current: &[Self::Tx],
        current_tx_index: usize,
    ) -> Self::ContextAux {
        monad_context_from_participants(
            monad_block_participants(grandparent),
            monad_block_participants(parent),
            current,
            current_tx_index,
        )
    }

    fn protocol_system_call(&self, tx: &Self::Tx) -> eyre::Result<Option<ProtocolSystemCall>> {
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
            tx.gas_priority_fee.is_none(),
            "invalid Monad protocol system transaction: priority fee must be absent"
        );
        eyre::ensure!(
            tx.access_list.0.is_empty(),
            "invalid Monad protocol system transaction: access list must be empty"
        );
        eyre::ensure!(
            tx.blob_hashes.is_empty(),
            "invalid Monad protocol system transaction: blob hashes must be empty"
        );
        eyre::ensure!(
            tx.max_fee_per_blob_gas == 0,
            "invalid Monad protocol system transaction: blob gas fee must be zero"
        );
        eyre::ensure!(
            tx.authorization_list.is_empty(),
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
                eyre::bail!(
                    "invalid Monad protocol system transaction: unknown staking syscall selector"
                )
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

    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        context_aux: Self::ContextAux,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> Box<
        dyn NestedEvm<Spec = MonadHardfork, Block = BlockEnv, Tx = TxEnv, Aux = MonadContextAux>
            + 'db,
    > {
        let spec = evm_env.cfg_env.spec;
        let monad_cfg = MonadCfgEnv::from(evm_env.cfg_env);
        let mut evm = monad_context_with_db(db)
            .with_block(evm_env.block_env)
            .with_cfg(monad_cfg)
            .build_monad_with_inspector(inspector)
            .with_precompiles(MonadPrecompilesMap::new_with_spec(spec));

        evm.0.ctx.set_aux_state(context_aux);
        evm.0.ctx.cfg.tx_chain_id_check = true;
        evm.0.inspector.get_networks().inject_precompiles(&mut evm.0.precompiles);

        Box::new(evm)
    }
}

impl<'db, I: FoundryInspectorExt<MonadContext<&'db mut dyn DatabaseExt<MonadEvmFactory>>>> NestedEvm
    for MonadRevmEvm<'db, I>
{
    type Spec = MonadHardfork;
    type Block = BlockEnv;
    type Tx = TxEnv;
    type Aux = MonadContextAux;

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx_mut().journaled_state.inner
    }

    fn context_state(&self) -> FoundryContextState<Self::Aux> {
        self.ctx_ref().context_state()
    }

    fn aux_state(&self) -> Self::Aux {
        self.ctx_ref().aux_state()
    }

    fn set_context_state(&mut self, state: FoundryContextState<Self::Aux>) {
        self.ctx_mut().set_context_state(state);
    }

    fn preserve_aux_state_on_transaction(&mut self) {
        self.ctx_mut().journaled_state.set_preserve_reserve_balance_tracker(true);
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        let mut handler = MonadEvmHandler::<I>::new();
        let reservoir = frame.reservoir();

        let memory =
            SharedMemory::new_with_buffer(self.ctx_ref().local().shared_memory_buffer().clone());
        let first_frame_input = FrameInit { depth: 0, memory, frame_input: frame };

        let mut frame_result = handler.inspect_run_exec_loop(self, first_frame_input)?;

        handler.last_frame_result(self, reservoir, &mut frame_result)?;

        Ok(frame_result)
    }

    fn transact_raw(&mut self, tx: Self::Tx) -> Result<ResultAndState, EVMError<DatabaseError>> {
        ContextSetters::set_tx(&mut self.0.ctx, tx);

        let mut handler = MonadEvmHandler::<I>::new();
        let result = handler.inspect_run(self)?;

        Ok(ResultAndState::new(result, self.ctx_ref().journaled_state.inner.state.clone()))
    }

    fn transact_replay(&mut self, tx: Self::Tx) -> eyre::Result<ResultAndState> {
        let factory = MonadEvmFactory::default();
        let Some(system_call) = factory.protocol_system_call(&tx)? else {
            return self.transact_raw(tx).map_err(Into::into);
        };

        system_call.validate_chain_id(self.ctx_ref().cfg().chain_id())?;
        let prestate = system_call.apply_prestate(self.0.ctx.db_mut())?;
        let result = self
            .inspect_system_call_with_caller(
                system_call.caller,
                system_call.contract,
                system_call.data,
            )
            .wrap_err("failed to execute protocol system transaction")?;
        finish_protocol_system_call(result, prestate)
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block> {
        self.ctx_ref().evm_clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evm::{BlockContext, MonadEvmNetwork};
    use revm::{
        context_interface::{
            either::Either,
            transaction::{
                AccessListItem, Authorization, RecoveredAuthority, RecoveredAuthorization,
            },
        },
        primitives::{B256, U256},
    };

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
        let err = MonadEvmFactory::default().protocol_system_call(&tx).unwrap_err();
        assert!(err.to_string().contains(expected), "expected {expected:?} in error, got {err:?}");
    }

    #[test]
    fn monad_evm_factory_implements_foundry_evm_factory() {
        fn assert_foundry_factory<F: FoundryEvmFactory>() {}

        assert_foundry_factory::<MonadEvmFactory>();
    }

    #[test]
    fn monad_factory_classifies_canonical_system_envelopes() {
        let factory = MonadEvmFactory::default();
        let reward = U256::from(25);
        let reward_tx = system_transaction(
            syscallRewardCall { blockAuthor: Address::with_last_byte(1) }.abi_encode(),
            reward,
        );
        let reward_call = factory.protocol_system_call(&reward_tx).unwrap().unwrap();
        assert_eq!(reward_call.data.len(), 68);
        assert_eq!(reward_call.balance_increment, Some((STAKING_ADDRESS, reward)));

        let snapshot_tx = system_transaction(syscallSnapshotCall {}.abi_encode(), U256::ZERO);
        assert!(factory.protocol_system_call(&snapshot_tx).unwrap().is_some());

        let epoch_tx =
            system_transaction(syscallOnEpochChangeCall { epoch: 9 }.abi_encode(), U256::ZERO);
        assert!(factory.protocol_system_call(&epoch_tx).unwrap().is_some());

        let mut unrelated = snapshot_tx;
        unrelated.caller = Address::with_last_byte(2);
        unrelated.tx_type = TransactionType::Eip1559 as u8;
        assert!(factory.protocol_system_call(&unrelated).unwrap().is_none());
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
        let system_call = MonadEvmFactory::default().protocol_system_call(&tx).unwrap().unwrap();

        system_call.validate_chain_id(143).unwrap();
        assert!(
            system_call.validate_chain_id(1).unwrap_err().to_string().contains("chain ID mismatch")
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

        assert_eq!(context.chain.current_tx_index, 1);
        assert_eq!(context.chain.grandparent_senders_and_authorities.len(), 2);
        assert!(context.chain.grandparent_senders_and_authorities.contains(&grandparent_sender));
        assert!(context.chain.grandparent_senders_and_authorities.contains(&grandparent_authority));
        assert_eq!(context.chain.parent_senders_and_authorities.len(), 2);
        assert!(context.chain.parent_senders_and_authorities.contains(&parent_sender));
        assert!(context.chain.parent_senders_and_authorities.contains(&parent_authority));
        assert_eq!(context.chain.current_block_senders, vec![current_sender, next_sender]);
        assert_eq!(context.chain.current_block_authorities.len(), 2);
        assert!(context.chain.current_block_authorities[0].contains(&current_authority));
        assert!(context.chain.current_block_authorities[1].contains(&next_authority));
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
        .child(&transaction(child_sender, child_authority));

        assert_eq!(context.chain.current_tx_index, 0);
        assert_eq!(context.chain.grandparent_senders_and_authorities.len(), 2);
        assert!(context.chain.grandparent_senders_and_authorities.contains(&parent_sender));
        assert!(context.chain.grandparent_senders_and_authorities.contains(&parent_authority));
        assert_eq!(context.chain.parent_senders_and_authorities.len(), 2);
        assert!(context.chain.parent_senders_and_authorities.contains(&current_sender));
        assert!(context.chain.parent_senders_and_authorities.contains(&current_authority));
        assert_eq!(context.chain.current_block_senders, vec![child_sender]);
        assert!(context.chain.current_block_authorities[0].contains(&child_authority));
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

        assert_eq!(context.chain.current_tx_index, 1);
        assert_eq!(context.chain.current_block_senders, vec![preceding_sender, synthetic_sender]);
        assert!(!context.chain.current_block_senders.contains(&target_sender));
        assert!(!context.chain.current_block_senders.contains(&future_sender));
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

        assert_eq!(context.chain.current_tx_index, 1);
        assert_eq!(context.chain.current_block_senders, vec![first_sender, second_sender]);
        assert!(context.chain.parent_senders_and_authorities.contains(&fork_sender));
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

        assert_eq!(context.chain.current_tx_index, 0);
        assert_eq!(context.chain.current_block_senders, vec![second_sender]);
        assert!(context.chain.parent_senders_and_authorities.contains(&first_sender));
        assert!(context.chain.grandparent_senders_and_authorities.contains(&fork_sender));
        assert!(!context.chain.grandparent_senders_and_authorities.contains(&fork_parent_sender));
    }
}
