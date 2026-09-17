//! Transaction prestate reconstructed from a block access list.
//!
//! The caller supplies an unchanged, hash-pinned parent database and checks network applicability.
//! Alloy validates the list; Revm selects writes strictly before the target's one-based access
//! index, including index-zero system writes. Only a fully prepared overlay is committed, so a
//! failed read or validation leaves the parent ready for replay. A header commitment is checked
//! when present; historical lists without one have the same RPC trust boundary as parent state.
//! Creation-eligible parent accounts with nonzero BAL-listed storage require replay because a
//! creation can reset those slots without recording a BAL write.

use alloy_consensus::BlockHeader;
use alloy_eip7928::{BlockAccessList, compute_block_access_list_hash, validate_block_access_list};
use alloy_eips::BlockNumHash;
use alloy_network::{AnyRpcBlock, AnyRpcTransaction, BlockResponse, TransactionResponse};
use alloy_primitives::B256;
use alloy_rpc_types::BlockTransactions;
use eyre::{Result, WrapErr, ensure};
use revm::{
    Database, DatabaseRef,
    database_interface::{
        WrapDatabaseRef,
        bal::{BalDatabase, BalState},
    },
    primitives::hardfork::SpecId,
    state::{
        Account, EvmState, EvmStorageSlot,
        bal::{Bal, BlockAccessIndex},
    },
};
use std::sync::Arc;

/// Checks that the requested transaction and the fork parent belong to the fetched block.
pub(crate) fn validate_target(
    tx: &AnyRpcTransaction,
    block: &AnyRpcBlock,
    fork_block: BlockNumHash,
) -> Result<usize> {
    let header = block.header();
    ensure!(
        tx.block_hash() == Some(header.hash) && tx.block_number() == Some(header.number()),
        "BAL transaction block does not match the fetched block"
    );
    ensure!(
        header.number().checked_sub(1) == Some(fork_block.number)
            && header.parent_hash() == fork_block.hash,
        "BAL block does not extend the fork parent"
    );
    let BlockTransactions::Full(transactions) = block.transactions() else {
        eyre::bail!("BAL requires full block transactions")
    };
    let index = tx
        .transaction_index()
        .and_then(|index| usize::try_from(index).ok())
        .ok_or_else(|| eyre::eyre!("BAL transaction has no usable block position"))?;
    ensure!(
        transactions.get(index).is_some_and(|candidate| candidate.tx_hash() == tx.tx_hash()),
        "BAL transaction position does not match the fetched block"
    );
    ensure!(
        transactions.iter().filter(|candidate| candidate.tx_hash() == tx.tx_hash()).count() == 1,
        "BAL transaction occurs more than once in the fetched block"
    );
    Ok(index)
}

/// Materializes writes before a transaction without changing the parent database.
pub(crate) fn prepare_prestate<DB: DatabaseRef>(
    db: &DB,
    bal: BlockAccessList,
    transaction_index: usize,
    transaction_count: usize,
    expected_hash: Option<B256>,
    spec: SpecId,
) -> Result<EvmState> {
    // Before EIP-6780, SELFDESTRUCT can clear storage that the BAL does not enumerate.
    ensure!(spec.is_enabled_in(SpecId::CANCUN), "BAL prestate requires Cancun or later");
    ensure!(transaction_index < transaction_count, "BAL transaction position is out of bounds");
    validate_block_access_list(&bal, transaction_count).wrap_err("invalid block access list")?;
    if let Some(expected_hash) = expected_hash {
        ensure!(
            compute_block_access_list_hash(&bal) == expected_hash,
            "block access list does not match the block header commitment"
        );
    }

    let bal = Arc::new(Bal::try_from_alloy(bal).wrap_err("invalid BAL bytecode")?);
    let index = BlockAccessIndex::new(
        u64::try_from(transaction_index)?
            .checked_add(1)
            .ok_or_else(|| eyre::eyre!("BAL transaction position overflow"))?,
    );
    let mut positioned = BalDatabase {
        bal_state: BalState { bal: Some(bal.clone()), bal_index: index, ..Default::default() },
        db: WrapDatabaseRef(db),
    };
    let mut state = EvmState::default();

    for (&address, changes) in &bal.accounts {
        // CREATE can reset storage without a BAL write, including after create-and-selfdestruct.
        // Check read-only and future-write entries too, before skipping unchanged accounts.
        if !changes.storage.storage.is_empty()
            && db.basic_ref(address)?.is_none_or(|info| info.has_no_code_and_nonce())
        {
            for &slot in changes.storage.storage.keys() {
                ensure!(
                    db.storage_ref(address, slot)?.is_zero(),
                    "BAL cannot reconstruct a possible storage reset for {address}"
                );
            }
        }

        // Read-only entries and writes at or after the target need no overlay.
        let has_prior_changes = changes.balance.get(index).is_some()
            || changes.nonce.get(index).is_some()
            || changes.code.get(index).is_some()
            || changes.storage.storage.values().any(|writes| writes.get(index).is_some());
        if !has_prior_changes {
            continue;
        }

        let info = positioned.basic(address)?.ok_or_else(|| {
            eyre::eyre!("BAL contains storage changes for missing account {address}")
        })?;
        let mut account = Account::from(info);
        account.mark_touch();
        for (&slot, writes) in &changes.storage.storage {
            if writes.get(index).is_some() {
                let value = positioned.storage(address, slot)?;
                account.storage.insert(slot, EvmStorageSlot::new(value, Default::default()));
            }
        }
        state.insert(address, account);
    }

    // The caller commits only this completed overlay. A failed read leaves replay on parent state.
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{SignableTransaction, TxLegacy, transaction::Recovered};
    use alloy_eip7928::{
        AccountChanges, BalanceChange, CodeChange, NonceChange, SlotChanges, StorageChange,
    };
    use alloy_primitives::{Address, Bytes, Signature, TxKind, U256, bytes};
    use alloy_rpc_types::{Block, Transaction as RpcTransaction};
    use revm::{
        Context, DatabaseCommit, ExecuteCommitEvm, MainBuilder, MainContext,
        context::TxEnv,
        database::InMemoryDB,
        database_interface::ErasedError,
        state::{AccountInfo, Bytecode},
    };
    use std::io;

    #[test]
    fn target_validation_binds_transaction_position_and_parent() {
        let mut block = Block::<RpcTransaction>::default();
        block.header.hash = B256::with_last_byte(2);
        block.header.inner.number = 10;
        block.header.inner.parent_hash = B256::with_last_byte(1);
        let tx = RpcTransaction {
            inner: Recovered::new_unchecked(
                TxLegacy::default().into_signed(Signature::test_signature()).into(),
                Address::with_last_byte(1),
            ),
            block_hash: Some(block.header.hash),
            block_number: Some(10),
            transaction_index: Some(0),
            effective_gas_price: None,
            block_timestamp: None,
        };
        block.transactions = BlockTransactions::Full(vec![tx.clone()]);
        let tx = <AnyRpcTransaction as From<_>>::from(tx);
        let block = AnyRpcBlock::from(block);
        let parent = BlockNumHash::new(9, B256::with_last_byte(1));
        assert_eq!(validate_target(&tx, &block, parent).unwrap(), 0);

        let mut wrong = tx.clone();
        wrong.transaction_index = Some(1);
        assert!(validate_target(&wrong, &block, parent).is_err());
        wrong = tx.clone();
        wrong.block_hash = Some(B256::ZERO);
        assert!(validate_target(&wrong, &block, parent).is_err());
        wrong = tx.clone();
        wrong.block_number = Some(11);
        assert!(validate_target(&wrong, &block, parent).is_err());
        assert!(validate_target(&tx, &block, BlockNumHash::new(9, B256::ZERO)).is_err());
        assert!(validate_target(&tx, &block, BlockNumHash::new(8, parent.hash)).is_err());

        let mut duplicate = block;
        duplicate.transactions = BlockTransactions::Full(vec![tx.clone(), tx.clone()]);
        assert!(validate_target(&tx, &duplicate, parent).is_err());
    }

    #[test]
    fn prestate_includes_system_writes_and_excludes_target_writes() {
        let address = Address::with_last_byte(1);
        let slot = U256::ZERO;
        let mut db = InMemoryDB::default();
        db.insert_account_info(address, AccountInfo { nonce: 1, ..Default::default() });
        db.insert_account_storage(address, slot, U256::from(10)).unwrap();
        let bal = vec![AccountChanges {
            storage_changes: vec![SlotChanges::new(
                slot,
                vec![
                    StorageChange::new(BlockAccessIndex::new(0), U256::from(20)),
                    StorageChange::new(BlockAccessIndex::new(1), U256::from(30)),
                ],
            )],
            ..AccountChanges::new(address)
        }];

        let prestate = prepare_prestate(&db, bal, 0, 2, None, SpecId::CANCUN).unwrap();
        assert_eq!(db.storage_ref(address, slot).unwrap(), U256::from(10));
        db.commit(prestate);
        assert_eq!(db.storage_ref(address, slot).unwrap(), U256::from(20));
    }

    #[test]
    fn prestate_restores_first_middle_and_last_transaction_boundaries() {
        let address = Address::with_last_byte(1);
        let code = Bytecode::new_raw(Bytes::from_static(&[0x60, 0x01]));
        let mut parent = InMemoryDB::default();
        parent.insert_account_info(
            address,
            AccountInfo { balance: U256::from(100), nonce: 5, ..Default::default() },
        );
        parent.insert_account_storage(address, U256::ZERO, U256::from(10)).unwrap();
        parent.insert_account_storage(address, U256::from(1), U256::from(77)).unwrap();
        let bal = vec![AccountChanges {
            balance_changes: vec![
                BalanceChange::new(BlockAccessIndex::new(1), U256::from(90)),
                BalanceChange::new(BlockAccessIndex::new(2), U256::from(80)),
                BalanceChange::new(BlockAccessIndex::new(3), U256::from(70)),
                BalanceChange::new(BlockAccessIndex::new(4), U256::from(60)),
            ],
            nonce_changes: vec![
                NonceChange::new(BlockAccessIndex::new(1), 6),
                NonceChange::new(BlockAccessIndex::new(2), 7),
                NonceChange::new(BlockAccessIndex::new(3), 8),
            ],
            code_changes: vec![CodeChange::new(BlockAccessIndex::new(1), code.original_bytes())],
            storage_changes: vec![SlotChanges::new(
                U256::ZERO,
                vec![
                    StorageChange::new(BlockAccessIndex::new(1), U256::from(20)),
                    StorageChange::new(BlockAccessIndex::new(2), U256::from(30)),
                    StorageChange::new(BlockAccessIndex::new(3), U256::from(40)),
                ],
            )],
            storage_reads: vec![U256::from(1)],
            ..AccountChanges::new(address)
        }];

        for (index, balance, nonce, value) in [(0, 100, 5, 10), (1, 90, 6, 20), (2, 80, 7, 30)] {
            let state =
                prepare_prestate(&parent, bal.clone(), index, 3, None, SpecId::CANCUN).unwrap();
            let mut db = parent.clone();
            db.commit(state);
            let info = db.basic_ref(address).unwrap().unwrap();
            assert_eq!(info.balance, U256::from(balance));
            assert_eq!(info.nonce, nonce);
            assert_eq!(db.storage_ref(address, U256::ZERO).unwrap(), U256::from(value));
            assert_eq!(db.storage_ref(address, U256::from(1)).unwrap(), U256::from(77));
            if index > 0 {
                assert_eq!(info.code_hash, code.hash_slow());
                assert_eq!(db.code_by_hash_ref(info.code_hash).unwrap(), code);
            }
        }
    }

    #[test]
    fn prestate_rejects_creation_over_parent_storage() {
        let caller = Address::with_last_byte(100);
        let address = caller.create(0);
        let mut parent = InMemoryDB::default();
        parent.insert_account_info(caller, AccountInfo::from_balance(U256::from(1_000_000_000)));
        parent.insert_account_info(address, AccountInfo::from_balance(U256::from(10)));
        parent.insert_account_storage(address, U256::ZERO, U256::from(77)).unwrap();

        let mut db = BalDatabase::new(parent.clone()).with_bal_builder();
        db.bal_state.bal_index = BlockAccessIndex::new(1);
        let mut evm = Context::mainnet()
            .modify_cfg_chained(|cfg| cfg.set_spec_and_mainnet_gas_params(SpecId::CANCUN))
            .with_db(db)
            .build_mainnet();
        // Deploy a contract that returns slot zero, without writing that slot in its constructor.
        let creation = evm
            .transact_commit(
                TxEnv::builder()
                    .caller(caller)
                    .kind(TxKind::Create)
                    .gas_limit(1_000_000)
                    .data(bytes!("600b600c600039600b6000f360005460005260206000f3"))
                    .build()
                    .unwrap(),
            )
            .unwrap();
        assert!(creation.is_success());
        evm.ctx.journaled_state.database.bal_state.bal_index = BlockAccessIndex::new(2);
        let target = evm
            .transact_commit(
                TxEnv::builder()
                    .caller(caller)
                    .nonce(1)
                    .kind(TxKind::Call(address))
                    .gas_limit(1_000_000)
                    .build()
                    .unwrap(),
            )
            .unwrap();
        assert!(target.is_success());
        assert_eq!(target.output().unwrap().as_ref(), &[0; 32]);

        let bal = evm.ctx.journaled_state.database.bal_state.take_built_alloy_bal().unwrap();
        let changes = bal.iter().find(|account| account.address == address).unwrap();
        assert!(changes.storage_changes.is_empty());
        assert_eq!(changes.storage_reads, [U256::ZERO]);
        assert!(prepare_prestate(&parent, bal, 1, 2, None, SpecId::CANCUN).is_err());
        assert_eq!(parent.storage_ref(address, U256::ZERO).unwrap(), U256::from(77));
    }

    #[test]
    fn prestate_rejects_storage_resets_without_prior_writes() {
        let address = Address::with_last_byte(1);
        let mut parent = InMemoryDB::default();
        parent.insert_account_info(address, AccountInfo::default());
        parent.insert_account_storage(address, U256::ZERO, U256::from(77)).unwrap();

        // Creation followed by SELFDESTRUCT can clear storage without any prior BAL writes.
        // A slot the target only reads or first writes still needs the same parent-state check.
        for read_only in [true, false] {
            let mut account = AccountChanges::new(address);
            if read_only {
                account.storage_reads.push(U256::ZERO);
            } else {
                account.storage_changes.push(SlotChanges::new(
                    U256::ZERO,
                    vec![StorageChange::new(BlockAccessIndex::new(2), U256::from(1))],
                ));
            }
            let bal = vec![account];
            validate_block_access_list(&bal, 2).unwrap();
            assert!(prepare_prestate(&parent, bal, 1, 2, None, SpecId::CANCUN).is_err());
        }
    }

    #[test]
    fn prestate_preserves_storage_when_delegation_is_installed_and_cleared() {
        let address = Address::with_last_byte(1);
        let delegation = Bytecode::new_eip7702(Address::with_last_byte(2));
        let mut parent = InMemoryDB::default();
        // An authority can retain storage after clearing a previous delegation.
        parent.insert_account_info(address, AccountInfo { nonce: 1, ..Default::default() });
        parent.insert_account_storage(address, U256::ZERO, U256::from(77)).unwrap();
        let bal = vec![AccountChanges {
            nonce_changes: vec![
                NonceChange::new(BlockAccessIndex::new(1), 2),
                NonceChange::new(BlockAccessIndex::new(2), 3),
            ],
            code_changes: vec![
                CodeChange::new(BlockAccessIndex::new(1), delegation.original_bytes()),
                CodeChange::new(BlockAccessIndex::new(2), Bytes::new()),
            ],
            storage_reads: vec![U256::ZERO],
            ..AccountChanges::new(address)
        }];

        for (index, nonce, code) in
            [(0, 1, Bytecode::default()), (1, 2, delegation), (2, 3, Bytecode::default())]
        {
            let state =
                prepare_prestate(&parent, bal.clone(), index, 3, None, SpecId::PRAGUE).unwrap();
            let mut db = parent.clone();
            db.commit(state);
            let info = db.basic_ref(address).unwrap().unwrap();
            assert_eq!(info.nonce, nonce);
            assert_eq!(info.code_hash, code.hash_slow());
            assert_eq!(db.code_by_hash_ref(info.code_hash).unwrap(), code);
            assert_eq!(db.storage_ref(address, U256::ZERO).unwrap(), U256::from(77));
        }
    }

    #[test]
    fn prestate_rejects_invalid_structure_bytecode_and_commitment() {
        let db = InMemoryDB::default();
        let address = Address::with_last_byte(1);
        let account = AccountChanges::new(address);
        let duplicate = vec![account.clone(), account.clone()];
        assert!(prepare_prestate(&db, duplicate, 0, 1, None, SpecId::CANCUN).is_err());

        let duplicate_index = vec![AccountChanges {
            balance_changes: vec![
                BalanceChange::new(BlockAccessIndex::new(1), U256::from(1)),
                BalanceChange::new(BlockAccessIndex::new(1), U256::from(2)),
            ],
            ..account.clone()
        }];
        assert!(prepare_prestate(&db, duplicate_index, 0, 1, None, SpecId::CANCUN).is_err());

        let invalid_code = vec![AccountChanges {
            code_changes: vec![CodeChange::new(
                BlockAccessIndex::new(1),
                Bytes::from_static(&[0xef, 0x01]),
            )],
            ..account.clone()
        }];
        assert!(prepare_prestate(&db, invalid_code, 0, 1, None, SpecId::CANCUN).is_err());

        let bal = vec![account];
        let hash = compute_block_access_list_hash(&bal);
        assert!(prepare_prestate(&db, bal.clone(), 0, 1, Some(hash), SpecId::CANCUN).is_ok());
        assert!(prepare_prestate(&db, bal, 0, 1, Some(B256::ZERO), SpecId::CANCUN).is_err());
    }

    #[test]
    fn prestate_rejects_historical_selfdestruct_semantics_and_invalid_position() {
        let db = InMemoryDB::default();
        assert!(prepare_prestate(&db, vec![], 0, 1, None, SpecId::SHANGHAI).is_err());
        assert!(prepare_prestate(&db, vec![], 1, 1, None, SpecId::CANCUN).is_err());
        assert!(prepare_prestate(&db, vec![], 0, 0, None, SpecId::CANCUN).is_err());
    }

    #[test]
    fn prestate_read_failure_leaves_parent_state_untouched() {
        let first = Address::with_last_byte(1);
        let second = Address::with_last_byte(2);
        let mut parent = InMemoryDB::default();
        parent.insert_account_info(
            first,
            AccountInfo { balance: U256::from(10), ..Default::default() },
        );
        let db = FailingDb { parent, failure: second };
        let bal = [first, second]
            .map(|address| AccountChanges {
                balance_changes: vec![BalanceChange::new(BlockAccessIndex::new(0), U256::from(20))],
                ..AccountChanges::new(address)
            })
            .to_vec();

        let error = prepare_prestate(&db, bal, 0, 1, None, SpecId::CANCUN).unwrap_err();
        assert!(error.to_string().contains("unavailable parent account"));
        assert_eq!(db.basic_ref(first).unwrap().unwrap().balance, U256::from(10));
    }

    struct FailingDb {
        parent: InMemoryDB,
        failure: Address,
    }

    impl DatabaseRef for FailingDb {
        type Error = ErasedError;

        fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
            if address == self.failure {
                return Err(ErasedError::new(io::Error::other("unavailable parent account")));
            }
            Ok(self.parent.basic_ref(address).unwrap())
        }

        fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
            Ok(self.parent.code_by_hash_ref(code_hash).unwrap())
        }

        fn storage_ref(&self, address: Address, slot: U256) -> Result<U256, Self::Error> {
            Ok(self.parent.storage_ref(address, slot).unwrap())
        }

        fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
            Ok(self.parent.block_hash_ref(number).unwrap())
        }
    }
}
