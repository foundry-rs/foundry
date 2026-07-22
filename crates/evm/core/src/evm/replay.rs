use alloy_evm::Evm;
use alloy_primitives::{
    Address, Bytes, U256,
    map::{AddressMap, Entry},
};
use eyre::{Result, WrapErr};
use revm::{
    Database, DatabaseCommit,
    context_interface::result::ResultAndState,
    state::{Account, EvmState},
};

use super::FoundryEvmFactory;

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
    /// Optional protocol mint applied before system-call execution.
    pub balance_increment: Option<(Address, U256)>,
}

impl ProtocolSystemCall {
    pub(crate) fn apply_prestate<DB: alloy_evm::Database + DatabaseCommit>(
        &self,
        db: &mut DB,
    ) -> Result<AddressMap<Account>> {
        let mut changes = AddressMap::default();
        let caller = load_account(db, &mut changes, self.caller)?;
        if caller.info.nonce != self.nonce {
            eyre::bail!(
                "protocol system transaction nonce mismatch: envelope {}, state {}",
                self.nonce,
                caller.info.nonce
            );
        }
        caller.info.nonce = self
            .nonce
            .checked_add(1)
            .ok_or_else(|| eyre::eyre!("protocol system transaction nonce overflow"))?;
        caller.mark_touch();

        if let Some((address, amount)) = self.balance_increment {
            let account = load_account(db, &mut changes, address)?;
            account.info.balance = account
                .info
                .balance
                .checked_add(amount)
                .ok_or_else(|| eyre::eyre!("protocol system transaction balance overflow"))?;
            account.mark_touch();
        }

        db.commit(changes.clone());
        Ok(changes)
    }
}

fn load_account<'a, DB: alloy_evm::Database>(
    db: &mut DB,
    changes: &'a mut AddressMap<Account>,
    address: Address,
) -> Result<&'a mut Account> {
    match changes.entry(address) {
        Entry::Occupied(entry) => Ok(entry.into_mut()),
        Entry::Vacant(entry) => {
            let info = Database::basic(db, address)?.unwrap_or_default();
            Ok(entry.insert(Account::from(info)))
        }
    }
}

/// Executes one canonical replay transaction, including family-specific protocol system calls.
pub fn execute_replay_transaction<F, E>(
    factory: &F,
    evm: &mut E,
    tx: F::Tx,
) -> Result<ResultAndState<F::HaltReason>>
where
    F: FoundryEvmFactory,
    E: Evm<Tx = F::Tx, HaltReason = F::HaltReason>,
    E::DB: DatabaseCommit,
{
    let Some(system_call) = factory.protocol_system_call(&tx) else {
        return evm.transact(tx).wrap_err("failed to replay transaction");
    };

    let prestate = system_call.apply_prestate(evm.db_mut())?;
    let result = evm
        .transact_system_call(system_call.caller, system_call.contract, system_call.data)
        .wrap_err("failed to execute protocol system transaction")?;
    finish_protocol_system_call(result, prestate)
}

pub(crate) fn finish_protocol_system_call<H>(
    mut result: ResultAndState<H>,
    prestate: AddressMap<Account>,
) -> Result<ResultAndState<H>> {
    if !result.result.is_success() {
        eyre::bail!("protocol system transaction reverted or halted");
    }

    merge_prestate(&mut result.state, prestate);
    Ok(result)
}

fn merge_prestate(state: &mut EvmState, prestate: AddressMap<Account>) {
    for (address, account) in prestate {
        state.entry(address).or_insert(account);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use revm::{database::InMemoryDB, primitives::address, state::AccountInfo};

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
            balance_increment: Some((recipient, U256::from(25))),
        };

        let changes = call.apply_prestate(&mut db).unwrap();

        assert_eq!(changes[&caller].info.nonce, 8);
        assert_eq!(changes[&recipient].info.balance, U256::from(35));
        assert_eq!(db.basic(caller).unwrap().unwrap().nonce, 8);
        assert_eq!(db.basic(recipient).unwrap().unwrap().balance, U256::from(35));
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
            balance_increment: None,
        };

        let err = call.apply_prestate(&mut db).unwrap_err();

        assert!(err.to_string().contains("nonce mismatch"));
        assert_eq!(db.basic(caller).unwrap().unwrap().nonce, 3);
    }
}

#[cfg(all(test, feature = "monad"))]
mod monad_tests {
    use super::*;
    use alloy_evm::{EvmEnv, EvmFactory};
    use alloy_monad_evm::MonadEvmFactory;
    use alloy_sol_types::{SolCall, SolEvent};
    use monad_revm::{
        MonadHardfork,
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
        context::{BlockEnv, CfgEnv, TxEnv},
        database::InMemoryDB,
        inspector::CountInspector,
        primitives::{TxKind, address},
        state::AccountInfo,
    };

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
        let mut evm = factory.create_evm_with_inspector(db, evm_env, CountInspector::default());

        let result = execute_replay_transaction(&factory, &mut evm, tx).unwrap();

        assert!(result.result.is_success());
        assert!(evm.inspector().call_count() > 0);
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
}
