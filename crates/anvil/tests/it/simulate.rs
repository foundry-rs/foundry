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
use std::time::Duration;

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
async fn test_simulate_normalizes_block_sequence_rpc() {
    let (_api, handle) = spawn(NodeConfig::test().with_genesis_timestamp(Some(1_000u64))).await;
    let endpoint = handle.http_endpoint();
    let latest = rpc_request(&endpoint, "eth_getBlockByNumber", json!(["latest", false])).await;
    let base = &latest["result"];
    let base_number = quantity(&base["number"]);
    let base_timestamp = quantity(&base["timestamp"]);

    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [
                {
                    "blockOverrides": {
                        "number": format!("{:#x}", base_number + 3),
                        "time": format!("{:#x}", base_timestamp + 100)
                    }
                },
                {},
                {
                    "blockOverrides": {
                        "number": format!("{:#x}", base_number + 7)
                    }
                }
            ]
        }, "latest"]),
    )
    .await;
    assert!(response.get("error").is_none(), "{response}");
    let blocks = response["result"].as_array().unwrap();

    let expected = [
        (base_number + 1, base_timestamp + 12),
        (base_number + 2, base_timestamp + 24),
        (base_number + 3, base_timestamp + 100),
        (base_number + 4, base_timestamp + 112),
        (base_number + 5, base_timestamp + 124),
        (base_number + 6, base_timestamp + 136),
        (base_number + 7, base_timestamp + 148),
    ];
    let mut expected_parent = base["hash"].clone();
    for (block, (number, timestamp)) in blocks.iter().zip(expected) {
        assert_eq!(quantity(&block["number"]), number);
        assert_eq!(quantity(&block["timestamp"]), timestamp);
        assert_eq!(block["parentHash"], expected_parent);
        expected_parent = block["hash"].clone();
    }
    assert!(blocks[0]["calls"].as_array().unwrap().is_empty());
    assert!(blocks[1]["calls"].as_array().unwrap().is_empty());
    assert!(blocks[4]["calls"].as_array().unwrap().is_empty());
    assert!(blocks[5]["calls"].as_array().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_uses_configured_block_interval_rpc() {
    for mixed_mining in [false, true] {
        let config = NodeConfig::test()
            .with_genesis_timestamp(Some(1_000u64))
            .with_mixed_mining(mixed_mining, Some(Duration::from_millis(1_500)));
        let (api, handle) = spawn(config).await;
        let endpoint = handle.http_endpoint();
        let base = rpc_request(&endpoint, "eth_getBlockByNumber", json!(["latest", false])).await;
        let base_timestamp = quantity(&base["result"]["timestamp"]);

        let response = rpc_request(
            &endpoint,
            "eth_simulateV1",
            json!([{"blockStateCalls": [{}, {}]}, "latest"]),
        )
        .await;
        let blocks = response["result"].as_array().unwrap();
        assert_eq!(quantity(&blocks[0]["timestamp"]), base_timestamp + 2);
        assert_eq!(quantity(&blocks[1]["timestamp"]), base_timestamp + 4);

        api.evm_set_block_timestamp_interval(7).unwrap();
        let response = rpc_request(
            &endpoint,
            "eth_simulateV1",
            json!([{"blockStateCalls": [{}, {}]}, "latest"]),
        )
        .await;
        let blocks = response["result"].as_array().unwrap();
        assert_eq!(quantity(&blocks[0]["timestamp"]), base_timestamp + 7);
        assert_eq!(quantity(&blocks[1]["timestamp"]), base_timestamp + 14);
        assert!(api.evm_remove_block_timestamp_interval().unwrap());

        api.anvil_set_interval_mining(3).unwrap();
        let response =
            rpc_request(&endpoint, "eth_simulateV1", json!([{"blockStateCalls": [{}]}, "latest"]))
                .await;
        assert_eq!(quantity(&response["result"][0]["timestamp"]), base_timestamp + 3);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_rejects_invalid_block_sequence_rpc() {
    let (_api, handle) = spawn(NodeConfig::test().with_genesis_timestamp(Some(1_000u64))).await;
    let endpoint = handle.http_endpoint();
    let base = rpc_request(&endpoint, "eth_getBlockByNumber", json!(["latest", false])).await;
    let base_number = quantity(&base["result"]["number"]);
    let base_timestamp = quantity(&base["result"]["timestamp"]);
    let cases = [
        (
            json!([{"blockStateCalls": [{
                "blockOverrides": {"number": format!("{base_number:#x}")}
            }]}, "latest"]),
            -38020,
        ),
        (
            json!([{"blockStateCalls": [{
                "blockOverrides": {"time": format!("{base_timestamp:#x}")}
            }]}, "latest"]),
            -38021,
        ),
        (
            json!([{"blockStateCalls": [{
                "blockOverrides": {
                    "number": format!("{:#x}", base_number + 2),
                    "time": format!("{:#x}", base_timestamp + 12)
                }
            }]}, "latest"]),
            -38021,
        ),
        (
            json!([{"blockStateCalls": [{
                "blockOverrides": {"number": format!("{:#x}", base_number + 257)}
            }]}, "latest"]),
            -38026,
        ),
    ];

    for (params, code) in cases {
        let response = rpc_request(&endpoint, "eth_simulateV1", params).await;
        assert_eq!(response["error"]["code"], code, "{response}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_chains_block_hashes_rpc() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let base = rpc_request(&endpoint, "eth_getBlockByNumber", json!(["latest", false])).await;
    let base_hash = base["result"]["hash"].clone();
    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [
                {
                    "stateOverrides": {
                        "0xc000000000000000000000000000000000000000": {
                            "code": "0x5f405f5260205ff3"
                        }
                    },
                    "calls": [{
                        "to": "0xc000000000000000000000000000000000000000"
                    }]
                },
                {
                    "stateOverrides": {
                        "0xc100000000000000000000000000000000000000": {
                            "code": "0x6001405f5260205ff3"
                        }
                    },
                    "calls": [{
                        "to": "0xc100000000000000000000000000000000000000"
                    }]
                }
            ]
        }, "latest"]),
    )
    .await;
    assert!(response.get("error").is_none(), "{response}");
    let blocks = response["result"].as_array().unwrap();

    assert_eq!(blocks[0]["parentHash"], base_hash);
    assert_eq!(blocks[0]["calls"][0]["returnData"], base_hash);
    assert_eq!(blocks[1]["parentHash"], blocks[0]["hash"]);
    assert_eq!(blocks[1]["calls"][0]["returnData"], blocks[0]["hash"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_scopes_block_hash_overrides_rpc() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let base = rpc_request(&endpoint, "eth_getBlockByNumber", json!(["latest", false])).await;
    let base_hash = base["result"]["hash"].clone();
    let fake_hash = format!("0x{}", "42".repeat(32));
    let contract = "0xc000000000000000000000000000000000000000";
    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [
                {
                    "blockOverrides": {
                        "blockHash": {"0": fake_hash}
                    },
                    "stateOverrides": {
                        (contract): {"code": "0x5f405f5260205ff3"}
                    },
                    "calls": [{"to": contract}]
                },
                {
                    "calls": [{"to": contract}]
                }
            ]
        }, "latest"]),
    )
    .await;
    assert!(response.get("error").is_none(), "{response}");
    let blocks = response["result"].as_array().unwrap();

    assert_eq!(blocks[0]["calls"][0]["returnData"], fake_hash);
    assert_eq!(blocks[1]["calls"][0]["returnData"], base_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_inherits_parent_block_context_rpc() {
    let config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::London.into()));
    let (_api, handle) = spawn(config).await;
    let endpoint = handle.http_endpoint();
    let contract = "0xc000000000000000000000000000000000000000";
    let fee_recipient = "0xc200000000000000000000000000000000000000";
    let gas_limit = 1_000_000;
    let difficulty = 42;
    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [
                {
                    "blockOverrides": {
                        "gasLimit": format!("{gas_limit:#x}"),
                        "feeRecipient": fee_recipient,
                        "difficulty": format!("{difficulty:#x}")
                    },
                    "stateOverrides": {
                        (contract): {"code": "0x45600052416020524460405260606000f3"}
                    },
                    "calls": [{"to": contract}]
                },
                {"calls": [{"to": contract}]}
            ]
        }, "latest"]),
    )
    .await;
    assert!(response.get("error").is_none(), "{response}");
    let blocks = response["result"].as_array().unwrap();
    let return_data = format!(
        "0x{gas_limit:064x}{}{}{difficulty:064x}",
        "0".repeat(24),
        fee_recipient.trim_start_matches("0x")
    );

    for block in blocks {
        assert_eq!(quantity(&block["gasLimit"]), gas_limit);
        assert_eq!(block["miner"], fee_recipient);
        assert_eq!(block["calls"][0]["returnData"], return_data);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_scopes_block_overrides_and_derives_base_fee_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_base_fee(Some(0));
    let (_api, handle) = spawn(config).await;
    let endpoint = handle.http_endpoint();
    let random = format!("0x{:064x}", 42);
    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [
                {
                    "blockOverrides": {
                        "prevRandao": random,
                        "baseFeePerGas": "0xa",
                        "blobBaseFee": "0x15"
                    },
                    "stateOverrides": {
                        "0xc100000000000000000000000000000000000000": {
                            "code": "0x445f52486020524a60405260605ff3"
                        }
                    },
                    "calls": [{
                        "to": "0xc100000000000000000000000000000000000000"
                    }]
                },
                {
                    "calls": [{
                        "to": "0xc100000000000000000000000000000000000000"
                    }]
                }
            ]
        }, "latest"]),
    )
    .await;
    let blocks = response["result"].as_array().unwrap();
    assert_eq!(blocks[0]["calls"][0]["returnData"], format!("0x{:064x}{:064x}{:064x}", 42, 10, 21));
    assert_eq!(blocks[1]["calls"][0]["returnData"], format!("0x{:064x}{:064x}{:064x}", 0, 0, 1));

    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [
                {"blockOverrides": {"baseFeePerGas": "0x3e8"}},
                {}
            ],
            "validation": true
        }, "latest"]),
    )
    .await;
    assert_eq!(response["result"][0]["baseFeePerGas"], "0x3e8");
    assert_eq!(response["result"][1]["baseFeePerGas"], "0x36b");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_derives_from_historical_base_rpc() {
    let (api, handle) = spawn(NodeConfig::test().with_genesis_timestamp(Some(1_000u64))).await;
    api.mine_one().await;
    let endpoint = handle.http_endpoint();
    let genesis = rpc_request(&endpoint, "eth_getBlockByNumber", json!(["0x0", false])).await;
    let response =
        rpc_request(&endpoint, "eth_simulateV1", json!([{"blockStateCalls": [{}]}, "0x0"])).await;

    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"][0]["number"], "0x1");
    assert_eq!(response["result"][0]["parentHash"], genesis["result"]["hash"]);
    assert_eq!(
        quantity(&response["result"][0]["timestamp"]),
        quantity(&genesis["result"]["timestamp"]) + 12
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_derives_from_pending_base_rpc() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    api.evm_set_block_timestamp_interval(12).unwrap();
    let endpoint = handle.http_endpoint();
    let pending = rpc_request(&endpoint, "eth_getBlockByNumber", json!(["pending", false])).await;
    let response =
        rpc_request(&endpoint, "eth_simulateV1", json!([{"blockStateCalls": [{}]}, "pending"]))
            .await;

    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        quantity(&response["result"][0]["number"]),
        quantity(&pending["result"]["number"]) + 1
    );
    assert_eq!(response["result"][0]["parentHash"], pending["result"]["hash"]);
    assert_eq!(
        quantity(&response["result"][0]["timestamp"]),
        quantity(&pending["result"]["timestamp"]) + 12
    );
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

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_enforces_request_block_limits_rpc() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();

    let response =
        rpc_request(&endpoint, "eth_simulateV1", json!([{"blockStateCalls": []}, "latest"])).await;
    assert_eq!(response["error"], json!({"code": -32602, "message": "empty input"}));

    let blocks = vec![json!({}); 256];
    let response =
        rpc_request(&endpoint, "eth_simulateV1", json!([{"blockStateCalls": blocks}, "latest"]))
            .await;
    assert_eq!(response["result"].as_array().unwrap().len(), 256);

    let blocks = vec![json!({}); 257];
    let response =
        rpc_request(&endpoint, "eth_simulateV1", json!([{"blockStateCalls": blocks}, "latest"]))
            .await;
    assert_eq!(response["error"], json!({"code": -38026, "message": "too many blocks"}));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_future_base_block_returns_header_not_found_rpc() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{"blockStateCalls": [{}]}, "0x1"]),
    )
    .await;

    assert_eq!(response["error"], json!({"code": -32000, "message": "header not found"}));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_enforces_request_gas_budget_rpc() {
    let config = NodeConfig::test().with_base_fee(Some(0)).with_gas_limit(Some(75_398_208));
    let (_api, handle) = spawn(config).await;
    let sender = "0xc000000000000000000000000000000000000000";
    let receiver = "0xc100000000000000000000000000000000000000";
    let reverter = "0xc200000000000000000000000000000000000000";
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [
                {
                    "stateOverrides": {
                        (sender): {"balance": "0x1"},
                        (reverter): {"code": "0x5f5ffd"}
                    },
                    "calls": [{"from": sender, "to": reverter}]
                },
                {"calls": [{"from": sender, "to": receiver}]}
            ],
            "validation": true,
            "returnFullTransactions": true
        }, "latest"]),
    )
    .await;

    assert!(response.get("error").is_none(), "{response}");
    let blocks = response["result"].as_array().unwrap();
    assert_eq!(blocks[0]["calls"][0]["status"], "0x0");
    assert_eq!(blocks[0]["transactions"][0]["gas"], "0x2faf080");
    let failed_call_gas = u64::from_str_radix(
        blocks[0]["calls"][0]["gasUsed"].as_str().unwrap().trim_start_matches("0x"),
        16,
    )
    .unwrap();
    assert_eq!(
        blocks[1]["transactions"][0]["gas"],
        format!("0x{:x}", 50_000_000 - failed_call_gas)
    );
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

fn quantity(value: &Value) -> u64 {
    u64::from_str_radix(value.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
}
