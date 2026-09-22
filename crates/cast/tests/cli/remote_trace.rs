//! Tests for pinning remote traces to one canonical block context.

use alloy_network::{BlockResponse, TransactionBuilder, primitives::HeaderResponse};
use alloy_primitives::{B256, address, hex};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, TransactionRequest};
use anvil::{NodeConfig, NodeHandle};
use axum::{Json, Router, routing::post};
use foundry_test_utils::util::OutputExt;
use serde_json::{Value, json};
use std::{
    slice,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone)]
enum ResponseMutation {
    None,
    ReceiptBlockHash {
        tx_hash: String,
        replacement: String,
    },
    RefetchedTransactionBlockHash {
        tx_hash: String,
        replacement: String,
        lookups: Arc<AtomicUsize>,
    },
    MissingTransactionBlock {
        block_hash: String,
    },
    MissingTransactionFromFullBlock {
        tx_hash: String,
    },
    CanonicalBlockHash {
        block_number: String,
        replacement: String,
    },
    RefetchedCanonicalBlockHash {
        block_number: String,
        replacement: String,
        lookups: Arc<AtomicUsize>,
    },
}

async fn spawn_recording_rpc_proxy(
    endpoint: String,
    mutation: ResponseMutation,
) -> (String, Arc<Mutex<Vec<Value>>>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded_requests = Arc::clone(&requests);
    let client = reqwest::Client::new();
    let router = Router::new().route(
        "/",
        post(move |Json(request): Json<Value>| {
            let client = client.clone();
            let endpoint = endpoint.clone();
            let mutation = mutation.clone();
            let recorded_requests = Arc::clone(&recorded_requests);
            async move {
                recorded_requests.lock().unwrap().push(request.clone());
                let mut response = client
                    .post(endpoint)
                    .json(&request)
                    .send()
                    .await
                    .unwrap()
                    .json::<Value>()
                    .await
                    .unwrap();
                mutate_rpc_response(&request, &mut response, &mutation);
                Json(response)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{address}"), requests)
}

fn mutate_rpc_response(request: &Value, response: &mut Value, mutation: &ResponseMutation) {
    if let Some(requests) = request.as_array() {
        let Some(responses) = response.as_array_mut() else { return };
        for response in responses {
            let Some(response_id) = response.get("id") else { continue };
            if let Some(request) =
                requests.iter().find(|request| request.get("id") == Some(response_id))
            {
                mutate_rpc_result(request, response, mutation);
            }
        }
    } else {
        mutate_rpc_result(request, response, mutation);
    }
}

fn mutate_rpc_result(request: &Value, response: &mut Value, mutation: &ResponseMutation) {
    let Some(method) = request.get("method").and_then(Value::as_str) else { return };
    let requested_target = request.pointer("/params/0").and_then(Value::as_str);

    if let ResponseMutation::MissingTransactionBlock { block_hash } = mutation
        && method == "eth_getBlockByHash"
        && requested_target.is_some_and(|target| target.eq_ignore_ascii_case(block_hash))
    {
        response["result"] = Value::Null;
        return;
    }

    if let ResponseMutation::MissingTransactionFromFullBlock { tx_hash } = mutation
        && method == "eth_getBlockByNumber"
        && request.pointer("/params/1").and_then(Value::as_bool) == Some(true)
        && let Some(transactions) =
            response.pointer_mut("/result/transactions").and_then(Value::as_array_mut)
    {
        transactions.retain(|transaction| {
            transaction
                .get("hash")
                .and_then(Value::as_str)
                .is_none_or(|hash| !hash.eq_ignore_ascii_case(tx_hash))
        });
        return;
    }

    let replacement = match mutation {
        ResponseMutation::None => return,
        ResponseMutation::ReceiptBlockHash { tx_hash, replacement }
            if method == "eth_getTransactionReceipt"
                && requested_target.is_some_and(|target| target.eq_ignore_ascii_case(tx_hash)) =>
        {
            replacement
        }
        ResponseMutation::RefetchedTransactionBlockHash { tx_hash, replacement, lookups }
            if method == "eth_getTransactionByHash"
                && requested_target.is_some_and(|target| target.eq_ignore_ascii_case(tx_hash))
                && lookups.fetch_add(1, Ordering::Relaxed) == 1 =>
        {
            replacement
        }
        ResponseMutation::CanonicalBlockHash { block_number, replacement }
            if method == "eth_getBlockByNumber"
                && requested_target
                    .is_some_and(|target| target.eq_ignore_ascii_case(block_number))
                && response
                    .pointer("/result/number")
                    .and_then(Value::as_str)
                    .is_some_and(|number| number.eq_ignore_ascii_case(block_number)) =>
        {
            replacement
        }
        ResponseMutation::RefetchedCanonicalBlockHash { block_number, replacement, lookups }
            if method == "eth_getBlockByNumber"
                && requested_target
                    .is_some_and(|target| target.eq_ignore_ascii_case(block_number))
                && response
                    .pointer("/result/number")
                    .and_then(Value::as_str)
                    .is_some_and(|number| number.eq_ignore_ascii_case(block_number))
                && lookups.fetch_add(1, Ordering::Relaxed) == 1 =>
        {
            replacement
        }
        _ => return,
    };

    if let Some(result) = response.get_mut("result").and_then(Value::as_object_mut) {
        let field = if method == "eth_getBlockByNumber" { "hash" } else { "blockHash" };
        result.insert(field.to_string(), json!(replacement));
    }
}

fn flatten_requests(requests: &[Value]) -> impl Iterator<Item = &Value> {
    requests.iter().flat_map(|request| match request.as_array() {
        Some(requests) => requests.as_slice(),
        None => slice::from_ref(request),
    })
}

fn assert_block_hash_param(param: &Value, expected: B256) {
    let actual = param.as_str().or_else(|| param.get("blockHash").and_then(Value::as_str));
    let expected = expected.to_string();
    assert_eq!(actual, Some(expected.as_str()), "unexpected block parameter: {param}");
}

async fn send_identity_transaction(handle: &NodeHandle) -> (B256, u64, B256) {
    let provider = handle.http_provider();
    let from = provider.get_accounts().await.unwrap()[0];
    let receipt = provider
        .send_transaction(
            TransactionRequest::default()
                .with_from(from)
                .with_to(address!("0x0000000000000000000000000000000000000004"))
                .with_input(hex!("deadbeef"))
                .into(),
        )
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    (receipt.transaction_hash(), receipt.block_number.unwrap(), receipt.block_hash.unwrap())
}

casttest!(cast_call_remote_trace_pins_rpc_requests_to_block_hash, async |prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    api.mine_one().await.unwrap();
    let block = handle
        .http_provider()
        .get_block_by_number(BlockNumberOrTag::Number(1))
        .await
        .unwrap()
        .unwrap();
    let block_hash = block.header().hash();
    let (endpoint, requests) =
        spawn_recording_rpc_proxy(handle.http_endpoint(), ResponseMutation::None).await;

    cmd.set_current_dir(prj.root());
    cmd.args([
        "call",
        "0x0000000000000000000000000000000000000004",
        "--data",
        "0xdeadbeef",
        "--debug-trace-call",
        "--block",
        "1",
        "--with-local-artifacts",
        "--rpc-url",
        &endpoint,
    ])
    .assert_success();

    let requests = requests.lock().unwrap();
    let trace_request = flatten_requests(&requests)
        .find(|request| request["method"] == "debug_traceCall")
        .expect("debug_traceCall request was recorded");
    assert_block_hash_param(&trace_request["params"][1], block_hash);

    let code_requests = flatten_requests(&requests)
        .filter(|request| request["method"] == "eth_getCode")
        .collect::<Vec<_>>();
    assert!(!code_requests.is_empty(), "expected local-artifact code lookups");
    for request in code_requests {
        assert_block_hash_param(&request["params"][1], block_hash);
    }
});

casttest!(cast_call_remote_trace_rejects_canonical_block_mismatch, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    api.mine_one().await.unwrap();
    let (endpoint, _) = spawn_recording_rpc_proxy(
        handle.http_endpoint(),
        ResponseMutation::RefetchedCanonicalBlockHash {
            block_number: "0x1".to_string(),
            replacement: B256::repeat_byte(0x88).to_string(),
            lookups: Arc::new(AtomicUsize::new(0)),
        },
    )
    .await;

    let output = cmd
        .args([
            "call",
            "0x0000000000000000000000000000000000000004",
            "--data",
            "0xdeadbeef",
            "--debug-trace-call",
            "--block",
            "1",
            "--rpc-url",
            &endpoint,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(output.contains("changed canonicality"), "{output}");
    assert!(output.contains("canonical block lookup reported block"), "{output}");
});

casttest!(cast_run_remote_trace_pins_artifact_code_to_transaction_block, async |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let (tx_hash, _, block_hash) = send_identity_transaction(&handle).await;
    let (endpoint, requests) =
        spawn_recording_rpc_proxy(handle.http_endpoint(), ResponseMutation::None).await;

    cmd.set_current_dir(prj.root());
    let tx_hash = tx_hash.to_string();
    cmd.args([
        "run",
        "--debug-trace-transaction",
        &tx_hash,
        "--with-local-artifacts",
        "--rpc-url",
        &endpoint,
    ])
    .assert_success();

    let requests = requests.lock().unwrap();
    let code_requests = flatten_requests(&requests)
        .filter(|request| request["method"] == "eth_getCode")
        .collect::<Vec<_>>();
    assert!(!code_requests.is_empty(), "expected local-artifact code lookups");
    for request in code_requests {
        assert_block_hash_param(&request["params"][1], block_hash);
    }
});

casttest!(cast_run_remote_trace_rejects_receipt_inclusion_mismatch, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let (tx_hash, _, _) = send_identity_transaction(&handle).await;
    let (endpoint, _) = spawn_recording_rpc_proxy(
        handle.http_endpoint(),
        ResponseMutation::ReceiptBlockHash {
            tx_hash: tx_hash.to_string(),
            replacement: B256::repeat_byte(0x99).to_string(),
        },
    )
    .await;

    let tx_hash = tx_hash.to_string();
    let output = cmd
        .args(["run", "--debug-trace-transaction", &tx_hash, "--rpc-url", &endpoint])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(output.contains("transaction receipt reported block"), "{output}");
    assert!(output.contains("changed inclusion"), "{output}");
});

casttest!(cast_run_remote_trace_rejects_missing_transaction_block, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let (tx_hash, _, block_hash) = send_identity_transaction(&handle).await;
    let (endpoint, _) = spawn_recording_rpc_proxy(
        handle.http_endpoint(),
        ResponseMutation::MissingTransactionBlock { block_hash: block_hash.to_string() },
    )
    .await;

    let tx_hash = tx_hash.to_string();
    let output = cmd
        .args(["run", "--debug-trace-transaction", &tx_hash, "--rpc-url", &endpoint])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(output.contains("block fetched by hash no longer reports it as mined"), "{output}");
    assert!(output.contains("retry the command"), "{output}");
});

casttest!(cast_run_remote_trace_rejects_refetched_transaction_mismatch, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let (tx_hash, _, _) = send_identity_transaction(&handle).await;
    let (endpoint, _) = spawn_recording_rpc_proxy(
        handle.http_endpoint(),
        ResponseMutation::RefetchedTransactionBlockHash {
            tx_hash: tx_hash.to_string(),
            replacement: B256::repeat_byte(0xaa).to_string(),
            lookups: Arc::new(AtomicUsize::new(0)),
        },
    )
    .await;

    let tx_hash = tx_hash.to_string();
    let output = cmd
        .args(["run", "--debug-trace-transaction", &tx_hash, "--rpc-url", &endpoint])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(output.contains("transaction lookup reported block"), "{output}");
    assert!(output.contains("changed inclusion"), "{output}");
});

casttest!(cast_run_remote_trace_rejects_canonical_block_mismatch, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let (tx_hash, block_number, _) = send_identity_transaction(&handle).await;
    let (endpoint, _) = spawn_recording_rpc_proxy(
        handle.http_endpoint(),
        ResponseMutation::CanonicalBlockHash {
            block_number: format!("0x{block_number:x}"),
            replacement: B256::repeat_byte(0xbb).to_string(),
        },
    )
    .await;

    let tx_hash = tx_hash.to_string();
    let output = cmd
        .args(["run", "--debug-trace-transaction", &tx_hash, "--rpc-url", &endpoint])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(output.contains("canonical block lookup reported block"), "{output}");
    assert!(output.contains("changed inclusion"), "{output}");
});

casttest!(cast_run_rejects_target_missing_from_replay_block, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let (tx_hash, _, _) = send_identity_transaction(&handle).await;
    let (endpoint, _) = spawn_recording_rpc_proxy(
        handle.http_endpoint(),
        ResponseMutation::MissingTransactionFromFullBlock { tx_hash: tx_hash.to_string() },
    )
    .await;

    let tx_hash = tx_hash.to_string();
    let output = cmd
        .args(["run", &tx_hash, "--rpc-url", &endpoint])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(
        output.contains(&format!("transaction {tx_hash} is missing from its block")),
        "{output}"
    );
});
