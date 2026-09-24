use super::*;
use alloy_eips::{
    BlockNumHash,
    eip7928::{
        AccountChanges, BalanceChange, BlockAccessIndex, CodeChange, NonceChange, SlotChanges,
        StorageChange,
    },
};
use alloy_network::{AnyHeader, AnyRpcHeader};
use alloy_primitives::{Address, B256, Bytes, U256, bytes};
use alloy_provider::{ProviderBuilder, mock::Asserter};
use alloy_rpc_types::{Block, BlockTransactions};
use foundry_config::FoundryHardfork;
use foundry_evm_networks::NetworkVariant;

fn context() -> ForkContext {
    ForkContext {
        execution_chain_id: 1,
        source_chain_id: 1,
        network: NetworkVariant::Ethereum,
        network_profile: NetworkConfigs::default(),
        block_number: 20_000_000,
        hardfork: None,
        instance_id: None,
        source_fork_block_number: None,
        source_fork_block_hash: None,
    }
}

fn resolved(context: ForkContext) -> ResolvedFork {
    ResolvedFork::new(
        "http://localhost:8545",
        None,
        None,
        Some(context.block_number),
        BlockNumHash::new(context.block_number, B256::repeat_byte(1)),
        context,
    )
}

fn block(resolved: &ResolvedFork, transactions: usize) -> AnyRpcBlock {
    let header =
        AnyHeader { number: resolved.number(), timestamp: 1_710_338_135, ..Default::default() };
    AnyRpcBlock::new(
        Block::new(
            AnyRpcHeader::from_sealed(header.seal(resolved.hash())),
            BlockTransactions::Hashes(vec![B256::ZERO; transactions]),
        )
        .into(),
    )
}

fn index(index: u64) -> BlockAccessIndex {
    BlockAccessIndex::new(index)
}

fn complete_account(address: Address, code: Bytes) -> AccountChanges {
    AccountChanges::new(address)
        .with_balance_change(BalanceChange::new(index(1), U256::from(42)))
        .with_nonce_change(NonceChange::new(index(1), 3))
        .with_code_change(CodeChange::new(index(1), code))
}

#[test]
fn fork_bal_cache_keeps_final_zero_and_system_writes() {
    let resolved = resolved(context());
    let block = block(&resolved, 2);
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
    let db = MemDb::default();

    let bal = vec![account];
    validate(&bal, &block).unwrap();
    cache(&db, bal);

    let storage = db.storage.read();
    assert_eq!(storage[&address][&slot], U256::ZERO);
    assert_eq!(storage[&address][&system_slot], U256::from(5));
    assert!(db.accounts.read().is_empty());
}

#[test]
fn fork_bal_cache_leaves_partial_accounts_and_reads_unknown() {
    let resolved = resolved(context());
    let address = Address::repeat_byte(1);
    let account = AccountChanges::new(address)
        .with_balance_change(BalanceChange::new(index(1), U256::from(42)))
        .with_storage_read(U256::from(2));
    let db = MemDb::default();

    let bal = vec![account];
    validate(&bal, &block(&resolved, 1)).unwrap();
    cache(&db, bal);

    assert!(db.accounts.read().is_empty());
    assert!(db.storage.read().is_empty());
}

#[test]
fn fork_bal_cache_preserves_cached_values_and_merges_slots() {
    let resolved = resolved(context());
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
    let db = MemDb::default();
    let cached_account = AccountInfo { balance: U256::from(99), ..Default::default() };
    db.accounts.write().insert(address, cached_account.clone());
    db.storage.write().insert(
        address,
        [(U256::from(1), U256::from(101)), (U256::from(3), U256::from(303))].into_iter().collect(),
    );

    let bal = vec![account];
    validate(&bal, &block(&resolved, 1)).unwrap();
    cache(&db, bal);

    assert_eq!(db.accounts.read()[&address], cached_account);
    assert_eq!(
        db.storage.read()[&address],
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
fn fork_bal_cache_preserves_delegation_code_and_final_clearing() {
    let resolved = resolved(context());
    let authority = Address::repeat_byte(1);
    let cleared = Address::repeat_byte(2);
    let delegation = bytes!("ef01000000000000000000000000000000000000000042");
    let cleared_account = complete_account(cleared, delegation.clone())
        .with_balance_change(BalanceChange::new(index(2), U256::ZERO))
        .with_nonce_change(NonceChange::new(index(2), 0))
        .with_code_change(CodeChange::new(index(2), Bytes::new()));
    let db = MemDb::default();

    let bal = vec![complete_account(authority, delegation.clone()), cleared_account];
    validate(&bal, &block(&resolved, 2)).unwrap();
    cache(&db, bal);

    let accounts = db.accounts.read();
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
fn fork_bal_validation_rejects_invalid_structure_hash_and_bytecode() {
    let resolved = resolved(context());
    let address = Address::repeat_byte(1);
    let valid = vec![complete_account(address, Bytes::new())];
    let mut block = block(&resolved, 1);
    block.header.block_access_list_hash = Some(compute_block_access_list_hash(&valid));
    assert!(validate(&valid, &block).is_ok());
    block.header.block_access_list_hash = Some(B256::ZERO);
    assert!(validate(&valid, &block).is_err());
    block.header.block_access_list_hash = None;

    let duplicate_accounts = vec![valid[0].clone(), valid[0].clone()];
    assert!(validate(&duplicate_accounts, &block).is_err());
    let invalid_index =
        AccountChanges::new(address).with_balance_change(BalanceChange::new(index(3), U256::ZERO));
    assert!(validate(&vec![invalid_index], &block).is_err());
    let empty_changes =
        AccountChanges::new(address).with_storage_change(SlotChanges::new(U256::ONE, vec![]));
    assert!(validate(&vec![empty_changes], &block).is_err());

    // Earlier invalid code rejects the whole BAL, including otherwise valid storage.
    let invalid_code = AccountChanges::new(Address::repeat_byte(2))
        .with_storage_change(SlotChanges::new(
            U256::ONE,
            vec![StorageChange::new(index(0), U256::ONE)],
        ))
        .with_code_change(CodeChange::new(index(0), bytes!("ef0100")))
        .with_code_change(CodeChange::new(index(1), Bytes::new()));
    assert!(validate(&vec![valid[0].clone(), invalid_code], &block).is_err());
}

#[test]
fn fork_bal_cache_accepts_empty_block_with_post_execution_write() {
    let resolved = resolved(context());
    let address = Address::repeat_byte(1);
    let account = AccountChanges::new(address).with_storage_change(SlotChanges::new(
        U256::ONE,
        vec![StorageChange::new(index(1), U256::from(42))],
    ));
    let db = MemDb::default();

    let bal = vec![account];
    validate(&bal, &block(&resolved, 0)).unwrap();
    cache(&db, bal);

    assert_eq!(db.storage.read()[&address][&U256::ONE], U256::from(42));
}

#[tokio::test]
async fn fork_bal_prepare_skips_mutable_and_custom_sources_without_requests() {
    let ordinary = context();
    let contexts = [
        ForkContext { source_chain_id: 31337, ..ordinary },
        ForkContext { network: NetworkVariant::Tempo, ..ordinary },
        ForkContext { network_profile: NetworkConfigs::with_celo(), ..ordinary },
        ForkContext { network_profile: NetworkConfigs::with_tempo(), ..ordinary },
        ForkContext {
            hardfork: Some(FoundryHardfork::Ethereum(EthereumHardfork::Cancun)),
            ..ordinary
        },
        ForkContext { instance_id: Some(B256::ZERO), ..ordinary },
        ForkContext { source_fork_block_number: Some(1), ..ordinary },
        ForkContext { source_fork_block_hash: Some(B256::ZERO), ..ordinary },
    ];
    for context in contexts {
        let resolved = resolved(context);
        let asserter = Asserter::new();
        asserter.push_success(&BlockAccessList::new());
        let provider =
            ProviderBuilder::<_, _, AnyNetwork>::default().connect_mocked_client(asserter.clone());
        assert!(prepare(&provider, &resolved, &block(&resolved, 0)).await.is_none());
        assert_eq!(asserter.read_q().len(), 1);
    }
}

#[tokio::test]
async fn fork_bal_prepare_checks_parent_identity_and_source_timestamp_before_bal() {
    let resolved = resolved(context());
    let valid = block(&resolved, 1);
    for field in ["hash", "number", "timestamp"] {
        let mut block = valid.clone();
        match field {
            "hash" => block.header.hash = B256::repeat_byte(2),
            "number" => block.header.number += 1,
            "timestamp" => block.header.timestamp -= 1,
            _ => unreachable!(),
        }
        let asserter = Asserter::new();
        asserter.push_success(&BlockAccessList::new());
        let provider =
            ProviderBuilder::<_, _, AnyNetwork>::default().connect_mocked_client(asserter.clone());

        assert!(prepare(&provider, &resolved, &block).await.is_none());
        assert_eq!(asserter.read_q().len(), 1);
    }
}

#[tokio::test]
async fn fork_bal_prepare_reuses_block_with_execution_chain_override() {
    for chain in [NamedChain::Mainnet, NamedChain::Sepolia, NamedChain::Holesky, NamedChain::Hoodi]
    {
        let resolved = resolved(ForkContext {
            source_chain_id: chain as u64,
            execution_chain_id: 31337,
            ..context()
        });
        let asserter = Asserter::new();
        rpc_error(&asserter, -32601);
        asserter.push_success(&BlockAccessList::new());
        rpc_error(&asserter, -32601);
        let provider =
            ProviderBuilder::<_, _, AnyNetwork>::default().connect_mocked_client(asserter.clone());

        assert!(prepare(&provider, &resolved, &block(&resolved, 0)).await.is_some());
        assert!(asserter.read_q().is_empty());
    }
}

#[tokio::test]
async fn fork_bal_prepare_falls_back_without_caching_unavailability() {
    let resolved = resolved(context());
    let block = block(&resolved, 0);
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::<_, _, AnyNetwork>::default().connect_mocked_client(asserter.clone());
    rpc_error(&asserter, -32601);
    asserter.push_failure_msg("BAL unsupported");
    assert!(prepare(&provider, &resolved, &block).await.is_none());
    rpc_error(&asserter, -32601);
    asserter.push_success(&Option::<BlockAccessList>::None);
    assert!(prepare(&provider, &resolved, &block).await.is_none());
    rpc_error(&asserter, -32601);
    asserter.push_success(&"malformed BAL");
    assert!(prepare(&provider, &resolved, &block).await.is_none());

    rpc_error(&asserter, -32601);
    asserter.push_success(&BlockAccessList::new());
    rpc_error(&asserter, -32601);
    assert!(prepare(&provider, &resolved, &block).await.is_some());
    assert!(asserter.read_q().is_empty());
}

fn rpc_error(asserter: &Asserter, code: i64) {
    asserter.push_failure(
        serde_json::from_value(serde_json::json!({"code": code, "message": "unavailable"}))
            .unwrap(),
    );
}

#[tokio::test]
async fn fork_bal_prepare_requires_immutable_source_before_and_after_bal() {
    let resolved = resolved(context());
    let block = block(&resolved, 0);
    for final_probe in [false, true] {
        for error in [None, Some(-32603)] {
            let asserter = Asserter::new();
            if final_probe {
                rpc_error(&asserter, -32601);
                asserter.push_success(&BlockAccessList::new());
            }
            if let Some(code) = error {
                rpc_error(&asserter, code);
            } else {
                // Even an unrecognized successful response identifies a potentially mutable node.
                asserter.push_success(&serde_json::json!({}));
            }
            asserter.push_success(&BlockAccessList::new());
            let provider = ProviderBuilder::<_, _, AnyNetwork>::default()
                .connect_mocked_client(asserter.clone());

            assert!(prepare(&provider, &resolved, &block).await.is_none());
            assert_eq!(asserter.read_q().len(), 1, "requests continued after an uncertain source");
        }
    }
}

#[tokio::test]
async fn fork_bal_prepare_uses_legacy_rpc_only_for_method_not_found() {
    let resolved = resolved(context());
    let block = block(&resolved, 0);
    for code in [-32601, -32603] {
        let asserter = Asserter::new();
        rpc_error(&asserter, -32601);
        rpc_error(&asserter, code);
        asserter.push_success(&BlockAccessList::new());
        rpc_error(&asserter, -32601);
        let provider =
            ProviderBuilder::<_, _, AnyNetwork>::default().connect_mocked_client(asserter.clone());

        let bal = prepare(&provider, &resolved, &block).await;
        assert_eq!(bal.is_some(), code == -32601);
        assert_eq!(asserter.read_q().len(), if code == -32601 { 0 } else { 2 });
    }
}
