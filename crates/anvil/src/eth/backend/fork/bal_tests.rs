//! Unit tests for BAL source eligibility, validation and fork cache seeding.

use super::{ClientForkConfig, ForkEndpointIdentity, cache_bal, validate_bal};
use alloy_eips::eip7928::{
    AccountChanges, BalanceChange, BlockAccessIndex, CodeChange, NonceChange, SlotChanges,
    StorageChange, compute_block_access_list_hash,
};
use alloy_network::{AnyHeader, AnyNetwork, AnyRpcBlock, AnyRpcHeader};
use alloy_primitives::{Address, B256, Bytes, U256, bytes};
use alloy_provider::ProviderBuilder;
use alloy_rpc_types::{Block, BlockTransactions};
use alloy_transport::mock::Asserter;
use foundry_evm::{
    backend::{BlockchainDb, BlockchainDbMeta},
    hardfork::EthereumHardfork,
};
use foundry_evm_networks::NetworkVariant;
use revm::{context::BlockEnv, state::AccountInfo};
use std::{sync::Arc, time::Duration};

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

fn fork_config(asserter: Asserter, block_hash: B256) -> ClientForkConfig {
    ClientForkConfig {
        fork_urls: vec!["http://localhost:8545".to_string()],
        block_number: 1,
        evm_block_number: 1,
        block_hash,
        transaction_hash: None,
        provider: Arc::new(
            ProviderBuilder::<_, _, AnyNetwork>::default().connect_mocked_client(asserter),
        ),
        chain_id: 1,
        execution_chain_id: 1,
        override_chain_id: None,
        fork_chain_id: None,
        hardfork: None,
        endpoint_identity: ForkEndpointIdentity {
            execution_chain_id: 1,
            source_chain_id: 1,
            network: None,
            network_profile: None,
            hardfork: None,
            instance_id: None,
            source_fork_block_number: None,
            source_fork_block_hash: None,
        },
        state_is_mutable: false,
        timestamp: 1_800_000_000,
        base_fee: None,
        blob_gas_used: None,
        blob_excess_gas_and_price: None,
        timeout: Duration::from_secs(1),
        retries: 0,
        backoff: Duration::ZERO,
        compute_units_per_second: 0,
        headers: vec![],
        total_difficulty: U256::ZERO,
    }
}

#[tokio::test]
async fn fork_bal_prefill_uses_source_rules() {
    let hash = B256::repeat_byte(1);
    let address = Address::repeat_byte(1);
    let ethereum = Some(NetworkVariant::Ethereum);
    let cancun = Some(EthereumHardfork::Cancun);
    let shanghai = Some(EthereumHardfork::Shanghai);
    for (name, chain_id, network, hardfork, timestamp, eligible) in [
        ("mainnet", 1, ethereum, None, 1_800_000_000, true),
        ("mainnet before Cancun", 1, ethereum, None, 1_600_000_000, false),
        ("undiscovered network", 1, None, None, 1_800_000_000, true),
        ("Sepolia", 11_155_111, ethereum, None, 1_800_000_000, true),
        ("Holesky", 17_000, ethereum, None, 1_800_000_000, true),
        ("Hoodi", 560_048, ethereum, None, 1_800_000_000, true),
        ("unknown source", 31_337, ethereum, None, 1_800_000_000, false),
        ("Arbitrum source", 42_161, ethereum, None, 1_800_000_000, false),
        ("Tempo source", 1, Some(NetworkVariant::Tempo), cancun, 1_800_000_000, false),
        ("Anvil before Cancun", 1, ethereum, shanghai, 1_800_000_000, false),
        ("Anvil with Cancun override", 1, ethereum, cancun, 1_600_000_000, true),
        ("Anvil custom chain", 31_337, ethereum, cancun, 1_800_000_000, true),
    ] {
        let asserter = Asserter::new();
        let mut config = fork_config(asserter.clone(), hash);
        config.endpoint_identity.source_chain_id = chain_id;
        config.endpoint_identity.network = network;
        config.endpoint_identity.hardfork = hardfork.map(Into::into);
        config.timestamp = timestamp;
        // Local execution overrides must not make an ineligible source eligible.
        config.hardfork = Some(EthereumHardfork::Amsterdam.into());
        config.execution_chain_id = 31_337;
        config.override_chain_id = Some(31_337);
        config.endpoint_identity.execution_chain_id = 31_337;
        config.state_is_mutable = true;

        let bal = vec![AccountChanges::new(address).with_storage_change(SlotChanges::new(
            U256::ZERO,
            vec![StorageChange::new(index(1), U256::ONE)],
        ))];
        let block = AnyRpcBlock::new(
            Block::new(
                AnyRpcHeader {
                    hash,
                    inner: AnyHeader {
                        number: config.block_number,
                        timestamp,
                        block_access_list_hash: Some(compute_block_access_list_hash(&bal)),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                BlockTransactions::Hashes(vec![B256::repeat_byte(2)]),
            )
            .into(),
        );
        asserter.push_success(&block);
        asserter.push_success(&bal);
        asserter.push_success(&U256::from(99));
        let queued = asserter.read_q().len();
        let db = database(hash);

        config.prefill_cache(&db, false).await;

        assert_eq!(asserter.read_q().len(), if eligible { 0 } else { queued }, "{name}");
        assert_eq!(
            db.storage().read().get(&address).and_then(|slots| slots.get(&U256::ZERO)).copied(),
            eligible.then_some(U256::from(99)),
            "{name}"
        );
        assert!(db.accounts().read().is_empty(), "{name}");
    }
}

#[tokio::test]
async fn fork_bal_prefill_validates_before_caching_and_refetches_only_mutable_state() {
    let hash = B256::repeat_byte(1);
    let address = Address::repeat_byte(1);
    for (wrong_block, bad_commitment, mutable) in
        [(false, false, false), (false, true, false), (true, false, false), (false, false, true)]
    {
        let asserter = Asserter::new();
        let mut config = fork_config(asserter.clone(), hash);
        config.state_is_mutable = mutable;
        let account = if mutable {
            AccountChanges::new(address)
        } else {
            complete_account(address, bytes!("6000"))
        };
        let bal = vec![account.with_storage_change(SlotChanges::new(
            U256::ONE,
            vec![StorageChange::new(index(1), U256::ONE)],
        ))];
        let block = AnyRpcBlock::new(
            Block::new(
                AnyRpcHeader {
                    hash,
                    inner: AnyHeader {
                        number: config.block_number,
                        timestamp: config.timestamp,
                        block_access_list_hash: Some(if bad_commitment {
                            B256::ZERO
                        } else {
                            compute_block_access_list_hash(&bal)
                        }),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                BlockTransactions::Hashes(vec![B256::repeat_byte(2)]),
            )
            .into(),
        );
        let unsupported = || {
            asserter.push_failure(
                serde_json::from_str(r#"{"code":-32601,"message":"method not found"}"#).unwrap(),
            );
        };
        if !mutable {
            unsupported();
        }
        asserter.push_success(&block);
        asserter.push_success(&bal);
        if mutable {
            asserter.push_success(&U256::from(99));
        } else if !bad_commitment {
            unsupported();
        }
        let queued = asserter.read_q().len();
        let db = database(if wrong_block { B256::ZERO } else { hash });

        config.prefill_cache(&db, false).await;

        assert_eq!(asserter.read_q().len(), if wrong_block { queued } else { 0 });
        if wrong_block || bad_commitment {
            assert!(db.accounts().read().is_empty());
            assert!(db.storage().read().is_empty());
        } else {
            assert_eq!(
                db.storage().read()[&address][&U256::ONE],
                if mutable { U256::from(99) } else { U256::ONE }
            );
            if mutable {
                assert!(db.accounts().read().is_empty());
            } else {
                let account = &db.accounts().read()[&address];
                assert_eq!(account.balance, U256::from(42));
                assert_eq!(account.nonce, 3);
                assert_eq!(account.code.as_ref().unwrap().original_bytes(), bytes!("6000"));
            }
        }
    }
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

    cache_bal(&db, vec![account]);

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
    validate_bal(&vec![post_execution.clone()], 0, None).unwrap();
    cache_bal(&db, vec![post_execution]);
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

    cache_bal(&db, vec![account]);

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

    cache_bal(&db, vec![account]);

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
    cache_bal(&db, vec![account, authority_account, cleared_account]);
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
fn fork_bal_seed_rejects_invalid_structure_hash_and_bytecode() {
    let address = Address::repeat_byte(1);
    let valid = vec![complete_account(address, Bytes::new())];
    let commitment = compute_block_access_list_hash(&valid);
    assert!(validate_bal(&valid, 1, Some(commitment)).is_ok());
    assert!(validate_bal(&valid, 1, Some(B256::ZERO)).is_err());

    let duplicate_accounts = vec![valid[0].clone(), valid[0].clone()];
    assert!(validate_bal(&duplicate_accounts, 1, None).is_err());
    let invalid_index =
        AccountChanges::new(address).with_balance_change(BalanceChange::new(index(3), U256::ZERO));
    assert!(validate_bal(&vec![invalid_index], 1, None).is_err());
    let empty_changes =
        AccountChanges::new(address).with_storage_change(SlotChanges::new(U256::from(1), vec![]));
    assert!(validate_bal(&vec![empty_changes], 1, None).is_err());

    // Invalid code must discard the seed even for an incomplete account or an earlier write.
    let invalid_code = AccountChanges::new(address)
        .with_code_change(CodeChange::new(index(0), bytes!("ef0100")))
        .with_code_change(CodeChange::new(index(1), Bytes::new()));
    assert!(validate_bal(&vec![invalid_code], 1, None).is_err());
}
