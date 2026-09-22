//! Unit tests for BAL validation and fork cache seeding.

use super::*;
use alloy_eips::eip7928::{
    AccountChanges, BalanceChange, BlockAccessIndex, CodeChange, NonceChange, SlotChanges,
    StorageChange,
};
use alloy_primitives::{Bytes, bytes};

fn database(hash: B256) -> BlockchainDb {
    BlockchainDb::new(
        BlockchainDbMeta::new(BlockEnv::default(), "http://localhost:8545".to_string())
            .with_fork_identity(hash, B256::ZERO),
        None,
    )
}

fn index(index: u64) -> BlockAccessIndex {
    BlockAccessIndex::new(index)
}

#[test]
fn fork_bal_seed_preserves_storage_boundaries() {
    let hash = B256::repeat_byte(1);
    let address = Address::repeat_byte(1);
    let slot = U256::from(1);
    let system_slot = U256::from(2);
    let account = AccountChanges::new(address)
        .with_storage_change(SlotChanges::new(
            slot,
            vec![
                StorageChange::new(index(0), U256::from(3)),
                StorageChange::new(index(1), U256::from(4)),
                StorageChange::new(index(3), U256::ZERO),
            ],
        ))
        .with_storage_change(SlotChanges::new(
            system_slot,
            vec![StorageChange::new(index(0), U256::from(5))],
        ));
    let db = database(hash);

    PreparedBalSeed::new(vec![account], hash, 2, None).unwrap().apply(&db);

    let storage = db.storage().read();
    assert_eq!(storage[&address][&slot], U256::ZERO);
    assert_eq!(storage[&address][&system_slot], U256::from(5));
    assert!(db.accounts().read().is_empty());

    let post_execution_slot = U256::from(3);
    let post_execution = AccountChanges::new(address).with_storage_change(SlotChanges::new(
        post_execution_slot,
        vec![StorageChange::new(index(1), U256::from(42))],
    ));
    let db = database(hash);
    PreparedBalSeed::new(vec![post_execution], hash, 0, None).unwrap().apply(&db);
    assert_eq!(db.storage().read()[&address][&post_execution_slot], U256::from(42));
}

#[test]
fn fork_bal_seed_leaves_partial_accounts_and_reads_unknown() {
    let hash = B256::repeat_byte(1);
    let address = Address::repeat_byte(1);
    let account = AccountChanges::new(address)
        .with_balance_change(BalanceChange::new(index(1), U256::from(42)))
        .with_storage_read(U256::from(2));
    let db = database(hash);

    let seed = PreparedBalSeed::new(vec![account], hash, 1, None).unwrap();
    assert!(seed.is_empty());
    seed.apply(&db);

    assert!(db.accounts().read().is_empty());
    assert!(db.storage().read().is_empty());
}

fn complete_account(address: Address, code: Bytes) -> AccountChanges {
    AccountChanges::new(address)
        .with_balance_change(BalanceChange::new(index(1), U256::from(42)))
        .with_nonce_change(NonceChange::new(index(1), 3))
        .with_code_change(CodeChange::new(index(1), code))
}

#[test]
fn fork_bal_seed_preserves_cached_values_and_merges_slots() {
    let hash = B256::repeat_byte(1);
    let address = Address::repeat_byte(1);
    let account = complete_account(address, Bytes::new())
        .with_storage_change(SlotChanges::new(
            U256::from(1),
            vec![StorageChange::new(index(1), U256::from(11))],
        ))
        .with_storage_change(SlotChanges::new(
            U256::from(2),
            vec![StorageChange::new(index(1), U256::from(22))],
        ));
    let db = database(hash);
    let cached_account = AccountInfo { balance: U256::from(99), ..Default::default() };
    db.accounts().write().insert(address, cached_account.clone());
    db.storage().write().insert(
        address,
        [(U256::from(1), U256::from(101)), (U256::from(3), U256::from(303))].into_iter().collect(),
    );

    PreparedBalSeed::new(vec![account], hash, 1, None).unwrap().apply(&db);

    assert_eq!(db.accounts().read()[&address], cached_account);
    assert_eq!(
        db.storage().read()[&address],
        [
            (U256::from(1), U256::from(101)),
            (U256::from(2), U256::from(22)),
            (U256::from(3), U256::from(303)),
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn fork_bal_seed_survives_disk_cache_reload() {
    let hash = B256::repeat_byte(1);
    let address = Address::repeat_byte(1);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("storage.json");
    let meta = BlockchainDbMeta::new(BlockEnv::default(), "http://localhost:8545".to_string())
        .with_fork_identity(hash, B256::ZERO);
    let db = BlockchainDb::new(meta.clone(), Some(path.clone()));
    db.storage().write().entry(address).or_default().insert(U256::from(9), U256::from(99));
    db.cache().flush();
    drop(db);

    let db = BlockchainDb::new(meta.clone(), Some(path.clone()));
    let code = bytes!("6000");
    let account = complete_account(address, code.clone()).with_storage_change(SlotChanges::new(
        U256::ONE,
        vec![StorageChange::new(index(1), U256::ZERO)],
    ));
    let authority = Address::repeat_byte(3);
    let cleared = Address::repeat_byte(4);
    let delegation = bytes!("ef01000000000000000000000000000000000000000042");
    let cleared_account = complete_account(cleared, delegation.clone())
        .with_balance_change(BalanceChange::new(index(2), U256::ZERO))
        .with_nonce_change(NonceChange::new(index(2), 0))
        .with_code_change(CodeChange::new(index(2), Bytes::new()));
    let authority_account = complete_account(authority, delegation.clone());
    PreparedBalSeed::new(vec![account, authority_account, cleared_account], hash, 2, None)
        .unwrap()
        .apply(&db);
    db.cache().flush();
    drop(db);

    let db = BlockchainDb::new(meta, Some(path));
    assert_eq!(db.storage().read()[&address][&U256::ONE], U256::ZERO);
    assert_eq!(db.storage().read()[&address][&U256::from(9)], U256::from(99));
    assert!(!db.storage().read()[&address].contains_key(&U256::from(2)));
    let accounts = db.accounts().read();
    let account = &accounts[&address];
    assert_eq!(account.balance, U256::from(42));
    assert_eq!(account.nonce, 3);
    assert_eq!(account.code.as_ref().unwrap().original_bytes(), code);
    let account = &accounts[&authority];
    assert_eq!(account.balance, U256::from(42));
    assert_eq!(account.nonce, 3);
    assert_eq!(account.code_hash, alloy_primitives::keccak256(&delegation));
    assert_eq!(account.code.as_ref().unwrap().original_bytes(), delegation);
    assert!(account.code.as_ref().unwrap().is_eip7702());
    let account = &accounts[&cleared];
    assert_eq!(account.balance, U256::ZERO);
    assert_eq!(account.nonce, 0);
    assert_eq!(account.code_hash, alloy_primitives::KECCAK256_EMPTY);
    assert!(account.code.as_ref().unwrap().is_empty());
}

#[test]
fn fork_bal_seed_rejects_wrong_block_without_mutation() {
    let hash = B256::repeat_byte(1);
    let address = Address::repeat_byte(1);
    let account = complete_account(address, Bytes::new()).with_storage_change(SlotChanges::new(
        U256::from(1),
        vec![StorageChange::new(index(0), U256::from(42))],
    ));
    let db = database(B256::repeat_byte(2));

    PreparedBalSeed::new(vec![account], hash, 1, None).unwrap().apply(&db);

    assert!(db.accounts().read().is_empty());
    assert!(db.storage().read().is_empty());
}

#[test]
fn fork_bal_seed_rejects_invalid_structure_hash_and_bytecode() {
    let hash = B256::repeat_byte(1);
    let address = Address::repeat_byte(1);
    let valid = vec![complete_account(address, Bytes::new())];
    let commitment = compute_block_access_list_hash(&valid);
    assert!(PreparedBalSeed::new(valid.clone(), hash, 1, Some(commitment)).is_ok());
    assert!(PreparedBalSeed::new(valid.clone(), hash, 1, Some(B256::ZERO)).is_err());

    let duplicate_accounts = vec![valid[0].clone(), valid[0].clone()];
    assert!(PreparedBalSeed::new(duplicate_accounts, hash, 1, None).is_err());
    let invalid_index =
        AccountChanges::new(address).with_balance_change(BalanceChange::new(index(3), U256::ZERO));
    assert!(PreparedBalSeed::new(vec![invalid_index], hash, 1, None).is_err());
    let empty_changes =
        AccountChanges::new(address).with_storage_change(SlotChanges::new(U256::from(1), vec![]));
    assert!(PreparedBalSeed::new(vec![empty_changes], hash, 1, None).is_err());

    // Invalid code must discard the seed even for an incomplete account or an earlier write.
    let invalid_code = AccountChanges::new(address)
        .with_code_change(CodeChange::new(index(0), bytes!("ef0100")))
        .with_code_change(CodeChange::new(index(1), Bytes::new()));
    assert!(PreparedBalSeed::new(vec![invalid_code], hash, 1, None).is_err());
}
