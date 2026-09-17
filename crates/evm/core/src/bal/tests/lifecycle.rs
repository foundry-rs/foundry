//! Differential lifecycle coverage using BALs collected from actual EVM execution.

use crate::bal::{PreparedPrestate, prepare_prestate};
use alloy_consensus::proofs::storage_root_unhashed;
use alloy_eip7928::{BlockAccessIndex, BlockAccessList};
use alloy_eips::eip7702::{Authorization, RecoveredAuthority, RecoveredAuthorization};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, bytes};
use alloy_provider::{ProviderBuilder, mock::Asserter};
use alloy_rpc_types::EIP1186AccountProofResponse;
use revm::{
    Context, DatabaseCommit, DatabaseRef, ExecuteCommitEvm, MainBuilder, MainContext,
    context::TxEnv,
    context_interface::result::ExecutionResult,
    database::InMemoryDB,
    database_interface::bal::BalDatabase,
    primitives::hardfork::SpecId,
    state::{AccountInfo, Bytecode, EvmState},
};

fn transaction(caller: Address, nonce: u64, kind: TxKind, data: Bytes) -> TxEnv {
    TxEnv::builder()
        .caller(caller)
        .nonce(nonce)
        .kind(kind)
        .data(data)
        .gas_limit(1_000_000)
        .build()
        .unwrap()
}

fn funded_parent(caller: Address) -> InMemoryDB {
    let mut parent = InMemoryDB::default();
    parent.insert_account_info(caller, AccountInfo::from_balance(U256::from(1_000_000_000)));
    parent
}

fn collect_block(
    parent: &InMemoryDB,
    transactions: &[TxEnv],
    spec: SpecId,
) -> (BlockAccessList, Vec<ExecutionResult>, Vec<InMemoryDB>) {
    let mut source = Context::mainnet()
        .modify_cfg_chained(|cfg| cfg.set_spec_and_mainnet_gas_params(spec))
        .with_db(BalDatabase::new(parent.clone()).with_bal_builder())
        .build_mainnet();
    let mut results = Vec::new();
    let mut states = Vec::new();
    for (index, transaction) in transactions.iter().enumerate() {
        states.push(source.ctx.journaled_state.database.db.clone());
        source.ctx.journaled_state.database.bal_state.bal_index =
            BlockAccessIndex::new(index as u64 + 1);
        results.push(source.transact_commit(transaction.clone()).unwrap());
    }
    states.push(source.ctx.journaled_state.database.db.clone());
    let bal = source.ctx.journaled_state.database.bal_state.take_built_alloy_bal().unwrap();
    (bal, results, states)
}

async fn verify_parent_storage(
    parent: &InMemoryDB,
    prepared: PreparedPrestate,
) -> eyre::Result<EvmState> {
    let responses = Asserter::new();
    for (address, info) in &prepared.possible_resets {
        let storage = parent.cache.accounts.get(address).into_iter().flat_map(|account| {
            account
                .storage
                .iter()
                .filter(|(_, value)| !value.is_zero())
                .map(|(slot, value)| (B256::from(slot.to_be_bytes::<32>()), *value))
        });
        responses.push_success(&EIP1186AccountProofResponse {
            address: *address,
            balance: info.balance,
            nonce: info.nonce,
            code_hash: info.code_hash,
            storage_hash: storage_root_unhashed(storage),
            ..Default::default()
        });
    }
    let provider = ProviderBuilder::new().connect_mocked_client(responses);
    prepared.verify_storage_roots(&provider, B256::with_last_byte(1)).await
}

async fn compare_boundaries(
    parent: &InMemoryDB,
    transactions: &[TxEnv],
    spec: SpecId,
    accounts: &[Address],
    slots: &[U256],
) -> (BlockAccessList, Vec<ExecutionResult>, Vec<InMemoryDB>) {
    let (bal, results, states) = collect_block(parent, transactions, spec);
    for (index, transaction) in transactions.iter().enumerate() {
        let prepared =
            prepare_prestate(parent, bal.clone(), index, transactions.len(), None, spec).unwrap();
        let overlay = verify_parent_storage(parent, prepared).await.unwrap();
        let mut restored = parent.clone();
        restored.commit(overlay);
        for &address in accounts {
            assert_eq!(
                restored.basic_ref(address).unwrap().unwrap_or_default(),
                states[index].basic_ref(address).unwrap().unwrap_or_default(),
                "account {address} before transaction {index}"
            );
            for &slot in slots {
                assert_eq!(
                    restored.storage_ref(address, slot).unwrap(),
                    states[index].storage_ref(address, slot).unwrap(),
                    "slot {slot} of {address} before transaction {index}"
                );
            }
        }
        let mut target = Context::mainnet()
            .modify_cfg_chained(|cfg| cfg.set_spec_and_mainnet_gas_params(spec))
            .with_db(restored)
            .build_mainnet();
        assert_eq!(
            target.transact_commit(transaction.clone()).unwrap(),
            results[index],
            "local execution at transaction {index}"
        );
    }
    (bal, results, states)
}

#[tokio::test]
async fn generated_bal_preserves_storage_across_delegation_install_clear_and_reinstall() {
    let caller = Address::with_last_byte(100);
    let delegate = Address::with_last_byte(101);
    let authority = Address::with_last_byte(102);
    let mut parent = funded_parent(caller);
    parent.insert_account_info(
        delegate,
        AccountInfo {
            nonce: 1,
            // Store calldata in slot zero and return it from the authority's storage.
            code: Some(Bytecode::new_raw(bytes!("60003560005560005460005260206000f3"))),
            ..Default::default()
        },
    );
    parent.insert_account_info(authority, AccountInfo { nonce: 1, ..Default::default() });
    parent.insert_account_storage(authority, U256::ZERO, U256::from(7)).unwrap();
    parent.insert_account_storage(authority, U256::from(99), U256::from(77)).unwrap();
    let transactions = [delegate, Address::ZERO, delegate]
        .into_iter()
        .enumerate()
        .map(|(index, address)| {
            let mut tx = transaction(
                caller,
                index as u64,
                TxKind::Call(authority),
                Bytes::copy_from_slice(&U256::from(index + 10).to_be_bytes::<32>()),
            );
            tx.set_recovered_authorization(vec![RecoveredAuthorization::new_unchecked(
                Authorization { chain_id: U256::ZERO, address, nonce: index as u64 + 1 },
                RecoveredAuthority::Valid(authority),
            )]);
            tx.derive_tx_type().unwrap();
            tx
        })
        .collect::<Vec<_>>();
    let (bal, results, states) = compare_boundaries(
        &parent,
        &transactions,
        SpecId::PRAGUE,
        &[caller, authority, delegate],
        &[U256::ZERO, U256::from(99)],
    )
    .await;
    assert!(results.iter().all(ExecutionResult::is_success));
    let changes = bal.iter().find(|account| account.address == authority).unwrap();
    assert_eq!(changes.code_changes.len(), 3);
    assert!(changes.code_changes[1].new_code.is_empty());
    assert!(changes.storage_changes.iter().all(|slot| slot.slot != U256::from(99)));
    for (index, expected) in [7, 10, 10, 12].into_iter().enumerate() {
        assert_eq!(states[index].storage_ref(authority, U256::ZERO).unwrap(), U256::from(expected));
        assert_eq!(states[index].storage_ref(authority, U256::from(99)).unwrap(), U256::from(77));
    }
}

#[tokio::test]
async fn generated_bal_restores_created_contract_before_storage_calls() {
    let caller = Address::with_last_byte(100);
    let created = caller.create(0);
    let parent = funded_parent(caller);
    let transactions = [
        transaction(
            caller,
            0,
            TxKind::Create,
            bytes!("6011600c60003960116000f360003560005560005460005260206000f3"),
        ),
        transaction(
            caller,
            1,
            TxKind::Call(created),
            Bytes::copy_from_slice(&U256::from(5).to_be_bytes::<32>()),
        ),
        transaction(
            caller,
            2,
            TxKind::Call(created),
            Bytes::copy_from_slice(&U256::from(9).to_be_bytes::<32>()),
        ),
    ];
    let (_, results, states) = compare_boundaries(
        &parent,
        &transactions,
        SpecId::CANCUN,
        &[caller, created],
        &[U256::ZERO, U256::from(99)],
    )
    .await;
    assert!(results.iter().all(ExecutionResult::is_success));
    assert_eq!(states[1].basic_ref(created).unwrap().unwrap().nonce, 1);
    assert_eq!(states[2].storage_ref(created, U256::ZERO).unwrap(), U256::from(5));
    assert_eq!(states[3].storage_ref(created, U256::ZERO).unwrap(), U256::from(9));
}

#[tokio::test]
async fn generated_bal_discards_reverted_creation_and_storage_writes() {
    let caller = Address::with_last_byte(100);
    let reverter = Address::with_last_byte(101);
    let created = caller.create(0);
    let mut parent = funded_parent(caller);
    parent.insert_account_info(
        reverter,
        AccountInfo {
            nonce: 1,
            code: Some(Bytecode::new_raw(bytes!("600160005560006000fd"))),
            ..Default::default()
        },
    );
    parent.insert_account_storage(reverter, U256::ZERO, U256::from(9)).unwrap();
    let transactions = [
        transaction(caller, 0, TxKind::Create, bytes!("600160005560006000fd")),
        transaction(caller, 1, TxKind::Call(reverter), Bytes::new()),
        transaction(caller, 2, TxKind::Call(created), Bytes::new()),
    ];
    let (bal, results, states) = compare_boundaries(
        &parent,
        &transactions,
        SpecId::CANCUN,
        &[caller, created, reverter],
        &[U256::ZERO, U256::from(99)],
    )
    .await;
    assert!(matches!(results[0], ExecutionResult::Revert { .. }));
    assert!(matches!(results[1], ExecutionResult::Revert { .. }));
    assert!(results[2].is_success());
    assert!(bal.iter().all(|account| account.storage_changes.is_empty()));
    for state in &states {
        assert!(state.basic_ref(created).unwrap().unwrap_or_default().is_empty());
        assert_eq!(state.storage_ref(reverter, U256::ZERO).unwrap(), U256::from(9));
    }
    assert_eq!(states[3].basic_ref(caller).unwrap().unwrap().nonce, 3);
}

#[tokio::test]
async fn generated_bal_requires_fallback_for_create_selfdestruct_with_unlisted_parent_storage() {
    let caller = Address::with_last_byte(100);
    let beneficiary = Address::with_last_byte(101);
    let created = caller.create(0);
    let slot = U256::from(99);
    // With no initial balance, creation and destruction can leave no account field changes.
    for balance in [0, 111] {
        let mut parent = funded_parent(caller);
        parent.insert_account_info(created, AccountInfo::from_balance(U256::from(balance)));
        parent.insert_account_storage(created, slot, U256::from(77)).unwrap();
        let transactions = [
            transaction(caller, 0, TxKind::Create, bytes!("6065ff")),
            transaction(caller, 1, TxKind::Call(beneficiary), Bytes::new()),
        ];
        let (bal, results, states) = collect_block(&parent, &transactions, SpecId::CANCUN);
        assert!(results.iter().all(ExecutionResult::is_success));
        assert_eq!(states[1].storage_ref(created, slot).unwrap(), U256::ZERO);
        let changes = bal.iter().find(|account| account.address == created).unwrap();
        assert!(changes.storage_changes.is_empty());
        assert!(changes.storage_reads.is_empty());
        assert!(changes.nonce_changes.is_empty());
        assert!(changes.code_changes.is_empty());
        if balance == 0 {
            assert!(changes.balance_changes.is_empty());
        }
        let prepared = prepare_prestate(&parent, bal, 1, 2, None, SpecId::CANCUN).unwrap();
        assert!(prepared.possible_resets.iter().any(|(address, _)| *address == created));
        let error = verify_parent_storage(&parent, prepared).await.unwrap_err();
        assert_eq!(error.to_string(), format!("BAL cannot exclude a storage reset for {created}"));
        assert_eq!(parent.storage_ref(created, slot).unwrap(), U256::from(77));
    }
}
