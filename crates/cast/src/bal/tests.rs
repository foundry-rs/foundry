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
    database_interface::{ErasedError, bal::BalDatabase},
    state::{AccountInfo, Bytecode},
};
use std::{cell::RefCell, io};

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
        let state = prepare_prestate(&parent, bal.clone(), index, 3, None, SpecId::CANCUN).unwrap();
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
fn prestate_reads_each_needed_parent_account_once() {
    let [created, existing, read_only, future_storage, future_account, untouched] =
        [1, 2, 3, 4, 5, 6].map(Address::with_last_byte);
    let slot = U256::ZERO;
    let mut db = CountingDb::default();
    db.parent.insert_account_info(created, AccountInfo::from_balance(U256::from(1)));
    db.parent.insert_account_info(existing, AccountInfo { nonce: 1, ..Default::default() });
    db.parent.insert_account_storage(existing, slot, U256::from(77)).unwrap();
    let bal = vec![
        AccountChanges {
            storage_changes: vec![SlotChanges::new(
                slot,
                vec![StorageChange::new(BlockAccessIndex::new(0), U256::from(11))],
            )],
            ..AccountChanges::new(created)
        },
        AccountChanges {
            storage_changes: vec![SlotChanges::new(
                slot,
                vec![StorageChange::new(BlockAccessIndex::new(0), U256::from(22))],
            )],
            ..AccountChanges::new(existing)
        },
        AccountChanges { storage_reads: vec![slot], ..AccountChanges::new(read_only) },
        AccountChanges {
            storage_changes: vec![SlotChanges::new(
                slot,
                vec![StorageChange::new(BlockAccessIndex::new(2), U256::from(33))],
            )],
            ..AccountChanges::new(future_storage)
        },
        AccountChanges {
            balance_changes: vec![BalanceChange::new(BlockAccessIndex::new(2), U256::from(44))],
            ..AccountChanges::new(future_account)
        },
        AccountChanges::new(untouched),
    ];

    let state = prepare_prestate(&db, bal, 0, 2, None, SpecId::CANCUN).unwrap();
    assert_eq!(*db.basic_reads.borrow(), [created, existing, read_only, future_storage]);
    assert_eq!(
        *db.storage_reads.borrow(),
        [(created, slot), (read_only, slot), (future_storage, slot)]
    );
    assert_eq!(state.len(), 2);
    assert_eq!(state[&created].storage[&slot].present_value, U256::from(11));
    assert_eq!(state[&existing].storage[&slot].present_value, U256::from(22));
    assert_eq!(db.parent.storage_ref(existing, slot).unwrap(), U256::from(77));
}

#[test]
fn prestate_preserves_sparse_write_boundaries() {
    let address = Address::with_last_byte(1);
    let slot = U256::ZERO;
    let mut parent = InMemoryDB::default();
    parent.insert_account_info(
        address,
        AccountInfo { balance: U256::from(99), nonce: 99, ..Default::default() },
    );
    parent.insert_account_storage(address, slot, U256::from(99)).unwrap();
    let indices = [2, 4, 6, 8, 10, 12];
    let bal = vec![AccountChanges {
        balance_changes: indices
            .iter()
            .map(|&index| BalanceChange::new(BlockAccessIndex::new(index), U256::from(index)))
            .collect(),
        nonce_changes: indices
            .iter()
            .map(|&index| NonceChange::new(BlockAccessIndex::new(index), index))
            .collect(),
        storage_changes: vec![SlotChanges::new(
            slot,
            indices
                .iter()
                .map(|&index| StorageChange::new(BlockAccessIndex::new(index), U256::from(index)))
                .collect(),
        )],
        ..AccountChanges::new(address)
    }];

    // Six writes exercise Revm's binary search, with targets on writes and in the gaps.
    for (index, expected) in
        [99, 99, 2, 2, 4, 4, 6, 6, 8, 8, 10, 10, 12, 12].into_iter().enumerate()
    {
        let state =
            prepare_prestate(&parent, bal.clone(), index, 14, None, SpecId::CANCUN).unwrap();
        assert_eq!(state.is_empty(), index < 2);
        let mut db = parent.clone();
        db.commit(state);
        let info = db.basic_ref(address).unwrap().unwrap();
        assert_eq!(info.balance, U256::from(expected));
        assert_eq!(info.nonce, expected);
        assert_eq!(db.storage_ref(address, slot).unwrap(), U256::from(expected));
    }
}

#[test]
fn prestate_requires_account_changes_to_create_a_missing_account() {
    let address = Address::with_last_byte(1);
    let parent = InMemoryDB::default();
    let storage = vec![SlotChanges::new(
        U256::ZERO,
        vec![StorageChange::new(BlockAccessIndex::new(0), U256::from(10))],
    )];
    let account = AccountChanges { storage_changes: storage, ..AccountChanges::new(address) };
    let error =
        prepare_prestate(&parent, vec![account.clone()], 0, 1, None, SpecId::CANCUN).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("BAL contains storage changes for missing account {address}")
    );

    let code = Bytecode::new_raw(bytes!("6001"));
    for (changes, expected) in [
        (
            AccountChanges {
                balance_changes: vec![BalanceChange::new(BlockAccessIndex::new(0), U256::from(1))],
                ..account.clone()
            },
            AccountInfo::from_balance(U256::from(1)),
        ),
        (
            AccountChanges {
                nonce_changes: vec![NonceChange::new(BlockAccessIndex::new(0), 1)],
                ..account.clone()
            },
            AccountInfo { nonce: 1, ..Default::default() },
        ),
        (
            AccountChanges {
                code_changes: vec![CodeChange::new(
                    BlockAccessIndex::new(0),
                    code.original_bytes(),
                )],
                ..account
            },
            AccountInfo { code_hash: code.hash_slow(), code: Some(code), ..Default::default() },
        ),
    ] {
        let state = prepare_prestate(&parent, vec![changes], 0, 1, None, SpecId::CANCUN).unwrap();
        assert_eq!(state[&address].info, expected);
        assert!(state[&address].is_touched());
        let mut db = parent.clone();
        db.commit(state);
        assert_eq!(db.basic_ref(address).unwrap(), Some(expected));
        assert_eq!(db.storage_ref(address, U256::ZERO).unwrap(), U256::from(10));
    }
    assert!(parent.basic_ref(address).unwrap().is_none());
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
        let state = prepare_prestate(&parent, bal.clone(), index, 3, None, SpecId::PRAGUE).unwrap();
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

    for index in [0, 1, 2] {
        let db = CountingDb::default();
        // Validate the later account's bytecode before reading any earlier account's parent.
        let invalid_code = vec![
            AccountChanges {
                balance_changes: vec![BalanceChange::new(BlockAccessIndex::new(0), U256::from(1))],
                ..account.clone()
            },
            AccountChanges {
                code_changes: vec![CodeChange::new(
                    BlockAccessIndex::new(index),
                    Bytes::from_static(&[0xef, 0x01]),
                )],
                ..AccountChanges::new(Address::with_last_byte(2))
            },
        ];
        let error = prepare_prestate(&db, invalid_code, 0, 1, None, SpecId::CANCUN).unwrap_err();
        assert_eq!(error.to_string(), "invalid BAL bytecode");
        assert!(db.basic_reads.borrow().is_empty());
    }

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
    parent
        .insert_account_info(first, AccountInfo { balance: U256::from(10), ..Default::default() });
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

#[derive(Default)]
struct CountingDb {
    parent: InMemoryDB,
    basic_reads: RefCell<Vec<Address>>,
    storage_reads: RefCell<Vec<(Address, U256)>>,
}

impl DatabaseRef for CountingDb {
    type Error = ErasedError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.basic_reads.borrow_mut().push(address);
        Ok(self.parent.basic_ref(address).unwrap())
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        Ok(self.parent.code_by_hash_ref(code_hash).unwrap())
    }

    fn storage_ref(&self, address: Address, slot: U256) -> Result<U256, Self::Error> {
        self.storage_reads.borrow_mut().push((address, slot));
        Ok(self.parent.storage_ref(address, slot).unwrap())
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        Ok(self.parent.block_hash_ref(number).unwrap())
    }
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
