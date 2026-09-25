//! Tests for block access list state reads and commits.

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
fn bal_reads_use_index_and_fall_back_to_parent_state() {
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

    backend.set_bal(bal.clone(), BlockAccessIndex::new(1));
    let account = backend.basic(address).unwrap().unwrap();
    assert_eq!(account.balance, U256::from(20));
    assert_eq!(account.code.unwrap().original_bytes(), code);
    assert_eq!(backend.storage(address, slot).unwrap(), U256::from(20));
    assert_eq!(backend.basic_ref(address).unwrap().unwrap().balance, U256::from(20));
    assert_eq!(backend.storage_ref(address, slot).unwrap(), U256::from(20));
    // A slot the block only read keeps its parent value.
    assert_eq!(backend.storage_ref(address, U256::from(99)).unwrap(), U256::from(77));
    let mut cow = CowBackend::new_borrowed(&backend);
    assert_eq!(cow.basic(address).unwrap().unwrap().balance, U256::from(20));
    assert_eq!(cow.storage(address, slot).unwrap(), U256::from(20));

    backend.set_bal(bal, BlockAccessIndex::new(2));
    assert_eq!(backend.basic(address).unwrap().unwrap().balance, U256::from(30));
    assert_eq!(backend.storage(address, slot).unwrap(), U256::from(30));
    // State the list does not mention was untouched by the block and comes from the parent
    // database, which reports every unknown account as an existing empty account.
    assert_eq!(backend.storage(address, U256::from(100)).unwrap(), U256::ZERO);
    assert_eq!(backend.basic(Address::with_last_byte(2)).unwrap(), Some(AccountInfo::default()));

    // Committing removes the list again.
    backend.commit(Default::default());
    assert_eq!(backend.basic(address).unwrap().unwrap().balance, U256::from(10));
    assert_eq!(backend.storage(address, slot).unwrap(), U256::from(10));
    assert_eq!(backend.basic_ref(address).unwrap().unwrap().nonce, 1);
    assert_eq!(backend.storage_ref(address, slot).unwrap(), U256::from(10));
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
    backend.insert_account_info(sender, AccountInfo { balance: U256::MAX, ..Default::default() });
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
    backend.set_bal(Arc::new(bal), BlockAccessIndex::new(2));
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
    assert_eq!(backend.code_by_hash_ref(account.code_hash).unwrap().original_bytes(), prefix_code);
    assert!(backend.basic_ref(missing).unwrap().is_none());
}
