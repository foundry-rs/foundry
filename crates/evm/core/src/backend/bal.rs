//! Native BAL reads for a single transaction executed against its unchanged parent state.

use super::{Backend, DatabaseError, DatabaseResult};
use crate::evm::FoundryEvmNetwork;
use alloy_primitives::{Address, U256};
use revm::{
    DatabaseRef,
    database_interface::bal::BalState,
    state::{
        AccountInfo,
        bal::{Bal, BlockAccessIndex},
    },
};
use std::sync::Arc;

impl<FEN: FoundryEvmNetwork> Backend<FEN> {
    /// Positions native BAL reads before a transaction without changing the parent database.
    ///
    /// The caller must supply a validated BAL for Cancun or later and use `transaction_index + 1`.
    /// Install it only for a single transaction on an unchanged parent. Committing the transaction
    /// removes BAL and preserves accessed account code for trace decoding. Discard the backend
    /// afterward: skipped prefix storage is not materialized for subsequent transactions.
    /// Accounts that could have been created in the skipped prefix require ordinary replay
    /// when their storage is read: BAL does not enumerate every slot cleared by CREATE.
    pub fn set_bal(&mut self, bal: Option<Arc<Bal>>, index: BlockAccessIndex) {
        self.bal =
            bal.map(|bal| BalState { bal: Some(bal), bal_index: index, ..Default::default() });
    }

    pub(super) fn apply_bal_account(
        &self,
        address: Address,
        account: &mut Option<AccountInfo>,
    ) -> DatabaseResult<()> {
        if let Some(bal) = &self.bal {
            bal.basic(address, account)
                .map_err(|err| DatabaseError::GetAccount(address, Arc::new(err.into())))?;
        }
        Ok(())
    }

    pub(super) fn bal_storage(
        &self,
        address: Address,
        index: U256,
    ) -> DatabaseResult<Option<U256>> {
        let Some(bal) = &self.bal else { return Ok(None) };
        // Inspect raw parent account metadata, not BAL-overlaid nonce or code. A skipped CREATE
        // can clear unlisted storage, including in index-zero system execution. Reject even when
        // BAL has a prior slot write, since a later creation might have cleared that value.
        let parent = if let Some(db) = self.active_fork_db() {
            db.basic_ref(address)?
        } else {
            self.mem_db.basic_ref(address)?
        };
        if parent.is_none_or(|account| account.has_no_code_and_nonce()) {
            return Err(DatabaseError::GetStorage(
                address,
                index,
                Arc::new(eyre::eyre!(
                    "BAL cannot exclude a storage reset for {address}; replay required"
                )),
            ));
        }
        bal.storage(&address, index)
            .map_err(|err| DatabaseError::GetStorage(address, index, Arc::new(err.into())))
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        backend::{Backend, CowBackend},
        evm::EthEvmNetwork,
    };
    use alloy_eips::eip7928::{
        AccountChanges, BalanceChange, CodeChange, NonceChange, SlotChanges, StorageChange,
    };
    use alloy_primitives::{Address, TxKind, U256, bytes};
    use revm::{
        Context, Database, DatabaseCommit, DatabaseRef, ExecuteEvm, MainBuilder, MainContext,
        context::TxEnv,
        context_interface::{
            either::Either,
            transaction::{Authorization, RecoveredAuthority, RecoveredAuthorization},
        },
        database::DbAccount,
        primitives::hardfork::SpecId,
        state::{
            AccountInfo, Bytecode,
            bal::{Bal, BlockAccessIndex},
        },
    };
    use std::sync::Arc;

    #[test]
    fn bal_reads_use_index_without_changing_parent_cache() {
        let address = Address::with_last_byte(1);
        let slot = U256::ZERO;
        let mut backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        backend.insert_account_info(
            address,
            AccountInfo { nonce: 1, balance: U256::from(10), ..Default::default() },
        );
        backend.insert_account_storage(address, slot, U256::from(10)).unwrap();
        backend.insert_account_storage(address, U256::from(99), U256::from(77)).unwrap();
        let code = bytes!("60005400");
        let bal = Arc::new(
            Bal::try_from_alloy(vec![AccountChanges {
                balance_changes: vec![
                    BalanceChange::new(BlockAccessIndex::new(0), U256::from(20)),
                    BalanceChange::new(BlockAccessIndex::new(1), U256::from(30)),
                ],
                code_changes: vec![CodeChange::new(BlockAccessIndex::new(0), code.clone())],
                storage_changes: vec![SlotChanges::new(
                    slot,
                    vec![
                        StorageChange::new(BlockAccessIndex::new(0), U256::from(20)),
                        StorageChange::new(BlockAccessIndex::new(1), U256::from(30)),
                    ],
                )],
                storage_reads: vec![U256::from(99)],
                ..AccountChanges::new(address)
            }])
            .unwrap(),
        );

        backend.set_bal(Some(bal.clone()), BlockAccessIndex::new(1));
        let account = backend.basic(address).unwrap().unwrap();
        assert_eq!(account.balance, U256::from(20));
        assert_eq!(account.code.unwrap().original_bytes(), code);
        assert_eq!(backend.storage(address, slot).unwrap(), U256::from(20));
        assert_eq!(backend.basic_ref(address).unwrap().unwrap().balance, U256::from(20));
        assert_eq!(backend.storage_ref(address, slot).unwrap(), U256::from(20));
        assert_eq!(backend.storage_ref(address, U256::from(99)).unwrap(), U256::from(77));
        let mut cow = CowBackend::new_borrowed(&backend);
        assert_eq!(cow.basic(address).unwrap().unwrap().balance, U256::from(20));
        assert_eq!(cow.storage(address, slot).unwrap(), U256::from(20));

        backend.set_bal(Some(bal), BlockAccessIndex::new(2));
        assert_eq!(backend.basic(address).unwrap().unwrap().balance, U256::from(30));
        assert_eq!(backend.storage(address, slot).unwrap(), U256::from(30));
        assert!(backend.storage(address, U256::from(100)).is_err());
        assert!(backend.basic(Address::with_last_byte(2)).is_err());

        backend.set_bal(None, BlockAccessIndex::PRE_EXECUTION);
        assert_eq!(backend.basic(address).unwrap().unwrap().balance, U256::from(10));
        assert_eq!(backend.storage(address, slot).unwrap(), U256::from(10));
        assert_eq!(backend.basic_ref(address).unwrap().unwrap().nonce, 1);
        assert_eq!(backend.storage_ref(address, slot).unwrap(), U256::from(10));
    }

    #[test]
    fn bal_rejects_possible_storage_resets_including_system_creation() {
        let address = Address::with_last_byte(1);
        let slot = U256::from(99);
        // A prior BAL value is insufficient: a subsequent CREATE can reset that storage.
        for prior_value in [None, Some(U256::from(55))] {
            let mut changes = AccountChanges::new(address);
            if let Some(value) = prior_value {
                changes.storage_changes.push(SlotChanges::new(
                    slot,
                    vec![StorageChange::new(BlockAccessIndex::new(0), value)],
                ));
            } else {
                changes.storage_reads.push(slot);
            }
            let bal = Arc::new(Bal::try_from_alloy(vec![changes]).unwrap());
            for parent_exists in [false, true] {
                let mut backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
                if parent_exists {
                    backend.insert_account_info(address, AccountInfo::default());
                    backend.insert_account_storage(address, slot, U256::from(77)).unwrap();
                } else {
                    backend.mem_db.cache.accounts.insert(address, DbAccount::new_not_existing());
                }
                for index in [1, 2] {
                    backend.set_bal(Some(bal.clone()), BlockAccessIndex::new(index));
                    assert!(backend.storage(address, slot).is_err());
                    assert!(backend.storage_ref(address, slot).is_err());
                }
            }
        }
    }

    #[test]
    fn bal_commit_preserves_target_and_untouched_code() {
        let sender = Address::repeat_byte(0x11);
        let authority = Address::repeat_byte(0x22);
        let delegate = Address::repeat_byte(0x33);
        let readonly = Address::repeat_byte(0x44);
        let missing = Address::repeat_byte(0x66);
        let mut backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        backend.mem_db.cache.accounts.insert(missing, DbAccount::new_not_existing());
        backend
            .insert_account_info(sender, AccountInfo { balance: U256::MAX, ..Default::default() });
        backend.insert_account_info(authority, AccountInfo::default());
        backend.insert_account_info(readonly, AccountInfo { nonce: 1, ..Default::default() });
        // Read code without touching either account, including an account absent from the parent.
        let mut runtime = Vec::new();
        for address in [readonly, missing] {
            runtime.push(0x73);
            runtime.extend_from_slice(address.as_slice());
            runtime.extend_from_slice(&[0x3b, 0x50]);
        }
        runtime.push(0x00);
        let runtime = Bytecode::new_legacy(runtime.into());
        backend.insert_account_info(
            delegate,
            AccountInfo {
                nonce: 1,
                code_hash: runtime.hash_slow(),
                code: Some(runtime),
                ..Default::default()
            },
        );
        let prefix_code = bytes!("600100");
        let bal = Bal::try_from_alloy(vec![
            AccountChanges::new(Address::ZERO),
            AccountChanges::new(sender),
            AccountChanges {
                nonce_changes: vec![NonceChange::new(BlockAccessIndex::new(1), 1)],
                code_changes: vec![CodeChange::new(
                    BlockAccessIndex::new(1),
                    Bytecode::new_eip7702(Address::repeat_byte(0x55)).original_bytes(),
                )],
                ..AccountChanges::new(authority)
            },
            AccountChanges::new(delegate),
            AccountChanges {
                code_changes: vec![CodeChange::new(BlockAccessIndex::new(1), prefix_code.clone())],
                ..AccountChanges::new(readonly)
            },
            AccountChanges::new(missing),
        ])
        .unwrap();
        backend.set_bal(Some(Arc::new(bal)), BlockAccessIndex::new(2));
        let authorization = RecoveredAuthorization::new_unchecked(
            Authorization { chain_id: U256::ZERO, address: delegate, nonce: 1 },
            RecoveredAuthority::Valid(authority),
        );
        let result = Context::mainnet()
            .modify_cfg_chained(|cfg| cfg.set_spec_and_mainnet_gas_params(SpecId::PRAGUE))
            .with_db(&mut backend)
            .build_mainnet()
            .transact(
                TxEnv::builder()
                    .caller(sender)
                    .kind(TxKind::Call(authority))
                    .gas_limit(100_000)
                    .authorization_list(vec![Either::Right(authorization)])
                    .build()
                    .unwrap(),
            )
            .unwrap();
        assert!(result.result.is_success());
        assert!(result.state[&authority].is_touched());
        assert!(!result.state[&readonly].is_touched());
        assert!(result.state[&missing].is_loaded_as_not_existing());
        backend.commit(result.state);

        assert_eq!(
            backend.basic_ref(authority).unwrap().unwrap().code,
            Some(Bytecode::new_eip7702(delegate))
        );
        let account = backend.basic_ref(readonly).unwrap().unwrap();
        assert_eq!(account.code.unwrap().original_bytes(), prefix_code);
        assert_eq!(
            backend.code_by_hash_ref(account.code_hash).unwrap().original_bytes(),
            prefix_code
        );
        assert!(backend.basic_ref(missing).unwrap().is_none());
    }
}
