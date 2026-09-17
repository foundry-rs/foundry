//! BAL replay coverage using locally mined transactions and canned BAL responses.

use alloy_consensus::BlockHeader;
use alloy_eips::{
    BlockId,
    eip4788::BEACON_ROOTS_ADDRESS,
    eip7928::{
        AccountChanges, BalanceChange, BlockAccessIndex, BlockAccessList, CodeChange, NonceChange,
        SlotChanges, StorageChange,
    },
};
use alloy_hardforks::EthereumHardfork;
use alloy_network::{
    BlockResponse, ReceiptResponse, TransactionBuilder, primitives::HeaderResponse,
};
use alloy_primitives::{B256, Bytes, U256, hex};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, TransactionRequest};
use anvil::{NodeConfig, NodeHandle};
use foundry_test_utils::{
    TestCommand,
    rpc::{spawn_rpc_proxy_canned_method, spawn_rpc_proxy_method_not_found_before},
    snapbox::cmd::OutputAssert,
    str,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, process::Output, sync::atomic::Ordering};

const BAL_METHOD: &str = "eth_getBlockAccessListByBlockHash";

/// Three counter transactions in one block, including a system-operation read.
struct Fixture {
    handle: NodeHandle,
    transactions: [B256; 3],
    gas: [u64; 3],
    bal: BlockAccessList,
}

impl Fixture {
    async fn new(parent_storage: bool) -> Self {
        let (api, handle) =
            anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into())))
                .await;
        let provider = handle.http_provider();
        let sender = handle.dev_wallets().next().unwrap().address();
        let mut nonce = provider.get_transaction_count(sender).await.unwrap();
        let target = sender.create(nonce);
        if parent_storage {
            // CREATE clears storage even when a custom parent account already has slots.
            api.anvil_set_balance(target, U256::from(1)).await.unwrap();
            api.anvil_set_storage_at(target, U256::from(1), U256::from(77).into()).await.unwrap();
        }

        // System calls increment slot zero; ordinary calls return it. Unlike the standard
        // contract, this makes accidentally executing the system operation twice observable.
        api.anvil_set_code(
            BEACON_ROOTS_ADDRESS,
            hex!("3373fffffffffffffffffffffffffffffffffffffffe1460255760005460005260206000f35b60005460010160005500").into(),
        )
        .await
        .unwrap();
        // Increment slot zero, then log and return both its value and the beacon counter.
        // No compiler or public selector service is needed for this fixture.
        let extra_read = if parent_storage { "600154604052" } else { "" };
        let size = if parent_storage { "60" } else { "40" };
        let runtime = hex::decode(format!(
            "60005460010180600055600052602060206000600073{BEACON_ROOTS_ADDRESS:x}5afa50{extra_read}60{size}6000a060{size}6000f3"
        ))
        .unwrap();
        let init = hex::decode(format!(
            "600160005560{:02x}601160003960{:02x}6000f3{}",
            runtime.len(),
            runtime.len(),
            hex::encode(&runtime),
        ))
        .unwrap();
        if !parent_storage {
            let receipt = provider
                .send_transaction(
                    TransactionRequest::default()
                        .from(sender)
                        .with_deploy_code(Bytes::copy_from_slice(&init))
                        .into(),
                )
                .await
                .unwrap()
                .get_receipt()
                .await
                .unwrap();
            assert!(receipt.status());
            nonce += 1;
        }
        api.mine_one().await.unwrap();
        let parent = provider.get_block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
        let parent_hash = parent.header().hash();
        let system_value = provider.get_storage_at(BEACON_ROOTS_ADDRESS, U256::ZERO).await.unwrap()
            + U256::from(1);

        api.anvil_set_auto_mine(false).await.unwrap();
        let mut transactions = [B256::ZERO; 3];
        for (index, hash) in transactions.iter_mut().enumerate() {
            let tx = TransactionRequest::default()
                .from(sender)
                .nonce(nonce + index as u64)
                .gas_limit(1_000_000);
            let tx = if parent_storage && index == 0 {
                tx.with_deploy_code(Bytes::copy_from_slice(&init))
            } else {
                tx.to(target)
            };
            *hash = *provider.send_transaction(tx.into()).await.unwrap().tx_hash();
        }
        api.mine_one().await.unwrap();
        let block = provider.get_block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
        assert_eq!(block.transactions().hashes().collect::<Vec<_>>(), transactions);
        assert_eq!(
            provider.get_storage_at(target, U256::ZERO).await.unwrap(),
            U256::from(if parent_storage { 3 } else { 4 }),
        );

        let beneficiary = block.header().beneficiary();
        let mut sender_balance =
            provider.get_balance(sender).block_id(BlockId::hash(parent_hash)).await.unwrap();
        let mut beneficiary_balance =
            provider.get_balance(beneficiary).block_id(BlockId::hash(parent_hash)).await.unwrap();
        let mut accounts = BTreeMap::from([
            (sender, AccountChanges::new(sender)),
            (beneficiary, AccountChanges::new(beneficiary)),
        ]);
        let mut gas = [0; 3];
        for (index, hash) in transactions.iter().enumerate() {
            let receipt = provider.get_transaction_receipt(*hash).await.unwrap().unwrap();
            assert!(receipt.status());
            assert_eq!(receipt.transaction_index(), Some(index as u64));
            gas[index] = receipt.gas_used();
            let bal_index = BlockAccessIndex::new(index as u64 + 1);
            sender_balance -=
                U256::from(receipt.gas_used()) * U256::from(receipt.effective_gas_price());
            let changes = accounts.get_mut(&sender).unwrap();
            changes.balance_changes.push(BalanceChange::new(bal_index, sender_balance));
            changes.nonce_changes.push(NonceChange::new(bal_index, nonce + index as u64 + 1));
            beneficiary_balance += U256::from(receipt.gas_used())
                * U256::from(
                    receipt.effective_gas_price()
                        - block.header().base_fee_per_gas().unwrap() as u128,
                );
            accounts
                .get_mut(&beneficiary)
                .unwrap()
                .balance_changes
                .push(BalanceChange::new(bal_index, beneficiary_balance));
        }
        accounts.insert(
            target,
            AccountChanges {
                nonce_changes: if parent_storage {
                    vec![NonceChange::new(BlockAccessIndex::new(1), 1)]
                } else {
                    vec![]
                },
                code_changes: if parent_storage {
                    vec![CodeChange::new(BlockAccessIndex::new(1), runtime.into())]
                } else {
                    vec![]
                },
                storage_changes: vec![SlotChanges::new(
                    U256::ZERO,
                    (1..=3)
                        .map(|index| {
                            StorageChange::new(
                                BlockAccessIndex::new(index),
                                U256::from(index + u64::from(!parent_storage)),
                            )
                        })
                        .collect(),
                )],
                storage_reads: if parent_storage { vec![U256::from(1)] } else { vec![] },
                ..AccountChanges::new(target)
            },
        );
        accounts.insert(
            BEACON_ROOTS_ADDRESS,
            AccountChanges {
                storage_changes: vec![SlotChanges::new(
                    U256::ZERO,
                    vec![StorageChange::new(BlockAccessIndex::new(0), system_value)],
                )],
                ..AccountChanges::new(BEACON_ROOTS_ADDRESS)
            },
        );
        Self { handle, transactions, gas, bal: accounts.into_values().collect() }
    }
}

fn run_command<'a>(
    cmd: &'a mut TestCommand,
    hash: B256,
    endpoint: &str,
    flags: &[&str],
) -> &'a mut TestCommand {
    let project_dir = cmd.cmd().get_current_dir().unwrap().to_path_buf();
    cmd.cast_fuse().current_dir(project_dir);
    cmd.env("FOUNDRY_NO_STORAGE_CACHING", "true");
    cmd.env("FOUNDRY_DISABLE_NIGHTLY_WARNING", "true");
    cmd.env("RUST_LOG", "off");
    cmd.args([
        "run",
        &hash.to_string(),
        "--rpc-url",
        endpoint,
        "--disable-external-identification",
        "-vvvvv",
    ])
    .args(flags)
}

fn run(cmd: &mut TestCommand, hash: B256, endpoint: &str, flags: &[&str]) -> Output {
    run_command(cmd, hash, endpoint, flags).assert_success().get_output().clone()
}

casttest!(cast_run_fork_bal_matches_replay_at_every_position, async |_prj, cmd| {
    let fixture = Fixture::new(false).await;
    // Anvil currently returns null for locally mined BALs, so supply only that RPC response.
    let (endpoint, calls) = spawn_rpc_proxy_canned_method(
        fixture.handle.http_endpoint(),
        BAL_METHOD,
        json!(fixture.bal),
    )
    .await;
    for (index, hash) in fixture.transactions.iter().enumerate() {
        let replay = run(&mut cmd, *hash, &fixture.handle.http_endpoint(), &[]);
        OutputAssert::new(replay.clone()).stdout_eq(format!(
            "Traces:\n...\nTransaction successfully executed.\nGas used: {}\n",
            fixture.gas[index],
        ));
        run_command(&mut cmd, *hash, &endpoint, &[]);
        cmd.env("RUST_LOG", "cast::cmd::run=trace");
        // Compare full trace, return data, logs, storage changes and gas; require BAL selection
        // and the absence of the prefix-replay message so fallback cannot pass this test.
        cmd.with_no_redact().assert_success().stdout_eq(replay.stdout).stderr_eq(str![[r#"
[..] TRACE cast::cmd::run: BAL prestate applied successfully, skipping block replay
[..] TRACE cast::cmd::run: executing [..] transaction tx=[..]

"#]]);
    }
    assert_eq!(calls.load(Ordering::Relaxed), 3);
});

casttest!(cast_run_fork_bal_rejects_invalid_transaction_index, async |_prj, cmd| {
    let fixture = Fixture::new(false).await;
    let hash = fixture.transactions[2];
    let replay = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &[]);
    let mut transaction = serde_json::to_value(
        fixture.handle.http_provider().get_transaction_by_hash(hash).await.unwrap().unwrap(),
    )
    .unwrap();
    let (bal_endpoint, calls) = spawn_rpc_proxy_canned_method(
        fixture.handle.http_endpoint(),
        BAL_METHOD,
        json!(fixture.bal),
    )
    .await;

    // Missing, out-of-range and mismatched indices must fall back to locating the target by hash.
    for (attempt, index) in [Value::Null, json!("0x3"), json!("0x0")].into_iter().enumerate() {
        transaction["transactionIndex"] = index;
        let (endpoint, _) = spawn_rpc_proxy_canned_method(
            bal_endpoint.clone(),
            "eth_getTransactionByHash",
            transaction.clone(),
        )
        .await;
        let output = run(&mut cmd, hash, &endpoint, &[]);
        OutputAssert::new(output)
            .stdout_eq(replay.stdout.clone())
            .stderr_eq("Executing previous transactions from the block.\n");
        assert_eq!(calls.load(Ordering::Relaxed), attempt + 1);
    }
});

casttest!(cast_run_fork_bal_failures_restore_clean_replay, async |_prj, cmd| {
    let fixture = Fixture::new(false).await;
    let hash = fixture.transactions[2];
    let replay = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &[]);
    let mut missing_slot = fixture.bal.clone();
    missing_slot
        .iter_mut()
        .find(|account| account.address == BEACON_ROOTS_ADDRESS)
        .unwrap()
        .storage_changes
        .clear();
    let mut duplicate_account = fixture.bal.clone();
    duplicate_account.insert(0, duplicate_account[0].clone());
    for bal in [Value::Null, json!({"unexpected": true}), json!(duplicate_account)] {
        let (endpoint, calls) =
            spawn_rpc_proxy_canned_method(fixture.handle.http_endpoint(), BAL_METHOD, bal).await;
        let output = run(&mut cmd, hash, &endpoint, &[]);
        OutputAssert::new(output)
            .stdout_eq(replay.stdout.clone())
            .stderr_eq("Executing previous transactions from the block.\n");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
    // The missing beacon slot is read after the target writes its own storage. Require an
    // execution failure, then compare the retry to clean replay to catch leaked partial writes.
    let (endpoint, calls) = spawn_rpc_proxy_canned_method(
        fixture.handle.http_endpoint(),
        BAL_METHOD,
        json!(missing_slot),
    )
    .await;
    run_command(&mut cmd, hash, &endpoint, &[]);
    cmd.env("RUST_LOG", "cast::cmd::run=trace");
    cmd.with_no_redact().assert_success().stdout_eq(replay.stdout.clone()).stderr_eq(str![[r#"
[..] TRACE cast::cmd::run: BAL execution unavailable, falling back to block replay err=[..]
Executing previous transactions from the block.
[..] TRACE cast::cmd::run: preparing previous call transaction tx=[..]
[..] TRACE cast::cmd::run: preparing previous call transaction tx=[..]
[..] TRACE cast::cmd::run: executing call transaction tx=[..]
[..] TRACE cast::cmd::run: completed block replay tx_hash=[..]

"#]]);
    assert_eq!(calls.load(Ordering::Relaxed), 1);

    let endpoint = spawn_rpc_proxy_method_not_found_before(
        fixture.handle.http_endpoint(),
        BAL_METHOD,
        usize::MAX,
    )
    .await;
    let output = run(&mut cmd, hash, &endpoint, &[]);
    OutputAssert::new(output)
        .stdout_eq(replay.stdout)
        .stderr_eq("Executing previous transactions from the block.\n");
});

casttest!(cast_run_fork_bal_respects_prestate_quick_and_remote_modes, async |_prj, cmd| {
    let fixture = Fixture::new(false).await;
    let hash = fixture.transactions[2];
    let replay = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &[]);
    let (endpoint, calls) = spawn_rpc_proxy_canned_method(
        fixture.handle.http_endpoint(),
        BAL_METHOD,
        json!(fixture.bal),
    )
    .await;
    for flags in [
        &["--prestate-tracer"][..],
        &["--quick", "--prestate-tracer"],
        &["--debug-trace-transaction"],
        &["--evm-version", "cancun"],
        &["--trace-printer"],
    ] {
        let expected = run(&mut cmd, hash, &fixture.handle.http_endpoint(), flags);
        let actual = run(&mut cmd, hash, &endpoint, flags);
        OutputAssert::new(actual).stdout_eq(expected.stdout).stderr_eq(expected.stderr);
        assert_eq!(calls.load(Ordering::Relaxed), 0, "flags: {flags:?}");
    }

    // A failed explicitly requested prestate tracer tries BAL next, before ordinary replay.
    let (endpoint, prestate_calls) =
        spawn_rpc_proxy_canned_method(endpoint, "debug_traceTransaction", Value::Null).await;
    let output = run(&mut cmd, hash, &endpoint, &["--prestate-tracer"]);
    OutputAssert::new(output).stdout_eq(replay.stdout).stderr_eq("");
    assert_eq!(prestate_calls.load(Ordering::Relaxed), 1);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
});

casttest!(cast_run_fork_bal_falls_back_for_creation_over_parent_storage, async |_prj, cmd| {
    let fixture = Fixture::new(true).await;
    let hash = fixture.transactions[1];
    let replay = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &[]);
    let (endpoint, calls) = spawn_rpc_proxy_canned_method(
        fixture.handle.http_endpoint(),
        BAL_METHOD,
        json!(fixture.bal),
    )
    .await;
    // The target reads storage after the prefix CREATE cleared its nonzero parent value.
    // Require the runtime guard to reject this ambiguous read and restart from clean state.
    run_command(&mut cmd, hash, &endpoint, &[]);
    cmd.env("RUST_LOG", "cast::cmd::run=trace");
    cmd.with_no_redact().assert_success().stdout_eq(replay.stdout).stderr_eq(str![[r#"
[..] TRACE cast::cmd::run: BAL execution unavailable, falling back to block replay err=[..]
Executing previous transactions from the block.
[..] TRACE cast::cmd::run: preparing previous create transaction tx=[..]
[..] TRACE cast::cmd::run: executing call transaction tx=[..]
[..] TRACE cast::cmd::run: completed block replay tx_hash=[..]

"#]]);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
});
