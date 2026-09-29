//! Tests for BAL source eligibility, validation and fork cache insertion.

use super::*;
use alloy_eips::eip7928::{
    AccountChanges, BalanceChange, BlockAccessIndex, CodeChange, NonceChange, SlotChanges,
    StorageChange, compute_block_access_list_hash,
};
use alloy_network::{AnyHeader, AnyRpcHeader};
use alloy_primitives::bytes;
use alloy_provider::ProviderBuilder;
use alloy_rpc_types::Block;
use alloy_transport::mock::Asserter;
use foundry_evm::{backend::BlockchainDbMeta, hardfork::EthereumHardfork};
use revm::{context::BlockEnv, state::AccountInfo};

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
async fn fork_bal_skips_ineligible_sources_and_mismatched_cache() {
    let hash = B256::repeat_byte(1);
    for (mutable, multiple, wrong_block) in
        [(true, false, false), (false, true, false), (false, false, true)]
    {
        let asserter = Asserter::new();
        let mut config = fork_config(asserter.clone(), hash);
        config.state_is_mutable = mutable;
        if multiple {
            config.fork_urls.push("http://localhost:8546".to_string());
        }
        asserter.push_success(&serde_json::Value::Null);
        let db = database(if wrong_block { B256::ZERO } else { hash });

        config.prefill_cache(&db).await;

        assert_eq!(asserter.read_q().len(), 1, "ineligible sources must not issue prefill RPCs");
        assert!(db.accounts().read().is_empty());
        assert!(db.storage().read().is_empty());
    }
}

#[test]
fn fork_bal_uses_source_rules() {
    let ethereum = Some(NetworkVariant::Ethereum);
    for (chain_id, network, timestamp, eligible) in [
        (1, ethereum, 1_800_000_000, true),
        (1, ethereum, 1_600_000_000, false),
        (1, None, 1_800_000_000, true),
        (11_155_111, ethereum, 1_800_000_000, true),
        (17_000, ethereum, 1_800_000_000, true),
        (560_048, ethereum, 1_800_000_000, true),
        (31_337, ethereum, 1_800_000_000, false),
        (42_161, ethereum, 1_800_000_000, false),
        (1, Some(NetworkVariant::Tempo), 1_800_000_000, false),
    ] {
        let mut config = fork_config(Asserter::new(), B256::ZERO);
        config.endpoint_identity.source_chain_id = chain_id;
        config.endpoint_identity.network = network;
        config.timestamp = timestamp;
        // Local execution overrides must not make an ineligible source eligible.
        config.hardfork = Some(EthereumHardfork::Amsterdam.into());
        config.execution_chain_id = 31_337;
        config.override_chain_id = Some(31_337);
        config.endpoint_identity.execution_chain_id = 31_337;
        assert_eq!(config.bal_eligible(), eligible, "{chain_id}, {network:?}, {timestamp}");
    }
}

fn rpc_error(asserter: &Asserter, code: i64) {
    asserter.push_failure(
        serde_json::from_value(serde_json::json!({"code": code, "message": "unavailable"}))
            .unwrap(),
    );
}

#[tokio::test]
async fn fork_bal_prefill_validates_before_caching() {
    let hash = B256::repeat_byte(1);
    let address = Address::repeat_byte(1);
    for (bad_commitment, wrong_hash, identity_code) in [
        (false, false, -32601),
        (true, false, -32601),
        (false, true, -32601),
        (false, false, -32603),
        (false, false, 0),
    ] {
        let asserter = Asserter::new();
        let config = fork_config(asserter.clone(), hash);
        let bal = vec![complete_account(address, bytes!("6000")).with_storage_change(
            SlotChanges::new(U256::ONE, vec![StorageChange::new(index(1), U256::ONE)]),
        )];
        let block = AnyRpcBlock::new(
            Block::new(
                AnyRpcHeader {
                    hash: if wrong_hash { B256::ZERO } else { hash },
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
        asserter.push_success(&bal);
        asserter.push_success(&block);
        if !bad_commitment && !wrong_hash {
            if identity_code == 0 {
                asserter.push_success(&serde_json::json!({}));
            } else {
                rpc_error(&asserter, identity_code);
            }
        }
        let db = database(hash);

        config.prefill_cache(&db).await;

        assert!(asserter.read_q().is_empty());
        if bad_commitment || wrong_hash || identity_code != -32601 {
            assert!(db.accounts().read().is_empty());
            assert!(db.storage().read().is_empty());
        } else {
            assert_eq!(db.storage().read()[&address][&U256::ONE], U256::ONE);
            let accounts = db.accounts().read();
            let account = &accounts[&address];
            assert_eq!(account.balance, U256::from(42));
            assert_eq!(account.nonce, 3);
            assert_eq!(account.code.as_ref().unwrap().original_bytes(), bytes!("6000"));
        }
    }
}

#[tokio::test]
async fn fork_bal_unavailable_does_not_fetch_block_or_change_cache() {
    let hash = B256::repeat_byte(1);
    for error in [None, Some(-32603), Some(-32601)] {
        let asserter = Asserter::new();
        let config = fork_config(asserter.clone(), hash);
        if let Some(code) = error {
            rpc_error(&asserter, code);
        }
        if error.is_none() {
            asserter.push_success(&serde_json::Value::Null);
        }
        // A following response must stay untouched when no BAL is available.
        asserter.push_success(&serde_json::Value::Null);
        let db = database(hash);
        db.storage().write().entry(Address::ZERO).or_default().insert(U256::ZERO, U256::ONE);
        let storage = db.storage().read().clone();

        config.prefill_cache(&db).await;

        assert_eq!(asserter.read_q().len(), 1);
        assert_eq!(*db.storage().read(), storage);
        assert!(db.accounts().read().is_empty());
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

    cache_bal(db.db(), vec![account]);

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
    cache_bal(db.db(), vec![post_execution]);
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

    cache_bal(db.db(), vec![account]);

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

    cache_bal(db.db(), vec![account]);

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
fn fork_bal_seed_keeps_final_account_code() {
    let address = Address::repeat_byte(1);
    let delegation = bytes!("ef01000000000000000000000000000000000000000042");
    for code in [bytes!("6000"), delegation, Bytes::new()] {
        let db = database(B256::ZERO);
        let account = complete_account(address, bytes!("6001"))
            .with_code_change(CodeChange::new(index(2), code.clone()));

        cache_bal(db.db(), vec![account]);

        let accounts = db.accounts().read();
        let account = &accounts[&address];
        assert_eq!(account.balance, U256::from(42));
        assert_eq!(account.nonce, 3);
        assert_eq!(account.code_hash, alloy_primitives::keccak256(&code));
        assert_eq!(account.code.as_ref().unwrap().original_bytes(), code);
        assert_eq!(
            account.code.as_ref().unwrap().is_eip7702(),
            code.starts_with(&[0xef, 0x01, 0x00])
        );
    }
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
    // Invalid code must discard the seed even for an incomplete account or an earlier write.
    let invalid_code = AccountChanges::new(address)
        .with_code_change(CodeChange::new(index(0), bytes!("ef0100")))
        .with_code_change(CodeChange::new(index(1), Bytes::new()));
    assert!(validate_bal(&vec![invalid_code], 1, None).is_err());
}
