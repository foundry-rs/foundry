//! BAL replay coverage against a regular archive endpoint and deterministic Anvil fixtures.

use alloy_consensus::{BlockHeader, SignableTransaction, TxLegacy};
use alloy_eips::{
    BlockId, Encodable2718,
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
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, hex};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, TransactionRequest};
use alloy_signer::SignerSync;
use anvil::{NodeConfig, NodeHandle};
use axum::{Json, Router, routing::post};
use foundry_test_utils::{
    TestCommand, rpc::next_ws_archive_rpc_url, snapbox::cmd::OutputAssert, str, util::OutputExt,
};
use futures::future;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    process::Output,
    sync::{Arc, Mutex},
};

const BAL_METHOD: &str = "eth_getBlockAccessListByBlockHash";

/// A deployment followed by two calls in one block, with storage and beacon-root observations.
struct Fixture {
    handle: NodeHandle,
    transactions: [B256; 3],
    gas: [u64; 3],
    block_hash: B256,
    parent_hash: B256,
    bal: BlockAccessList,
}

enum FixtureMode {
    Increment,
    ChainId,
    ParentStorage(U256),
}

impl Fixture {
    async fn new() -> Self {
        Self::with_mode(FixtureMode::Increment).await
    }

    async fn with_mode(mode: FixtureMode) -> Self {
        Self::with_config(mode, NodeConfig::test()).await
    }

    async fn with_config(mode: FixtureMode, config: NodeConfig) -> Self {
        let capture_chain_id = matches!(mode, FixtureMode::ChainId);
        let parent_storage =
            if let FixtureMode::ParentStorage(value) = mode { Some(value) } else { None };
        let (api, handle) =
            anvil::spawn(config.with_hardfork(Some(EthereumHardfork::Cancun.into()))).await;
        let provider = handle.http_provider();
        let wallet = handle.dev_wallets().next().unwrap();
        let sender = wallet.address();
        let chain_id = provider.get_chain_id().await.unwrap();
        let nonce = provider.get_transaction_count(sender).await.unwrap();
        let target = sender.create(nonce);
        if let Some(value) = parent_storage {
            // Custom genesis and RPC-mutated accounts can carry storage before CREATE.
            api.anvil_set_balance(target, U256::from(1)).await.unwrap();
            api.anvil_set_storage_at(target, U256::from(1), value.into()).await.unwrap();
        }

        // System calls increment slot zero; ordinary calls return it. This makes executing the
        // block's system operation twice observable, unlike the idempotent standard contract.
        let beacon_code = hex!(
            "3373fffffffffffffffffffffffffffffffffffffffe1460255760005460005260206000f35b60005460010160005500"
        );
        api.anvil_set_code(BEACON_ROOTS_ADDRESS, beacon_code.into()).await.unwrap();
        api.mine_one().await.unwrap();
        let parent = provider.get_block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
        let parent_hash = parent.header().hash();
        let system_value = provider.get_storage_at(BEACON_ROOTS_ADDRESS, U256::ZERO).await.unwrap()
            + U256::from(1);

        // No Solidity compiler or public selector service is needed for this fixture.
        let read_storage = if capture_chain_id { "600054" } else { "60005460010180600055" };
        let read_parent_storage = if parent_storage.is_some() { "600154604052" } else { "" };
        let return_size = if parent_storage.is_some() { "60" } else { "40" };
        let runtime = hex::decode(format!(
            "{read_storage}600052602060206000600073{BEACON_ROOTS_ADDRESS:x}5afa50{read_parent_storage}60{return_size}6000a060{return_size}6000f3"
        ))
        .unwrap();
        let init_storage = if capture_chain_id { "46600055" } else { "6001600055" };
        let init = hex::decode(format!(
            "{init_storage}60{:02x}60{:02x}60003960{:02x}6000f3{}",
            runtime.len(),
            init_storage.len() / 2 + 12,
            runtime.len(),
            hex::encode(&runtime),
        ))
        .unwrap();
        api.anvil_set_auto_mine(false).await.unwrap();
        let mut transactions = [B256::ZERO; 3];
        for (index, hash) in transactions.iter_mut().enumerate() {
            if capture_chain_id {
                // Unprotected legacy transactions remain valid with a CHAINID override.
                let tx = TxLegacy {
                    nonce: nonce + index as u64,
                    gas_price: 2_000_000_000,
                    gas_limit: 1_000_000,
                    to: if index == 0 { TxKind::Create } else { target.into() },
                    input: if index == 0 { Bytes::copy_from_slice(&init) } else { Bytes::new() },
                    ..Default::default()
                };
                let signature = wallet.sign_hash_sync(&tx.signature_hash()).unwrap();
                let encoded = tx.into_signed(signature).encoded_2718();
                *hash = *provider.send_raw_transaction(&encoded).await.unwrap().tx_hash();
                continue;
            }
            let tx = TransactionRequest::default()
                .from(sender)
                .nonce(nonce + index as u64)
                .gas_limit(1_000_000);
            let tx = if index == 0 {
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
            provider.get_storage_at(BEACON_ROOTS_ADDRESS, U256::ZERO).await.unwrap(),
            system_value,
        );
        assert_eq!(
            provider.get_storage_at(target, U256::ZERO).await.unwrap(),
            U256::from(if capture_chain_id { chain_id } else { 3 }),
        );
        if parent_storage.is_some() {
            assert_eq!(provider.get_storage_at(target, U256::from(1)).await.unwrap(), U256::ZERO);
        }

        let beneficiary = block.header().beneficiary();
        let mut sender_balance =
            provider.get_balance(sender).block_id(BlockId::hash(parent_hash)).await.unwrap();
        let mut beneficiary_balance =
            provider.get_balance(beneficiary).block_id(BlockId::hash(parent_hash)).await.unwrap();
        let mut accounts = BTreeMap::new();
        accounts.insert(sender, AccountChanges::new(sender));
        accounts.insert(beneficiary, AccountChanges::new(beneficiary));
        let mut gas = [0; 3];
        for (index, hash) in transactions.iter().enumerate() {
            let receipt = provider.get_transaction_receipt(*hash).await.unwrap().unwrap();
            assert!(receipt.status());
            assert_eq!(receipt.transaction_index(), Some(index as u64));
            gas[index] = receipt.gas_used();
            let bal_index = BlockAccessIndex::new(index as u64 + 1);
            sender_balance -=
                U256::from(receipt.gas_used()) * U256::from(receipt.effective_gas_price());
            let sender_changes = accounts.get_mut(&sender).unwrap();
            sender_changes.balance_changes.push(BalanceChange::new(bal_index, sender_balance));
            sender_changes
                .nonce_changes
                .push(NonceChange::new(bal_index, nonce + index as u64 + 1));
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
                nonce_changes: vec![NonceChange::new(BlockAccessIndex::new(1), 1)],
                code_changes: vec![CodeChange::new(BlockAccessIndex::new(1), runtime.into())],
                storage_changes: vec![SlotChanges::new(
                    U256::ZERO,
                    (1..=if capture_chain_id { 1 } else { 3 })
                        .map(|index| {
                            StorageChange::new(
                                BlockAccessIndex::new(index),
                                U256::from(if capture_chain_id { chain_id } else { index }),
                            )
                        })
                        .collect(),
                )],
                storage_reads: if parent_storage.is_some() { vec![U256::from(1)] } else { vec![] },
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
        let block_hash = block.header().hash();
        Self {
            handle,
            transactions,
            gas,
            block_hash,
            parent_hash,
            bal: accounts.into_values().collect(),
        }
    }
}

struct RpcProxy {
    endpoint: String,
    requests: Arc<Mutex<Vec<Value>>>,
}

#[derive(Clone)]
enum BalResponse {
    Result(Value),
    Unsupported,
    Timeout,
}

#[derive(Clone)]
struct ProxyOptions {
    bal: BalResponse,
    reject_prestate: bool,
    malformed_prestate: bool,
    reported_hardfork: Option<&'static str>,
    invalid_prefix: bool,
    wrong_transaction_index: bool,
    wrong_parent_hash: bool,
    unavailable_account: Option<Address>,
}

impl ProxyOptions {
    const fn with_bal(bal: Value) -> Self {
        Self {
            bal: BalResponse::Result(bal),
            reject_prestate: false,
            malformed_prestate: false,
            reported_hardfork: None,
            invalid_prefix: false,
            wrong_transaction_index: false,
            wrong_parent_hash: false,
            unavailable_account: None,
        }
    }
}

impl RpcProxy {
    async fn new(fixture: &Fixture, options: ProxyOptions) -> Self {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let client = reqwest::Client::new();
        let endpoint = fixture.handle.http_endpoint();
        let block_hash = fixture.block_hash;
        let target =
            fixture.bal.iter().find(|account| !account.code_changes.is_empty()).unwrap().address;
        let router = Router::new().route(
            "/",
            post(move |Json(request): Json<Value>| {
                let client = client.clone();
                let endpoint = endpoint.clone();
                let options = options.clone();
                let recorded = Arc::clone(&recorded);
                async move {
                    // The proxy handles batches too, because fork account reads are batched.
                    let batch = request.is_array();
                    let requests = match request {
                        Value::Array(requests) => requests,
                        request => vec![request],
                    };
                    let mut responses = Vec::new();
                    for request in requests {
                        recorded.lock().unwrap().push(request.clone());
                        let method = request["method"].as_str().unwrap();
                        let mut response = if method == BAL_METHOD {
                            match &options.bal {
                                BalResponse::Result(bal) => {
                                    json!({"jsonrpc": "2.0", "id": request["id"], "result": bal})
                                }
                                BalResponse::Unsupported => rpc_error(&request, -32601),
                                BalResponse::Timeout => {
                                    // This response never arrives: only the BAL deadline can
                                    // let the command continue to replay.
                                    future::pending::<Value>().await
                                }
                            }
                        } else if (options.reject_prestate
                            && method == "debug_traceTransaction"
                            && request.pointer("/params/1/tracer")
                                == Some(&json!("prestateTracer")))
                            || (options.unavailable_account.is_some_and(|address| {
                                request.pointer("/params/0") == Some(&json!(address))
                            }) && matches!(
                                method,
                                "eth_getAccountInfo"
                                    | "eth_getBalance"
                                    | "eth_getCode"
                                    | "eth_getTransactionCount"
                            ))
                        {
                            rpc_error(&request, -32602)
                        } else {
                            client
                                .post(&endpoint)
                                .json(&request)
                                .send()
                                .await
                                .unwrap()
                                .json::<Value>()
                                .await
                                .unwrap()
                        };
                        if options.malformed_prestate
                            && method == "debug_traceTransaction"
                            && request.pointer("/params/1/tracer") == Some(&json!("prestateTracer"))
                        {
                            // Valid JSON and account structure, but invalid EIP-7702 bytecode.
                            response["result"][format!("{target:#x}")]["code"] = json!("0xef01");
                        }
                        if let Some(hardfork) = options.reported_hardfork
                            && method == "anvil_nodeInfo"
                        {
                            response["result"]["hardFork"] = json!(hardfork);
                        }
                        if options.wrong_transaction_index && method == "eth_getTransactionByHash" {
                            response["result"]["transactionIndex"] = json!("0x0");
                        }
                        if options.wrong_parent_hash
                            && matches!(method, "eth_getBlockByHash" | "eth_getBlockByNumber")
                            && response.pointer("/result/hash") == Some(&json!(block_hash))
                        {
                            response["result"]["parentHash"] = json!(B256::repeat_byte(0x99));
                        }
                        if options.invalid_prefix
                            && matches!(method, "eth_getBlockByHash" | "eth_getBlockByNumber")
                            && request.pointer("/params/1") == Some(&Value::Bool(true))
                            && response.pointer("/result/hash") == Some(&json!(block_hash))
                        {
                            response["result"]["transactions"][0]["gas"] = json!("0x0");
                        }
                        responses.push(response);
                    }
                    Json(if batch { Value::Array(responses) } else { responses.pop().unwrap() })
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self { endpoint: format!("http://{address}"), requests }
    }

    fn requests(&self, method: &str) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request["method"] == method)
            .cloned()
            .collect()
    }
}

fn rpc_error(request: &Value, code: i32) -> Value {
    json!({"jsonrpc": "2.0", "id": request["id"], "error": {"code": code, "message": "fixture rejection"}})
}

fn run(cmd: &mut TestCommand, hash: B256, endpoint: &str, flags: &[&str]) -> Output {
    run_command(cmd, hash, endpoint, flags).assert_success().get_output().clone()
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

casttest!(flaky_cast_run_fork_bal_live_matches_replay, |_prj, cmd| {
    let endpoint = next_ws_archive_rpc_url();
    // Mainnet block 19,999,957, transaction index 1. The preceding transaction changes slot 0
    // of pool 0x757d8d585ee1488097305bcb0fcec1ac4a55e9d6 used by this swap. The target and later
    // transactions change it again, so restoring the correct transaction boundary matters.
    let hash =
        "0x2c74889b29089993fc9026e3039670624dd3696864a48c5ab56c706c39c6e0c0".parse().unwrap();

    // A matching explicit hardfork forces ordinary replay under the same execution rules.
    run_command(&mut cmd, hash, &endpoint, &["--evm-version", "cancun"]);
    cmd.env("RUST_LOG", "off");
    let replay = cmd
        .with_no_redact()
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
...
Transaction successfully executed.
Gas used: 190596

"#]])
        .stderr_eq("Executing previous transactions from the block.\n")
        .get_output()
        .clone();

    // Compare the entire trace, including logs, storage changes, return data and receipt gas.
    // Require BAL application so an unsupported method or timeout cannot pass via fallback.
    run_command(&mut cmd, hash, &endpoint, &[]);
    cmd.env("RUST_LOG", "cast::cmd::run=trace");
    cmd.with_no_redact().assert_success().stdout_eq(replay.stdout).stderr_eq(str![[r#"
[..] TRACE cast::cmd::run: BAL prestate applied successfully, skipping block replay
[..] TRACE cast::cmd::run: executing call transaction tx=0x2c74889b29089993fc9026e3039670624dd3696864a48c5ab56c706c39c6e0c0 to=0x3fc91a3afd70395cd496c647d5a6cc9d4b2b7fad
[..] TRACE cast::cmd::run: completed block replay tx_hash=0x2c74889b29089993fc9026e3039670624dd3696864a48c5ab56c706c39c6e0c0

"#]]);
});

casttest!(cast_run_fork_bal_matches_replay_at_every_position, async |_prj, cmd| {
    let fixture = Fixture::new().await;
    let proxy = RpcProxy::new(&fixture, ProxyOptions::with_bal(json!(fixture.bal))).await;
    for (index, hash) in fixture.transactions.iter().enumerate() {
        let replay = run(&mut cmd, *hash, &fixture.handle.http_endpoint(), &[]);
        let accelerated = run(&mut cmd, *hash, &proxy.endpoint, &[]);
        // The entire rendered trace includes return data, emitted log data, state changes and
        // gas. Comparing the raw output also avoids the usual [GAS] snapshot redaction.
        assert_eq!(accelerated.stdout, replay.stdout, "transaction {index}");
        assert_eq!(
            accelerated.stdout_lossy().lines().find_map(|line| line.strip_prefix("Gas used: ")),
            Some(fixture.gas[index].to_string().as_str()),
        );
        OutputAssert::new(accelerated).stderr_eq(str![[r#"
"#]]);
    }
    let requests = proxy.requests(BAL_METHOD);
    assert_eq!(requests.len(), fixture.transactions.len());
    for request in requests {
        assert_eq!(request["params"], json!([fixture.block_hash]));
    }
    let account_reads = proxy.requests("eth_getBalance");
    assert!(!account_reads.is_empty(), "BAL preparation must load unchanged parent fields");
    for method in ["eth_getBalance", "eth_getCode", "eth_getTransactionCount", "eth_getStorageAt"] {
        for request in proxy.requests(method) {
            let parameter = if method == "eth_getStorageAt" { 2 } else { 1 };
            let block =
                serde_json::from_value::<BlockId>(request["params"][parameter].clone()).unwrap();
            assert_eq!(block.as_block_hash(), Some(fixture.parent_hash));
        }
    }
});

casttest!(cast_run_fork_bal_failures_restore_clean_replay, async |_prj, cmd| {
    let fixture = Fixture::new().await;
    let hash = fixture.transactions[2];
    let replay = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &[]);

    let mut duplicate = fixture.bal.clone();
    duplicate.insert(0, duplicate[0].clone());
    let mut out_of_range = fixture.bal.clone();
    out_of_range[0].nonce_changes.push(NonceChange::new(BlockAccessIndex::new(99), 1));
    let mut unavailable = fixture.bal.clone();
    // Force a failed read while staging real changes. Any partially applied BAL would run the
    // prefix against its own output and change the target's trace or fail CREATE.
    let unavailable_account = Address::repeat_byte(0xff);
    unavailable.push(AccountChanges {
        nonce_changes: vec![NonceChange::new(BlockAccessIndex::new(1), 1)],
        ..AccountChanges::new(unavailable_account)
    });

    let cases = [
        ("missing", ProxyOptions::with_bal(Value::Null)),
        (
            "unsupported",
            ProxyOptions { bal: BalResponse::Unsupported, ..ProxyOptions::with_bal(Value::Null) },
        ),
        (
            "timeout",
            ProxyOptions { bal: BalResponse::Timeout, ..ProxyOptions::with_bal(Value::Null) },
        ),
        ("malformed", ProxyOptions::with_bal(json!({"unexpected": true}))),
        ("duplicate account", ProxyOptions::with_bal(json!(duplicate))),
        ("index past block", ProxyOptions::with_bal(json!(out_of_range))),
        (
            "wrong transaction position",
            ProxyOptions {
                wrong_transaction_index: true,
                ..ProxyOptions::with_bal(json!(fixture.bal))
            },
        ),
        (
            "wrong parent",
            ProxyOptions { wrong_parent_hash: true, ..ProxyOptions::with_bal(json!(fixture.bal)) },
        ),
        (
            "failed staging read",
            ProxyOptions {
                unavailable_account: Some(unavailable_account),
                ..ProxyOptions::with_bal(json!(unavailable))
            },
        ),
    ];
    for (name, options) in cases {
        let proxy = RpcProxy::new(&fixture, options).await;
        let fallback = run(&mut cmd, hash, &proxy.endpoint, &[]);
        assert_eq!(fallback.stdout, replay.stdout, "{name}");
        if name == "failed staging read" {
            OutputAssert::new(fallback).stderr_eq(str![[r#"
[..]ERROR sharedbackend: Failed to send/recv `basic` err=failed to get account for [..]: server returned an error response: error code -32602: fixture rejection address=[..]
Executing previous transactions from the block.

"#]]);
        } else {
            OutputAssert::new(fallback).stderr_eq(str![[r#"
Executing previous transactions from the block.

"#]]);
        }
    }
});

casttest!(cast_run_fork_bal_does_not_execute_prefix, async |_prj, cmd| {
    let fixture = Fixture::new().await;
    let hash = fixture.transactions[2];
    let replay = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &[]);
    let proxy = RpcProxy::new(
        &fixture,
        ProxyOptions { invalid_prefix: true, ..ProxyOptions::with_bal(json!(fixture.bal)) },
    )
    .await;
    let output = run(&mut cmd, hash, &proxy.endpoint, &[]);
    assert_eq!(output.stdout, replay.stdout);
    OutputAssert::new(output).stderr_eq("");

    // The same prefix cannot execute when BAL is missing: its deployment has zero gas.
    let replay_only = RpcProxy::new(
        &fixture,
        ProxyOptions { invalid_prefix: true, ..ProxyOptions::with_bal(Value::Null) },
    )
    .await;
    cmd.cast_fuse()
        .args(["run", &hash.to_string(), "--rpc-url", &replay_only.endpoint])
        .assert_failure();
});

casttest!(cast_run_fork_bal_respects_prestate_quick_and_remote_modes, async |_prj, cmd| {
    let fixture = Fixture::new().await;
    let hash = fixture.transactions[2];
    let replay = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &[]);

    let prestate = RpcProxy::new(&fixture, ProxyOptions::with_bal(json!(fixture.bal))).await;
    let output = run(&mut cmd, hash, &prestate.endpoint, &["--prestate-tracer"]);
    assert_eq!(output.stdout, replay.stdout);
    assert_eq!(prestate.requests("debug_traceTransaction").len(), 1);
    assert!(prestate.requests(BAL_METHOD).is_empty());

    for available in [true, false] {
        let proxy = RpcProxy::new(
            &fixture,
            ProxyOptions {
                reject_prestate: true,
                ..ProxyOptions::with_bal(if available { json!(fixture.bal) } else { Value::Null })
            },
        )
        .await;
        let output = run(&mut cmd, hash, &proxy.endpoint, &["--prestate-tracer"]);
        assert_eq!(output.stdout, replay.stdout);
        let methods = proxy
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter_map(|request| request["method"].as_str().map(str::to_owned))
            .filter(|method| method == "debug_traceTransaction" || method == BAL_METHOD)
            .collect::<Vec<_>>();
        assert_eq!(methods, ["debug_traceTransaction", BAL_METHOD]);
        if available {
            OutputAssert::new(output).stderr_eq("");
        } else {
            OutputAssert::new(output).stderr_eq(str![[r#"
Executing previous transactions from the block.

"#]]);
        }
    }

    let quick = RpcProxy::new(&fixture, ProxyOptions::with_bal(json!(fixture.bal))).await;
    let expected_quick = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &["--quick"]);
    let output = run(&mut cmd, hash, &quick.endpoint, &["--quick", "--prestate-tracer"]);
    assert_eq!(output.stdout, expected_quick.stdout);
    assert_ne!(output.stdout, replay.stdout, "the contract is absent in the parent block");
    assert!(quick.requests(BAL_METHOD).is_empty());
    assert!(quick.requests("debug_traceTransaction").is_empty());

    let remote = RpcProxy::new(&fixture, ProxyOptions::with_bal(json!(fixture.bal))).await;
    let expected_remote =
        run(&mut cmd, hash, &fixture.handle.http_endpoint(), &["--debug-trace-transaction"]);
    let output = run(&mut cmd, hash, &remote.endpoint, &["--debug-trace-transaction"]);
    assert_eq!(output.stdout, expected_remote.stdout);
    assert!(remote.requests(BAL_METHOD).is_empty());
    assert_eq!(remote.requests("debug_traceTransaction").len(), 1);
    assert_eq!(
        remote.requests("debug_traceTransaction")[0]["params"][1]["tracer"],
        json!("callTracer")
    );
});

casttest!(cast_run_fork_bal_skips_inapplicable_modes_and_bad_prestate, async |_prj, cmd| {
    let fixture = Fixture::new().await;
    let hash = fixture.transactions[2];
    let replay = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &[]);

    let proxy = RpcProxy::new(&fixture, ProxyOptions::with_bal(json!(fixture.bal))).await;
    let flags = ["--evm-version", "cancun"];
    let overridden_replay = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &flags);
    let output = run(&mut cmd, hash, &proxy.endpoint, &flags);
    assert_eq!(output.stdout, overridden_replay.stdout);
    assert!(proxy.requests(BAL_METHOD).is_empty(), "explicit EVM overrides bypass BAL");
    OutputAssert::new(output).stderr_eq(str![[r#"
Executing previous transactions from the block.

"#]]);

    // Both endpoints report the same older execution rules. This checks eligibility without
    // asserting that deliberately altered source metadata reproduces the canonical receipt.
    let source_options =
        ProxyOptions { reported_hardfork: Some("Shanghai"), ..ProxyOptions::with_bal(Value::Null) };
    let old_source = RpcProxy::new(&fixture, source_options.clone()).await;
    let old_replay = run(&mut cmd, hash, &old_source.endpoint, &[]);
    let old_source_bal = RpcProxy::new(
        &fixture,
        ProxyOptions { bal: BalResponse::Result(json!(fixture.bal)), ..source_options },
    )
    .await;
    let output = run(&mut cmd, hash, &old_source_bal.endpoint, &[]);
    assert_eq!(output.stdout, old_replay.stdout);
    assert!(!old_source_bal.requests("anvil_nodeInfo").is_empty());
    assert!(old_source_bal.requests(BAL_METHOD).is_empty(), "pre-Cancun source state bypasses BAL");
    OutputAssert::new(output).stderr_eq(str![[r#"
Executing previous transactions from the block.

"#]]);

    for available in [true, false] {
        let proxy = RpcProxy::new(
            &fixture,
            ProxyOptions {
                malformed_prestate: true,
                ..ProxyOptions::with_bal(if available { json!(fixture.bal) } else { Value::Null })
            },
        )
        .await;
        let output = run(&mut cmd, hash, &proxy.endpoint, &["--prestate-tracer"]);
        assert_eq!(output.stdout, replay.stdout, "malformed prestate, BAL available: {available}");
        let methods = proxy
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter_map(|request| request["method"].as_str().map(str::to_owned))
            .filter(|method| method == "debug_traceTransaction" || method == BAL_METHOD)
            .collect::<Vec<_>>();
        assert_eq!(methods, ["debug_traceTransaction", BAL_METHOD]);
        if available {
            OutputAssert::new(output).stderr_eq("");
        } else {
            OutputAssert::new(output).stderr_eq(str![[r#"
Executing previous transactions from the block.

"#]]);
        }
    }
});

casttest!(cast_run_fork_bal_respects_chain_id_override, async |_prj, cmd| {
    let fixture = Fixture::with_mode(FixtureMode::ChainId).await;
    let hash = fixture.transactions[2];
    let chain_id = fixture.handle.http_provider().get_chain_id().await.unwrap();
    let canonical = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &[]);

    for override_id in [None, Some(chain_id), Some(chain_id + 1)] {
        let proxy = RpcProxy::new(&fixture, ProxyOptions::with_bal(json!(fixture.bal))).await;
        let [replay, restored] =
            [&fixture.handle.http_endpoint(), &proxy.endpoint].map(|endpoint| {
                run_command(&mut cmd, hash, endpoint, &[]);
                if let Some(override_id) = override_id {
                    cmd.env("FOUNDRY_CHAIN_ID", override_id.to_string());
                }
                cmd.assert_success().get_output().clone()
            });
        assert_eq!(restored.stdout, replay.stdout, "chain ID override: {override_id:?}");
        if override_id.is_none_or(|id| id == chain_id) {
            assert_eq!(restored.stdout, canonical.stdout);
            assert_eq!(proxy.requests(BAL_METHOD).len(), 1);
            OutputAssert::new(restored).stderr_eq("");
        } else {
            assert_ne!(replay.stdout, canonical.stdout, "the prefix must use the override");
            assert!(proxy.requests(BAL_METHOD).is_empty());
            OutputAssert::new(restored).stderr_eq(str![[r#"
Executing previous transactions from the block.

"#]]);
        }
    }
});

casttest!(cast_run_fork_bal_skips_forwarded_history, async |_prj, cmd| {
    let fixture = Fixture::with_mode(FixtureMode::ChainId).await;
    let hash = fixture.transactions[2];
    let provider = fixture.handle.http_provider();
    let source_chain_id = provider.get_chain_id().await.unwrap();
    let canonical = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &[]);

    // Cover both the fork anchor itself and an earlier upstream block.
    for extra_blocks in [0, 1] {
        if extra_blocks > 0 {
            provider.raw_request::<_, Value>("evm_mine".into(), ()).await.unwrap();
        }
        let fork_block = provider.get_block_number().await.unwrap();
        for (chain_id, hardfork, evm_version) in [
            (source_chain_id + 1, EthereumHardfork::Cancun, "cancun"),
            (source_chain_id, EthereumHardfork::Prague, "prague"),
        ] {
            let proxy = RpcProxy::new(&fixture, ProxyOptions::with_bal(json!(fixture.bal))).await;
            let (_, fork) = anvil::spawn(
                NodeConfig::test()
                    .with_eth_rpc_url(Some(proxy.endpoint.clone()))
                    .with_fork_block_number(Some(fork_block))
                    .with_chain_id(Some(chain_id))
                    .with_hardfork(Some(hardfork.into())),
            )
            .await;

            // Replay uses the endpoint's local rules. The forwarded BAL was produced with
            // upstream rules, which can differ even when the chain IDs match.
            run_command(
                &mut cmd,
                hash,
                &fixture.handle.http_endpoint(),
                &["--evm-version", evm_version],
            );
            cmd.env("FOUNDRY_CHAIN_ID", chain_id.to_string());
            let replay = cmd.assert_success().get_output().clone();
            if chain_id != source_chain_id {
                assert_ne!(replay.stdout, canonical.stdout);
            }

            let output = run(&mut cmd, hash, &fork.http_endpoint(), &[]);
            assert!(
                proxy.requests(BAL_METHOD).is_empty(),
                "forwarded historical BAL must not be fetched"
            );
            OutputAssert::new(output)
                .stdout_eq(String::from_utf8(replay.stdout).unwrap())
                .stderr_eq(str![[r#"
Executing previous transactions from the block.

"#]]);
        }
    }
});

casttest!(cast_run_fork_bal_applies_to_locally_mined_blocks, async |_prj, cmd| {
    let (_, origin) = anvil::spawn(NodeConfig::test()).await;
    let source_chain_id = origin.http_provider().get_chain_id().await.unwrap();
    let fixture = Fixture::with_config(
        FixtureMode::ChainId,
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin.http_endpoint()))
            .with_fork_block_number(Some(0u64))
            .with_chain_id(Some(source_chain_id + 1)),
    )
    .await;
    let hash = fixture.transactions[2];
    let replay = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &[]);
    let proxy = RpcProxy::new(&fixture, ProxyOptions::with_bal(json!(fixture.bal))).await;

    // Blocks mined after the anchor use the local rules, so BAL can still replace replay.
    let output = run(&mut cmd, hash, &proxy.endpoint, &[]);
    assert_eq!(proxy.requests(BAL_METHOD).len(), 1);
    OutputAssert::new(output).stdout_eq(String::from_utf8(replay.stdout).unwrap()).stderr_eq("");
});

casttest!(cast_run_fork_bal_falls_back_for_creation_over_parent_storage, async |_prj, cmd| {
    for parent_storage in [U256::ZERO, U256::from(77)] {
        let fixture = Fixture::with_mode(FixtureMode::ParentStorage(parent_storage)).await;
        let hash = fixture.transactions[1];
        let replay = run(&mut cmd, hash, &fixture.handle.http_endpoint(), &[]);
        let proxy = RpcProxy::new(&fixture, ProxyOptions::with_bal(json!(fixture.bal))).await;
        let restored = run(&mut cmd, hash, &proxy.endpoint, &[]);
        assert_eq!(restored.stdout, replay.stdout, "parent storage: {parent_storage}");
        assert_eq!(proxy.requests(BAL_METHOD).len(), 1);
        if parent_storage.is_zero() {
            OutputAssert::new(restored).stderr_eq("");
        } else {
            OutputAssert::new(restored).stderr_eq(str![[r#"
Executing previous transactions from the block.

"#]]);
        }
    }
});
