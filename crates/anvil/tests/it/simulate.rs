//! general eth api tests

use alloy_primitives::{Bytes, TxKind, U256, address};
use alloy_provider::Provider;
use alloy_rpc_types::{
    BlockOverrides,
    request::TransactionRequest,
    simulate::{SimBlock, SimulatePayload},
    state::{AccountOverride, StateOverridesBuilder},
};
use anvil::{EthereumHardfork, NodeConfig, spawn};
use foundry_test_utils::rpc;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_simulate_v1() {
    crate::init_tracing();
    let (api, _) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(rpc::next_http_archive_rpc_url()))).await;
    let block_overrides =
        Some(BlockOverrides { base_fee: Some(U256::from(9)), ..Default::default() });
    let account_override =
        AccountOverride { balance: Some(U256::from(999999999999u64)), ..Default::default() };
    let state_overrides = Some(
        StateOverridesBuilder::with_capacity(1)
            .append(address!("0xc000000000000000000000000000000000000001"), account_override)
            .build(),
    );
    let tx_request = TransactionRequest {
        from: Some(address!("0xc000000000000000000000000000000000000001")),
        to: Some(TxKind::from(address!("0xc000000000000000000000000000000000000001"))),
        value: Some(U256::from(1)),
        ..Default::default()
    };
    let payload = SimulatePayload {
        block_state_calls: vec![SimBlock {
            block_overrides,
            state_overrides,
            calls: vec![tx_request],
        }],
        trace_transfers: true,
        validation: false,
        return_full_transactions: true,
    };
    let _res = api.simulate_v1(payload, None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_enforces_block_gas_limit() {
    let (api, _) = spawn(NodeConfig::test()).await;
    let sender = address!("0xc000000000000000000000000000000000000000");
    let receiver = address!("0xc100000000000000000000000000000000000000");
    let gas_burner = address!("0xc200000000000000000000000000000000000000");
    let gas_limit = 100_000;
    let state_overrides = Some(
        StateOverridesBuilder::with_capacity(1)
            .append(
                gas_burner,
                AccountOverride {
                    code: Some(Bytes::from_static(&[0x5b, 0x5f, 0x56])),
                    ..Default::default()
                },
            )
            .build(),
    );
    let calls = vec![
        TransactionRequest {
            from: Some(sender),
            to: Some(TxKind::Call(receiver)),
            ..Default::default()
        },
        TransactionRequest {
            from: Some(sender),
            to: Some(TxKind::Call(gas_burner)),
            ..Default::default()
        },
    ];
    let payload = SimulatePayload {
        block_state_calls: vec![SimBlock {
            block_overrides: Some(BlockOverrides {
                gas_limit: Some(gas_limit),
                ..Default::default()
            }),
            state_overrides,
            calls,
        }],
        ..Default::default()
    };

    let blocks = api.simulate_v1(payload, None).await.unwrap();
    let block = &blocks[0];

    assert!(block.calls[0].status);
    assert!(!block.calls[1].status);
    assert_eq!(block.inner.header.gas_used, gas_limit);
    assert_eq!(block.calls[1].gas_used, gas_limit - block.calls[0].gas_used);
    assert_eq!(block.calls[1].error.as_ref().unwrap().code, -32015);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_rejects_call_above_remaining_block_gas() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let sender = address!("0xc000000000000000000000000000000000000000");
    let receiver = address!("0xc100000000000000000000000000000000000000");
    let gas_limit = 100_000;
    let payload = SimulatePayload {
        block_state_calls: vec![SimBlock {
            block_overrides: Some(BlockOverrides {
                gas_limit: Some(gas_limit),
                ..Default::default()
            }),
            calls: vec![
                TransactionRequest {
                    from: Some(sender),
                    to: Some(TxKind::Call(receiver)),
                    ..Default::default()
                },
                TransactionRequest {
                    from: Some(sender),
                    to: Some(TxKind::Call(receiver)),
                    gas: Some(gas_limit),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    let provider = handle.http_provider();
    let response: Result<serde_json::Value, _> =
        provider.client().request("eth_simulateV1", (payload,)).await;
    let error = response.unwrap_err();
    assert_eq!(error.as_error_resp().unwrap().code, -38015);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_tracks_amsterdam_gas_dimensions_separately() {
    let (api, _) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Amsterdam.into()))).await;
    let sender = address!("0xc000000000000000000000000000000000000000");
    let receiver = address!("0xc100000000000000000000000000000000000000");
    let storage = address!("0xc200000000000000000000000000000000000000");
    let gas_limit = 130_000;
    let payload = SimulatePayload {
        block_state_calls: vec![SimBlock {
            block_overrides: Some(BlockOverrides {
                gas_limit: Some(gas_limit),
                ..Default::default()
            }),
            state_overrides: Some(
                StateOverridesBuilder::with_capacity(1)
                    .append(
                        storage,
                        AccountOverride {
                            code: Some(Bytes::from_static(&[0x60, 0x01, 0x60, 0x00, 0x55, 0x00])),
                            ..Default::default()
                        },
                    )
                    .build(),
            ),
            calls: vec![
                TransactionRequest {
                    from: Some(sender),
                    to: Some(TxKind::Call(storage)),
                    ..Default::default()
                },
                TransactionRequest {
                    from: Some(sender),
                    to: Some(TxKind::Call(receiver)),
                    ..Default::default()
                },
            ],
        }],
        ..Default::default()
    };

    let blocks = api.simulate_v1(payload, None).await.unwrap();
    let block = &blocks[0];

    assert!(block.calls.iter().all(|call| call.status));
    let cumulative_gas_used = block.calls.iter().map(|call| call.gas_used).sum::<u64>();
    assert!(cumulative_gas_used > gas_limit);
    assert!(block.inner.header.gas_used <= gas_limit);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_create_calls_rpc() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [
                {},
                {
                    "calls": [
                        {},
                        {"from": "0xc000000000000000000000000000000000000000"},
                        {"input": "0x602a5f526001601ff3"},
                        {"input": "0x63deadbeef5f526004601cfd"}
                    ]
                }
            ],
            "returnFullTransactions": true
        }, "latest"]),
    )
    .await;

    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"][0]["calls"], json!([]));
    assert_eq!(response["result"][0]["transactions"], json!([]));

    let block = &response["result"][1];
    let calls = block["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[0]["status"], "0x1");
    assert_eq!(calls[0]["returnData"], "0x");
    assert_eq!(calls[1]["status"], "0x1");
    assert_eq!(calls[1]["returnData"], "0x");
    assert_eq!(calls[2]["status"], "0x1");
    assert_eq!(calls[2]["returnData"], "0x2a");
    assert_eq!(calls[3]["status"], "0x0");
    assert_eq!(calls[3]["returnData"], "0x");
    assert_eq!(calls[3]["error"]["code"], 3);
    assert_eq!(calls[3]["error"]["data"], "0xdeadbeef");

    let transactions = block["transactions"].as_array().unwrap();
    assert_eq!(transactions.len(), 4);
    assert!(transactions.iter().all(|transaction| transaction["to"].is_null()));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_validation_defaults_base_fee_to_zero() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [
                {},
                {"blockOverrides": {"baseFeePerGas": "0x9"}},
                {}
            ]
        }, "latest"]),
    )
    .await;

    assert_eq!(response["result"][0]["baseFeePerGas"], "0x0");
    assert_eq!(response["result"][1]["baseFeePerGas"], "0x9");
    assert_eq!(response["result"][2]["baseFeePerGas"], "0x0");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_resolves_nonces_from_state() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let sender = "0xc000000000000000000000000000000000000000";
    let receiver = "0xc100000000000000000000000000000000000000";
    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "blockOverrides": {"baseFeePerGas": "0x0"},
                "stateOverrides": {
                    (sender): {"balance": "0x1", "code": "0x00", "nonce": "0x7"}
                },
                "calls": [
                    {"from": sender, "to": receiver},
                    {"from": sender, "to": receiver}
                ]
            }],
            "validation": true,
            "returnFullTransactions": true
        }, "latest"]),
    )
    .await;

    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"][0]["transactions"][0]["nonce"], "0x7");
    assert_eq!(response["result"][0]["transactions"][1]["nonce"], "0x8");

    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {(sender): {"nonce": "0xfffffffffffffffe"}},
                "calls": [
                    {"from": sender, "to": receiver},
                    {"from": sender, "to": receiver},
                    {"from": sender, "to": receiver}
                ]
            }],
            "returnFullTransactions": true
        }, "latest"]),
    )
    .await;

    assert!(response.get("error").is_none(), "{response}");
    assert!(
        response["result"][0]["calls"]
            .as_array()
            .unwrap()
            .iter()
            .all(|call| call["status"] == "0x1")
    );
    assert_eq!(response["result"][0]["transactions"][0]["nonce"], "0xfffffffffffffffe");
    assert_eq!(response["result"][0]["transactions"][1]["nonce"], "0xffffffffffffffff");
    assert_eq!(response["result"][0]["transactions"][2]["nonce"], "0x0");

    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {(sender): {"nonce": "0xffffffffffffffff"}},
                "calls": [
                    {"from": sender},
                    {"from": sender, "to": receiver}
                ]
            }],
            "returnFullTransactions": true
        }, "latest"]),
    )
    .await;

    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"][0]["transactions"][0]["nonce"], "0xffffffffffffffff");
    assert_eq!(response["result"][0]["transactions"][1]["nonce"], "0xffffffffffffffff");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_maps_validation_errors() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let sender = "0xc000000000000000000000000000000000000000";
    let receiver = "0xc100000000000000000000000000000000000000";
    let cases = [
        (
            json!([{
                "blockStateCalls": [{
                    "blockOverrides": {"baseFeePerGas": "0x0"},
                    "stateOverrides": {(sender): {"nonce": "0x1"}},
                    "calls": [{"from": sender, "to": receiver, "nonce": "0x0"}]
                }],
                "validation": true
            }, "latest"]),
            -38010,
        ),
        (
            json!([{
                "blockStateCalls": [{
                    "blockOverrides": {"baseFeePerGas": "0x0"},
                    "stateOverrides": {(sender): {"nonce": "0x1"}},
                    "calls": [{"from": sender, "to": receiver, "nonce": "0x2"}]
                }],
                "validation": true
            }, "latest"]),
            -38011,
        ),
        (
            json!([{
                "blockStateCalls": [{
                    "blockOverrides": {"baseFeePerGas": "0x0"},
                    "stateOverrides": {(sender): {"nonce": "0xffffffffffffffff"}},
                    "calls": [{"from": sender, "to": receiver}]
                }],
                "validation": true
            }, "latest"]),
            -32603,
        ),
        (
            json!([{
                "blockStateCalls": [{
                    "blockOverrides": {"baseFeePerGas": "0xa"},
                    "calls": [{"from": sender, "to": receiver, "maxFeePerGas": "0x0"}]
                }],
                "validation": true
            }, "latest"]),
            -38012,
        ),
        (
            json!([{
                "blockStateCalls": [{"calls": [{"from": sender, "to": receiver, "gas": "0x0"}]}]
            }, "latest"]),
            -38013,
        ),
        (
            json!([{
                "blockStateCalls": [{"calls": [{"from": sender, "to": receiver, "value": "0x3e8"}]}]
            }, "latest"]),
            -38014,
        ),
    ];

    for (params, code) in cases {
        let response = rpc_request(&endpoint, "eth_simulateV1", params).await;
        assert_eq!(response["error"]["code"], code, "{response}");
    }
}

async fn rpc_request(endpoint: &str, method: &str, params: Value) -> Value {
    let response = reqwest::Client::new()
        .post(endpoint)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json().await.unwrap()
}
