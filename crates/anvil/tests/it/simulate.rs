//! `eth_simulateV1` tests.

use alloy_primitives::{TxKind, U256, address};
use alloy_rpc_types::{
    BlockOverrides,
    request::TransactionRequest,
    simulate::{SimBlock, SimulatePayload},
    state::{AccountOverride, StateOverridesBuilder},
};
use anvil::{NodeConfig, spawn};
use foundry_evm::hardfork::EthereumHardfork;
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
async fn test_simulate_block_sequence_rpc() {
    let (_api, handle) = spawn(NodeConfig::test().with_genesis_timestamp(Some(1_000u64))).await;
    let endpoint = handle.http_endpoint();
    let latest = rpc_request(&endpoint, "eth_getBlockByNumber", json!(["latest", false])).await;
    let base = &latest["result"];
    let base_number = quantity(&base["number"]);
    let base_timestamp = quantity(&base["timestamp"]);
    let first_target_number = base_number + 3;
    let first_target_timestamp = base_timestamp + 100;
    let second_target_number = base_number + 7;

    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [
                {
                    "blockOverrides": {
                        "number": format!("{first_target_number:#x}"),
                        "time": format!("{first_target_timestamp:#x}")
                    }
                },
                {},
                {
                    "blockOverrides": {
                        "number": format!("{second_target_number:#x}")
                    }
                }
            ]
        }, "latest"]),
    )
    .await;
    let blocks = response["result"].as_array().unwrap();

    let summaries = blocks
        .iter()
        .map(|block| {
            json!({
                "number": quantity(&block["number"]),
                "timestamp": quantity(&block["timestamp"]),
                "baseFeePerGas": block["baseFeePerGas"],
                "gasUsed": block["gasUsed"],
                "calls": block["calls"],
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        summaries,
        vec![
            json!({"number": base_number + 1, "timestamp": base_timestamp + 12, "baseFeePerGas": "0x0", "gasUsed": "0x0", "calls": []}),
            json!({"number": base_number + 2, "timestamp": base_timestamp + 24, "baseFeePerGas": "0x0", "gasUsed": "0x0", "calls": []}),
            json!({"number": base_number + 3, "timestamp": base_timestamp + 100, "baseFeePerGas": "0x0", "gasUsed": "0x0", "calls": []}),
            json!({"number": base_number + 4, "timestamp": base_timestamp + 112, "baseFeePerGas": "0x0", "gasUsed": "0x0", "calls": []}),
            json!({"number": base_number + 5, "timestamp": base_timestamp + 124, "baseFeePerGas": "0x0", "gasUsed": "0x0", "calls": []}),
            json!({"number": base_number + 6, "timestamp": base_timestamp + 136, "baseFeePerGas": "0x0", "gasUsed": "0x0", "calls": []}),
            json!({"number": base_number + 7, "timestamp": base_timestamp + 148, "baseFeePerGas": "0x0", "gasUsed": "0x0", "calls": []}),
        ]
    );

    let mut expected_parent = base["hash"].clone();
    for block in blocks {
        assert_eq!(block["parentHash"], expected_parent);
        expected_parent = block["hash"].clone();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_block_override_scoping_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_base_fee(Some(0));
    let (_api, handle) = spawn(config).await;
    let random = format!("0x{:064x}", 42);
    let response = rpc_request(
        &handle.http_endpoint(),
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
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_pending_block_metadata_rpc() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let accounts = rpc_request(&endpoint, "eth_accounts", json!([])).await;
    let accounts = accounts["result"].as_array().unwrap();

    rpc_request(&endpoint, "anvil_setAutomine", json!([false])).await;
    let sent = rpc_request(
        &endpoint,
        "eth_sendTransaction",
        json!([{
            "from": accounts[0],
            "to": accounts[1],
            "value": "0x1",
            "gas": "0x5208"
        }]),
    )
    .await;
    assert!(sent.get("error").is_none(), "failed to queue transaction: {sent}");

    let latest = rpc_request(&endpoint, "eth_getBlockByNumber", json!(["latest", false])).await;
    let pending = rpc_request(&endpoint, "eth_getBlockByNumber", json!(["pending", false])).await;
    let latest = &latest["result"];
    let pending = &pending["result"];
    assert_eq!(quantity(&pending["number"]), quantity(&latest["number"]) + 1);

    let response =
        rpc_request(&endpoint, "eth_simulateV1", json!([{"blockStateCalls": [{}]}, "pending"]))
            .await;
    let simulated = &response["result"][0];
    assert_eq!(quantity(&simulated["number"]), quantity(&pending["number"]) + 1);
    assert_eq!(simulated["parentHash"], pending["hash"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_trace_transfer_forwarding_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_base_fee(Some(0));
    let (_api, handle) = spawn(config).await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": {"balance": "0x7d0"},
                    "0xc100000000000000000000000000000000000000": {
                        "code": "0x60806040526004361061001e5760003560e01c80634b64e49214610023575b600080fd5b61003d6004803603810190610038919061011f565b61003f565b005b60008173ffffffffffffffffffffffffffffffffffffffff166108fc349081150290604051600060405180830381858888f193505050509050806100b8576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016100af906101a9565b60405180910390fd5b5050565b600080fd5b600073ffffffffffffffffffffffffffffffffffffffff82169050919050565b60006100ec826100c1565b9050919050565b6100fc816100e1565b811461010757600080fd5b50565b600081359050610119816100f3565b92915050565b600060208284031215610135576101346100bc565b5b60006101438482850161010a565b91505092915050565b600082825260208201905092915050565b7f4661696c656420746f2073656e64204574686572000000000000000000000000600082015250565b600061019360148361014c565b915061019e8261015d565b602082019050919050565b600060208201905081810360008301526101c281610186565b905091905056fea2646970667358221220563acd6f5b8ad06a3faf5c27fddd0ecbc198408b99290ce50d15c2cf7043694964736f6c63430008120033"
                    }
                },
                "calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc100000000000000000000000000000000000000",
                    "value": "0x3e8",
                    "input": "0x4b64e4920000000000000000000000000000000000000000000000000000000000000100"
                }]
            }],
            "traceTransfers": true
        }, "latest"]),
    )
    .await;
    assert_eq!(response["result"][0]["logsBloom"], format!("0x{}", "00".repeat(256)));
    let call = &response["result"][0]["calls"][0];
    assert_eq!(
        json!({
            "returnData": call["returnData"],
            "gasUsed": call["gasUsed"],
            "maxUsedGas": call["maxUsedGas"],
            "status": call["status"],
            "error": call.get("error"),
        }),
        json!({
            "returnData": "0x",
            "gasUsed": "0xdad2",
            "maxUsedGas": "0xdad2",
            "status": "0x1",
            "error": null,
        })
    );

    let logs = call["logs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|log| {
            json!({
                "address": log["address"],
                "topics": log["topics"],
                "data": log["data"],
                "transactionIndex": log["transactionIndex"],
                "logIndex": log["logIndex"],
                "removed": log["removed"],
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        logs,
        vec![
            json!({
                "address": "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "topics": [
                    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                    "0x000000000000000000000000c000000000000000000000000000000000000000",
                    "0x000000000000000000000000c100000000000000000000000000000000000000"
                ],
                "data": "0x00000000000000000000000000000000000000000000000000000000000003e8",
                "transactionIndex": "0x0",
                "logIndex": "0x0",
                "removed": false
            }),
            json!({
                "address": "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "topics": [
                    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                    "0x000000000000000000000000c100000000000000000000000000000000000000",
                    "0x0000000000000000000000000000000000000000000000000000000000000100"
                ],
                "data": "0x00000000000000000000000000000000000000000000000000000000000003e8",
                "transactionIndex": "0x0",
                "logIndex": "0x1",
                "removed": false
            }),
        ]
    );

    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": {"balance": "0x7d0"},
                    "0xc100000000000000000000000000000000000000": {"code": "0x6000"}
                },
                "calls": [
                    {
                        "from": "0xc000000000000000000000000000000000000000",
                        "to": "0xc100000000000000000000000000000000000000",
                        "value": "0x1",
                        "gas": "0x5208"
                    },
                    {
                        "from": "0xc000000000000000000000000000000000000000",
                        "to": "0xc100000000000000000000000000000000000000",
                        "value": "0x1",
                        "gas": "0x5208"
                    },
                    {
                        "from": "0xc000000000000000000000000000000000000000",
                        "to": "0xc100000000000000000000000000000000000000",
                        "value": "0x1"
                    }
                ]
            }],
            "traceTransfers": true
        }, "latest"]),
    )
    .await;
    let calls = response["result"][0]["calls"].as_array().unwrap();
    assert_eq!(calls[0]["status"], "0x0");
    assert_eq!(calls[0]["logs"], json!([]));
    assert_eq!(calls[1]["status"], "0x0");
    assert_eq!(calls[1]["logs"], json!([]));
    assert_eq!(
        calls[2]["logs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|log| {
                json!({
                    "transactionIndex": log["transactionIndex"],
                    "logIndex": log["logIndex"],
                })
            })
            .collect::<Vec<_>>(),
        vec![json!({"transactionIndex": "0x2", "logIndex": "0x2"})]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_selfdestruct_trace_transfer_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_base_fee(Some(0));
    let (_api, handle) = spawn(config).await;
    let endpoint = handle.http_endpoint();
    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0x0000000000000000000000000000000000000000": {"nonce": "0x1"},
                    "0xc200000000000000000000000000000000000000": {
                        "code": "0x6080604052348015600f57600080fd5b506004361060285760003560e01c806383197ef014602d575b600080fd5b60336035565b005b600073ffffffffffffffffffffffffffffffffffffffff16fffea26469706673582212208e566fde20a17fff9658b9b1db37e27876fd8934ccf9b2aa308cabd37698681f64736f6c63430008120033",
                        "balance": "0x1e8480"
                    }
                },
                "calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc200000000000000000000000000000000000000",
                    "input": "0x83197ef0"
                }]
            }],
            "traceTransfers": true
        }, "latest"]),
    )
    .await;
    let call = &response["result"][0]["calls"][0];
    assert_eq!(call["gasUsed"], "0x664a");
    assert_eq!(call["maxUsedGas"], "0x664a");
    assert_eq!(call["status"], "0x1");
    assert_eq!(
        call["logs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|log| {
                json!({
                    "address": log["address"],
                    "topics": log["topics"],
                    "data": log["data"],
                    "transactionIndex": log["transactionIndex"],
                    "logIndex": log["logIndex"],
                    "removed": log["removed"],
                })
            })
            .collect::<Vec<_>>(),
        vec![json!({
            "address": "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            "topics": [
                "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                "0x000000000000000000000000c200000000000000000000000000000000000000",
                "0x0000000000000000000000000000000000000000000000000000000000000000"
            ],
            "data": "0x00000000000000000000000000000000000000000000000000000000001e8480",
            "transactionIndex": "0x0",
            "logIndex": "0x0",
            "removed": false
        })]
    );

    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0x0000000000000000000000000000000000000000": {"nonce": "0x1"},
                    "0xc200000000000000000000000000000000000000": {
                        "code": "0x6080604052348015600f57600080fd5b506004361060285760003560e01c806383197ef014602d575b600080fd5b60336035565b005b600073ffffffffffffffffffffffffffffffffffffffff16fffea26469706673582212208e566fde20a17fff9658b9b1db37e27876fd8934ccf9b2aa308cabd37698681f64736f6c63430008120033"
                    }
                },
                "calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc200000000000000000000000000000000000000",
                    "input": "0x83197ef0"
                }]
            }],
            "traceTransfers": true
        }, "latest"]),
    )
    .await;
    let call = &response["result"][0]["calls"][0];
    assert_eq!(call["status"], "0x1");
    assert_eq!(call["logs"], json!([]));

    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc200000000000000000000000000000000000000": {
                        "code": "0x730000000000000000000000000000000000000000ff",
                        "balance": "0x1e8480"
                    },
                    "0xc300000000000000000000000000000000000000": {
                        "code": "0x5f5f5f5f5f73c2000000000000000000000000000000000000005af1505f5ffd"
                    }
                },
                "calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc300000000000000000000000000000000000000"
                }]
            }],
            "traceTransfers": true
        }, "latest"]),
    )
    .await;
    let call = &response["result"][0]["calls"][0];
    assert_eq!(call["status"], "0x0");
    assert_eq!(call["logs"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_nested_selfdestruct_logs_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_base_fee(Some(0));
    let (_api, handle) = spawn(config).await;
    let endpoint = handle.http_endpoint();

    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc200000000000000000000000000000000000000": {
                        "code": "0x730000000000000000000000000000000000000000ff",
                        "balance": "0x1e8480"
                    },
                    "0xc300000000000000000000000000000000000000": {
                        "code": "0x5f5f5f5f5f73c2000000000000000000000000000000000000005af1505f5ffd"
                    },
                    "0xc400000000000000000000000000000000000000": {
                        "code": "0x5f5f5f5f5f73c3000000000000000000000000000000000000005af15000"
                    }
                },
                "calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc400000000000000000000000000000000000000"
                }]
            }],
            "traceTransfers": true
        }, "latest"]),
    )
    .await;
    let call = &response["result"][0]["calls"][0];
    assert_eq!(call["status"], "0x1");
    assert_eq!(call["logs"], json!([]));

    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc200000000000000000000000000000000000000": {
                        "code": "0x730000000000000000000000000000000000000000ff",
                        "balance": "0x1e8480"
                    },
                    "0xc400000000000000000000000000000000000000": {
                        "code": "0x5f5fa05f5f5f5f5f73c2000000000000000000000000000000000000005af1505f5fa000"
                    }
                },
                "calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc400000000000000000000000000000000000000"
                }]
            }],
            "traceTransfers": true
        }, "latest"]),
    )
    .await;
    let logs = response["result"][0]["calls"][0]["logs"].as_array().unwrap();
    assert_eq!(
        logs.iter()
            .map(|log| json!({"address": log["address"], "logIndex": log["logIndex"]}))
            .collect::<Vec<_>>(),
        vec![
            json!({"address": "0xc400000000000000000000000000000000000000", "logIndex": "0x0"}),
            json!({"address": "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee", "logIndex": "0x1"}),
            json!({"address": "0xc400000000000000000000000000000000000000", "logIndex": "0x2"}),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_overflow_nonce_validation_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_base_fee(Some(0))
        .with_gas_limit(Some(1_000_000));
    let (_api, handle) = spawn(config).await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": {
                        "nonce": "0xffffffffffffffff"
                    }
                },
                "calls": [
                    {
                        "from": "0xc000000000000000000000000000000000000000",
                        "to": "0xc100000000000000000000000000000000000000"
                    },
                    {
                        "from": "0xc000000000000000000000000000000000000000",
                        "to": "0xc100000000000000000000000000000000000000"
                    }
                ]
            }],
            "validation": true
        }, "latest"]),
    )
    .await;
    assert_eq!(
        response,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32603,
                "message": "err: nonce has max value: address 0xC000000000000000000000000000000000000000, nonce: 18446744073709551615 (supplied gas 1000000)"
            }
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_block_gas_budget_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_base_fee(Some(0));
    let (_api, handle) = spawn(config).await;
    let endpoint = handle.http_endpoint();
    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [
                {
                    "blockOverrides": {"gasLimit": "0x16e360"},
                    "stateOverrides": {
                        "0xc000000000000000000000000000000000000000": {"balance": "0x1e8480"},
                        "0xc200000000000000000000000000000000000000": {
                            "code": "0x608060405234801561001057600080fd5b506004361061002b5760003560e01c8063815b8ab414610030575b600080fd5b61004a600480360381019061004591906100b6565b61004c565b005b60005a90505b60011561007657815a826100669190610112565b106100715750610078565b610052565b505b50565b600080fd5b6000819050919050565b61009381610080565b811461009e57600080fd5b50565b6000813590506100b08161008a565b92915050565b6000602082840312156100cc576100cb61007b565b5b60006100da848285016100a1565b91505092915050565b7f4e487b7100000000000000000000000000000000000000000000000000000000600052601160045260246000fd5b600061011d82610080565b915061012883610080565b92508282039050818111156101405761013f6100e3565b5b9291505056fea2646970667358221220a659ba4db729a6ee4db02fcc5c1118db53246b0e5e686534fc9add6f2e93faec64736f6c63430008120033"
                        }
                    }
                },
                {
                    "calls": [
                        {
                            "from": "0xc000000000000000000000000000000000000000",
                            "to": "0xc200000000000000000000000000000000000000",
                            "input": "0x815b8ab400000000000000000000000000000000000000000000000000000000000f4240"
                        },
                        {
                            "from": "0xc000000000000000000000000000000000000000",
                            "to": "0xc200000000000000000000000000000000000000",
                            "input": "0x815b8ab400000000000000000000000000000000000000000000000000000000000f4240"
                        }
                    ]
                }
            ]
        }, "latest"]),
    )
    .await;
    let block = &response["result"][1];
    assert_eq!(
        json!({
            "gasLimit": block["gasLimit"],
            "gasUsed": block["gasUsed"],
            "calls": block["calls"],
        }),
        json!({
            "gasLimit": "0x16e360",
            "gasUsed": "0x16e360",
            "calls": [
                {
                    "returnData": "0x",
                    "logs": [],
                    "gasUsed": "0xf983f",
                    "maxUsedGas": "0xf983f",
                    "status": "0x1"
                },
                {
                    "returnData": "0x",
                    "logs": [],
                    "gasUsed": "0x74b21",
                    "maxUsedGas": "0x74b21",
                    "status": "0x0",
                    "error": {"code": -32015, "message": "out of gas"}
                }
            ]
        })
    );

    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "blockOverrides": {"gasLimit": "0xa410"},
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": {"balance": "0x1"}
                },
                "calls": [
                    {
                        "from": "0xc000000000000000000000000000000000000000",
                        "to": "0xc100000000000000000000000000000000000000"
                    },
                    {
                        "from": "0xc000000000000000000000000000000000000000",
                        "to": "0xc100000000000000000000000000000000000000",
                        "gas": "0x55f0"
                    }
                ]
            }]
        }, "latest"]),
    )
    .await;
    assert_eq!(
        response,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -38015,
                "message": "block gas limit reached: remaining: 21000, required: 22000"
            }
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_rpc_gas_budget_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_base_fee(Some(0))
        .with_gas_limit(Some(75_398_208));
    let (_api, handle) = spawn(config).await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [
                {
                    "blockOverrides": {"baseFeePerGas": "0xf"},
                    "stateOverrides": {
                        "0xc000000000000000000000000000000000000000": {"balance": "0x3b9aca00"}
                    },
                    "calls": [{
                        "from": "0xc000000000000000000000000000000000000000",
                        "to": "0xc100000000000000000000000000000000000000",
                        "maxFeePerGas": "0x10"
                    }]
                },
                {
                    "calls": [{
                        "from": "0xc000000000000000000000000000000000000000",
                        "to": "0xc100000000000000000000000000000000000000",
                        "maxFeePerGas": "0x10"
                    }]
                }
            ],
            "validation": true,
            "returnFullTransactions": true
        }, "latest"]),
    )
    .await;

    let blocks = response["result"].as_array().unwrap();
    assert_eq!(
        blocks
            .iter()
            .map(|block| {
                json!({
                    "gasLimit": block["gasLimit"],
                    "gasUsed": block["gasUsed"],
                    "calls": block["calls"],
                    "transactionGas": block["transactions"][0]["gas"],
                })
            })
            .collect::<Vec<_>>(),
        vec![
            json!({
                "gasLimit": "0x47e7c40",
                "gasUsed": "0x5208",
                "calls": [{
                    "returnData": "0x",
                    "logs": [],
                    "gasUsed": "0x5208",
                    "maxUsedGas": "0x5208",
                    "status": "0x1"
                }],
                "transactionGas": "0x2faf080"
            }),
            json!({
                "gasLimit": "0x47e7c40",
                "gasUsed": "0x5208",
                "calls": [{
                    "returnData": "0x",
                    "logs": [],
                    "gasUsed": "0x5208",
                    "maxUsedGas": "0x5208",
                    "status": "0x1"
                }],
                "transactionGas": "0x2fa9e78"
            }),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_max_used_gas_before_refund_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_base_fee(Some(0));
    let (_api, handle) = spawn(config).await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc200000000000000000000000000000000000000": {
                        "code": "0x5f5f5500",
                        "state": {
                            "0x0000000000000000000000000000000000000000000000000000000000000000": "0x0000000000000000000000000000000000000000000000000000000000000001"
                        }
                    }
                },
                "calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc200000000000000000000000000000000000000"
                }]
            }]
        }, "latest"]),
    )
    .await;
    let call = &response["result"][0]["calls"][0];
    assert_eq!(call["gasUsed"], "0x52d4");
    assert_eq!(call["maxUsedGas"], "0x6594");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_creation_and_revert_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_base_fee(Some(0));
    let (_api, handle) = spawn(config).await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000099": {"code": "0x60006000fd"}
                },
                "calls": [
                    {},
                    {
                        "from": "0xc000000000000000000000000000000000000001",
                        "to": "0xc000000000000000000000000000000000000099"
                    }
                ]
            }],
            "returnFullTransactions": true
        }, "latest"]),
    )
    .await;
    let block = &response["result"][0];
    assert_eq!(
        block["calls"],
        json!([
            {
                "returnData": "0x",
                "logs": [],
                "gasUsed": "0xcf08",
                "maxUsedGas": "0xcf08",
                "status": "0x1"
            },
            {
                "returnData": "0x",
                "logs": [],
                "gasUsed": "0x520e",
                "maxUsedGas": "0x520e",
                "status": "0x0",
                "error": {"code": 3, "message": "execution reverted", "data": "0x"}
            }
        ])
    );
    assert_eq!(block["transactions"][0]["to"], Value::Null);
    for (transaction_index, transaction) in
        block["transactions"].as_array().unwrap().iter().enumerate()
    {
        assert_eq!(transaction["blockHash"], block["hash"]);
        assert_eq!(transaction["blockNumber"], block["number"]);
        assert_eq!(quantity(&transaction["transactionIndex"]), transaction_index as u64);
        assert_eq!(transaction["blockTimestamp"], block["timestamp"]);
    }

    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": {"balance": "0x7d0"},
                    "0xc100000000000000000000000000000000000000": {
                        "code": "0x608060405260006042576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401603990609d565b60405180910390fd5b005b600082825260208201905092915050565b7f416c7761797320726576657274696e6720636f6e747261637400000000000000600082015250565b600060896019836044565b91506092826055565b602082019050919050565b6000602082019050818103600083015260b481607e565b905091905056fea264697066735822122005cbbbc709291f66fadc17416c1b0ed4d72941840db11468a21b8e1a0362024c64736f6c63430008120033"
                    }
                },
                "calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc100000000000000000000000000000000000000",
                    "value": "0x3e8"
                }]
            }],
            "traceTransfers": true
        }, "latest"]),
    )
    .await;
    assert_eq!(
        response["result"][0]["calls"][0],
        json!({
            "returnData": "0x",
            "logs": [],
            "gasUsed": "0x5355",
            "maxUsedGas": "0x5355",
            "status": "0x0",
            "error": {
                "code": 3,
                "message": "execution reverted: Always reverting contract",
                "data": "0x08c379a000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000019416c7761797320726576657274696e6720636f6e747261637400000000000000"
            }
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_validation_errors_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_base_fee(Some(0))
        .with_gas_limit(Some(50_000_000));
    let (_api, handle) = spawn(config).await;
    let endpoint = handle.http_endpoint();

    let cases = [
        (
            json!([{
                "blockStateCalls": [{
                    "blockOverrides": {"baseFeePerGas": "0xa"},
                    "calls": [{
                        "from": "0xc000000000000000000000000000000000000000",
                        "to": "0xc100000000000000000000000000000000000000",
                        "value": "0x3e8",
                        "maxFeePerGas": "0x0"
                    }]
                }],
                "validation": true
            }, "latest"]),
            json!({
                "code": -38012,
                "message": "err: max fee per gas less than block base fee: address 0xC000000000000000000000000000000000000000, maxFeePerGas: 0, baseFee: 10 (supplied gas 50000000)"
            }),
        ),
        (
            json!([{"blockStateCalls": [{"calls": [{"gas": "0x0"}]}]}, "latest"]),
            json!({
                "code": -38013,
                "message": "err: intrinsic gas too high -- CallGasCostMoreThanGasLimit"
            }),
        ),
        (
            json!([{
                "blockStateCalls": [{"calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc100000000000000000000000000000000000000",
                    "value": "0x3e8"
                }]}]
            }, "latest"]),
            json!({
                "code": -38014,
                "message": "err: Insufficient funds for gas * price + value"
            }),
        ),
        (
            json!([{"blockStateCalls": [{}]}, "0x1"]),
            json!({"code": -32000, "message": "header not found"}),
        ),
        (
            json!([{
                "blockStateCalls": [{"stateOverrides": {
                    "0xc000000000000000000000000000000000000000": {
                        "movePrecompileToAddress": "0xc100000000000000000000000000000000000000"
                    }
                }}]
            }, "latest"]),
            json!({
                "code": -32000,
                "message": "account 0xC000000000000000000000000000000000000000 is not a precompile"
            }),
        ),
    ];

    for (params, expected_error) in cases {
        let response = rpc_request(&endpoint, "eth_simulateV1", params).await;
        assert_eq!(response, json!({"jsonrpc": "2.0", "id": 1, "error": expected_error}));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_move_precompile_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_base_fee(Some(0));
    let (_api, handle) = spawn(config).await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0x0000000000000000000000000000000000000004": {
                        "movePrecompileToAddress": "0xc100000000000000000000000000000000000000"
                    },
                    "0xc200000000000000000000000000000000000000": {
                        "code": "0x5f5f5f5f73c1000000000000000000000000000000000000005afa5000"
                    }
                },
                "calls": [
                    {"to": "0xc100000000000000000000000000000000000000", "input": "0x1234"},
                    {"to": "0x0000000000000000000000000000000000000004", "input": "0x1234"},
                    {"to": "0xc200000000000000000000000000000000000000"}
                ]
            }]
        }, "latest"]),
    )
    .await;
    let calls = &response["result"][0]["calls"];
    assert_eq!(
        calls
            .as_array()
            .unwrap()
            .iter()
            .map(|call| json!({"returnData": call["returnData"], "status": call["status"]}))
            .collect::<Vec<_>>(),
        vec![
            json!({"returnData": "0x1234", "status": "0x1"}),
            json!({"returnData": "0x", "status": "0x1"}),
            json!({"returnData": "0x", "status": "0x1"}),
        ]
    );
    assert_eq!(calls[2]["gasUsed"], "0x5c4e");
    assert_eq!(calls[2]["maxUsedGas"], "0x5c4e");
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
