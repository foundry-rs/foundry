//! `cast run` block access list (BAL) coverage using locally mined transactions. Anvil serves the
//! BALs of the Amsterdam blocks it mines; earlier hardforks use canned BAL responses instead.

use alloy_consensus::BlockHeader;
use alloy_eips::{
    BlockId,
    eip4788::BEACON_ROOTS_ADDRESS,
    eip7928::{
        AccountChanges, BalanceChange, BlockAccessIndex, BlockAccessList, NonceChange, SlotChanges,
        StorageChange, compute_block_access_list_hash,
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
    rpc::{
        spawn_rpc_proxy_canned_method, spawn_rpc_proxy_mapping_method,
        spawn_rpc_proxy_method_not_found_before,
    },
    snapbox::cmd::OutputAssert,
    str,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, process::Output, sync::atomic::Ordering};

const BAL_METHOD: &str = "eth_getBlockAccessList";

/// Three counter transactions in one block, plus the BAL the block would have produced.
struct Fixture {
    handle: NodeHandle,
    block_number: u64,
    transactions: [B256; 3],
    gas: [u64; 3],
    bal: BlockAccessList,
}

impl Fixture {
    async fn new(hardfork: EthereumHardfork) -> Self {
        let (api, handle) =
            anvil::spawn(NodeConfig::test().with_hardfork(Some(hardfork.into()))).await;
        let provider = handle.http_provider();
        let sender = handle.dev_wallets().next().unwrap().address();
        let mut nonce = provider.get_transaction_count(sender).await.unwrap();
        let target = sender.create(nonce);

        // System calls increment slot zero; ordinary calls return it. Unlike the standard
        // contract, this makes accidentally executing the system operation twice observable.
        api.anvil_set_code(
            BEACON_ROOTS_ADDRESS,
            hex!("3373fffffffffffffffffffffffffffffffffffffffe1460255760005460005260206000f35b60005460010160005500").into(),
        )
        .await
        .unwrap();
        // Increment slot zero, then log and return both its value and the beacon counter.
        let runtime = hex::decode(format!(
            "60005460010180600055600052602060206000600073{BEACON_ROOTS_ADDRESS:x}5afa5060406000a060406000f3"
        ))
        .unwrap();
        let init = hex::decode(format!(
            "600160005560{:02x}601160003960{:02x}6000f3{}",
            runtime.len(),
            runtime.len(),
            hex::encode(&runtime),
        ))
        .unwrap();
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
                .to(target)
                .nonce(nonce + index as u64)
                .gas_limit(1_000_000);
            *hash = *provider.send_transaction(tx.into()).await.unwrap().tx_hash();
        }
        api.mine_one().await.unwrap();
        let block = provider.get_block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
        assert_eq!(block.transactions().hashes().collect::<Vec<_>>(), transactions);
        assert_eq!(provider.get_storage_at(target, U256::ZERO).await.unwrap(), U256::from(4));

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
                storage_changes: vec![SlotChanges::new(
                    U256::ZERO,
                    (1..=3)
                        .map(|index| {
                            StorageChange::new(BlockAccessIndex::new(index), U256::from(index + 1))
                        })
                        .collect(),
                )],
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
        Self {
            handle,
            block_number: block.header().number(),
            transactions,
            gas,
            bal: accounts.into_values().collect(),
        }
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
    let fixture = Fixture::new(EthereumHardfork::Cancun).await;
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
        // Same trace, return data, logs, storage changes and gas, without the prefix replay.
        cmd.with_no_redact().assert_success().stdout_eq(replay.stdout).stderr_eq(str![[r#"
[..] TRACE cast::cmd::run: reading prestate from block access list, skipping block replay
[..] TRACE cast::cmd::run: executing call transaction tx=[..]
[..] TRACE cast::cmd::run: completed execution tx_hash=[..]

"#]]);
    }
    assert_eq!(calls.load(Ordering::Relaxed), 3);
});

casttest!(cast_run_fork_bal_respects_no_bal_quick_prestate_and_remote_modes, async |_prj, cmd| {
    let fixture = Fixture::new(EthereumHardfork::Cancun).await;
    let hash = fixture.transactions[2];
    let replay = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &[]);
    let (endpoint, calls) = spawn_rpc_proxy_canned_method(
        fixture.handle.http_endpoint(),
        BAL_METHOD,
        json!(fixture.bal),
    )
    .await;
    for flags in
        [&["--no-bal"][..], &["--quick"], &["--prestate-tracer"], &["--debug-trace-transaction"]]
    {
        let expected = run(&mut cmd, hash, &fixture.handle.http_endpoint(), flags);
        let actual = run(&mut cmd, hash, &endpoint, flags);
        OutputAssert::new(actual).stdout_eq(expected.stdout).stderr_eq(expected.stderr);
        assert_eq!(calls.load(Ordering::Relaxed), 0, "flags: {flags:?}");
    }

    // A failed explicitly requested prestate tracer tries the BAL next, before ordinary replay.
    let (endpoint, prestate_calls) =
        spawn_rpc_proxy_canned_method(endpoint, "debug_traceTransaction", Value::Null).await;
    let output = run(&mut cmd, hash, &endpoint, &["--prestate-tracer"]);
    OutputAssert::new(output).stdout_eq(replay.stdout).stderr_eq("");
    assert_eq!(prestate_calls.load(Ordering::Relaxed), 1);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
});

casttest!(cast_run_fork_bal_unavailable_falls_back_to_replay, async |_prj, cmd| {
    let fixture = Fixture::new(EthereumHardfork::Cancun).await;
    let hash = fixture.transactions[2];
    let replay = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &[]);

    let (endpoint, calls) =
        spawn_rpc_proxy_canned_method(fixture.handle.http_endpoint(), BAL_METHOD, Value::Null)
            .await;
    let output = run(&mut cmd, hash, &endpoint, &[]);
    OutputAssert::new(output)
        .stdout_eq(replay.stdout.clone())
        .stderr_eq("Executing previous transactions from the block.\n");
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

casttest!(cast_run_fork_bal_is_checked_against_the_header_hash, async |_prj, cmd| {
    let fixture = Fixture::new(EthereumHardfork::Cancun).await;
    let hash = fixture.transactions[2];
    let replay = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &[]);
    let block_number = json!(format!("{:#x}", fixture.block_number));
    // Anvil headers do not commit to a BAL yet, so present the block the way an Amsterdam node
    // would: once with the hash of the served list and once with a foreign one.
    for (header_hash, accepted) in
        [(compute_block_access_list_hash(&fixture.bal), true), (B256::ZERO, false)]
    {
        let block_number = block_number.clone();
        let endpoint = spawn_rpc_proxy_mapping_method(
            fixture.handle.http_endpoint(),
            "eth_getBlockByNumber",
            move |_, mut block| {
                if block.get("number") == Some(&block_number) {
                    block["blockAccessListHash"] = json!(header_hash);
                }
                block
            },
        )
        .await;
        let (endpoint, calls) =
            spawn_rpc_proxy_canned_method(endpoint, BAL_METHOD, json!(fixture.bal)).await;
        let output = run(&mut cmd, hash, &endpoint, &[]);
        let output = OutputAssert::new(output).stdout_eq(replay.stdout.clone());
        if accepted {
            output.stderr_eq("");
        } else {
            output.stderr_eq(str![[r#"
Warning: block access list of block [..] does not match its header, replaying the block instead
Executing previous transactions from the block.

"#]]);
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
});

casttest!(cast_run_fork_bal_uses_anvil_block_access_list, async |_prj, cmd| {
    // Amsterdam anvil serves the BAL of its own blocks, so no canned response is needed.
    let fixture = Fixture::new(EthereumHardfork::Amsterdam).await;
    let endpoint = fixture.handle.http_endpoint();
    for hash in fixture.transactions {
        let replay = run(&mut cmd, hash, &endpoint, &["--no-bal"]);
        OutputAssert::new(replay.clone())
            .stderr_eq("Executing previous transactions from the block.\n");
        run_command(&mut cmd, hash, &endpoint, &[]);
        cmd.env("RUST_LOG", "cast::cmd::run=trace");
        cmd.with_no_redact().assert_success().stdout_eq(replay.stdout).stderr_eq(str![[r#"
[..] TRACE cast::cmd::run: reading prestate from block access list, skipping block replay
[..] TRACE cast::cmd::run: executing call transaction tx=[..]
[..] TRACE cast::cmd::run: completed execution tx_hash=[..]

"#]]);
    }
});
