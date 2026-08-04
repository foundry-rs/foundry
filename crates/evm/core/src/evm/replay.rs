use alloy_primitives::{Address, Bytes, U256};
use eyre::Result;
#[cfg(any(feature = "monad", test))]
use revm::context::journaled_state::account::JournaledAccountTr;
use revm::{context_interface::result::ResultAndState, inspector::Inspector};

use super::FoundryEvmFactory;
#[cfg(any(feature = "monad", test))]
use crate::backend::JournaledState;

/// A protocol system transaction that must bypass ordinary transaction validation.
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
    #[cfg(feature = "monad")]
    pub(crate) fn validate_chain_id(&self, chain_id: u64) -> Result<()> {
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

    #[cfg(any(feature = "monad", test))]
    pub(crate) fn apply_prestate<DB: alloy_evm::Database>(
        &self,
        db: &mut DB,
        journal: &mut JournaledState,
    ) -> Result<()> {
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

/// Executes one canonical replay transaction, including family-specific protocol system calls.
pub fn execute_replay_transaction<F, DB, I>(
    factory: &F,
    evm: &mut F::Evm<DB, I>,
    tx: F::Tx,
) -> Result<ResultAndState<F::HaltReason>>
where
    F: FoundryEvmFactory,
    DB: alloy_evm::Database,
    I: Inspector<F::Context<DB>>,
{
    factory.transact_replay(evm, tx)
}

#[cfg(feature = "monad")]
pub(crate) fn finish_protocol_system_call<H>(
    mut result: ResultAndState<H>,
) -> Result<ResultAndState<H>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use revm::{Database, database::InMemoryDB, primitives::address, state::AccountInfo};

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
}

#[cfg(all(test, feature = "monad"))]
mod monad_tests {
    use super::*;
    use crate::FoundryContextExt;
    use alloy_evm::{Evm, EvmEnv, EvmFactory};
    use alloy_monad_evm::MonadEvmFactory;
    use alloy_sol_types::{SolCall, SolEvent};
    use monad_revm::{
        MonadContext, MonadHardfork,
        staking::{
            STAKING_ADDRESS,
            constants::{MON, SYSTEM_ADDRESS},
            interface::IMonadStaking::{ValidatorRewarded, syscallRewardCall},
            storage::{
                consensus_view_key, global_slots, val_id_secp_key, validator_key, validator_offsets,
            },
        },
    };
    use revm::{
        Database, DatabaseCommit,
        context::{BlockEnv, CfgEnv, TxEnv},
        database::InMemoryDB,
        interpreter::{CallInputs, CallOutcome},
        primitives::{TxKind, address},
        state::AccountInfo,
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

    fn left_aligned_u64(value: u64) -> U256 {
        let mut bytes = [0; 32];
        bytes[..8].copy_from_slice(&value.to_be_bytes());
        U256::from_be_bytes(bytes)
    }

    fn address_and_flags(address: Address, flags: u64) -> U256 {
        let mut bytes = [0; 32];
        bytes[..20].copy_from_slice(address.as_slice());
        bytes[20..28].copy_from_slice(&flags.to_be_bytes());
        U256::from_be_bytes(bytes)
    }

    #[test]
    fn reward_envelope_replays_mint_nonce_storage_and_log() {
        let block_author = address!("1111111111111111111111111111111111111111");
        let validator_auth = address!("2222222222222222222222222222222222222222");
        let validator_id = 7;
        let reward = U256::from(25) * MON;
        let initial_staking_balance = U256::from(3) * MON;
        let mut db = InMemoryDB::default();
        db.insert_account_info(SYSTEM_ADDRESS, AccountInfo { nonce: 11, ..Default::default() });
        db.insert_account_info(
            STAKING_ADDRESS,
            AccountInfo { balance: initial_staking_balance, ..Default::default() },
        );
        db.insert_account_storage(
            STAKING_ADDRESS,
            val_id_secp_key(&block_author),
            left_aligned_u64(validator_id),
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
            address_and_flags(validator_auth, 0),
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

        let result = execute_replay_transaction(&factory, &mut evm, tx).unwrap();

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
            left_aligned_u64(validator_id)
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
        let context_before = evm.context_state();

        let error = execute_replay_transaction(&factory, &mut evm, tx).unwrap_err();

        assert!(error.to_string().contains("reverted or halted"));
        assert!(evm.inspector().call_count > 0);
        assert_eq!(evm.inspector().staking_balance, Some(initial_staking_balance + reward));
        let context_after = evm.context_state();
        assert_eq!(context_after.journaled_state.state, context_before.journaled_state.state);
        assert_eq!(context_after.auxiliary, context_before.auxiliary);
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
}
