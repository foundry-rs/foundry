//! general eth api tests

use alloy_consensus::constants::EMPTY_WITHDRAWALS;

use alloy_eips::{
    eip6110::DEPOSIT_REQUEST_TYPE,
    eip7002::{WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS, WITHDRAWAL_REQUEST_TYPE},
    eip7251::{CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS, CONSOLIDATION_REQUEST_TYPE},
    eip7685::{EMPTY_REQUESTS_HASH, Requests},
};
use alloy_evm::precompiles::{DynPrecompile, PrecompileInput, PrecompilesMap};
use alloy_genesis::Genesis;
use alloy_primitives::{Address, B256, Bloom, Bytes, Log, TxKind, U256, address};
use alloy_provider::Provider;
use alloy_rpc_types::{
    BlockNumberOrTag, BlockOverrides,
    request::TransactionRequest,
    simulate::{SimBlock, SimulatePayload},
    state::{AccountOverride, StateOverridesBuilder},
};
use alloy_serde::WithOtherFields;
use alloy_sol_types::{SolEvent, sol};
use anvil::{EthereumHardfork, NodeConfig, PrecompileFactory, spawn};
use axum::{Json, Router, routing::post};
use foundry_test_utils::rpc;
use revm::precompile::{PrecompileError, PrecompileOutput, PrecompileStatus};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

sol! {
    event DepositEvent(
        bytes pubkey,
        bytes withdrawal_credentials,
        bytes amount,
        bytes signature,
        bytes index
    );
}

fn deposit_event_runtime(event: DepositEvent) -> Bytes {
    let log = DepositEvent::encode_log(&Log { address: Address::ZERO, data: event });
    let data = log.data.data.as_ref();
    let data_len = u16::try_from(data.len()).unwrap();
    const CODE_OFFSET: u16 = 47;
    let mut code = Vec::with_capacity(usize::from(CODE_OFFSET) + data.len());
    code.extend([0x61]);
    code.extend(data_len.to_be_bytes());
    code.extend([0x61]);
    code.extend(CODE_OFFSET.to_be_bytes());
    code.extend([0x5f, 0x39, 0x7f]);
    code.extend(DepositEvent::SIGNATURE_HASH);
    code.extend([0x61]);
    code.extend(data_len.to_be_bytes());
    code.extend([0x5f, 0xa1, 0x00]);
    code.extend(data);
    code.into()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_simulate_native_transfers_rpc() {
    crate::init_tracing();
    let (_, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(rpc::next_http_archive_rpc_url()))
            .with_fork_block_number(Some(24_000_000u64)),
    )
    .await;
    let from = address!("0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
    let payload = SimulatePayload {
        block_state_calls: vec![SimBlock {
            calls: vec![
                TransactionRequest {
                    from: Some(from),
                    to: Some(TxKind::from(address!("0x1000000000000000000000000000000000000001"))),
                    value: Some(U256::from(1)),
                    ..Default::default()
                },
                TransactionRequest {
                    from: Some(from),
                    to: Some(TxKind::from(address!("0x1000000000000000000000000000000000000002"))),
                    value: Some(U256::from(1)),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        validation: false,
        ..Default::default()
    };
    let response =
        rpc_request(&handle.http_endpoint(), "eth_simulateV1", json!([payload, "latest"])).await;
    assert!(response.get("error").is_none(), "{response}");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_simulate_multiple_blocks_rpc() {
    let (source_api, source_handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Shanghai.into()))).await;
    let contract = Address::with_last_byte(0x42);
    source_api
        .anvil_set_code(
            contract,
            Bytes::from_static(&[0x60, 0x01, 0x43, 0x60, 0x01, 0x03, 0x40, 0x55, 0x00]),
        )
        .await
        .unwrap();
    source_api.mine_one().await.unwrap();
    let accounts = source_handle.dev_accounts().take(2).collect::<Vec<_>>();
    let payload = json!({
        "blockStateCalls": [
            {
                "calls": [{
                    "from": accounts[0],
                    "to": accounts[1],
                    "value": "0x1"
                }]
            },
            {
                "calls": [{
                    "from": accounts[0],
                    "to": contract
                }]
            }
        ],
        "validation": false
    });

    let (_, fork_handle) = spawn(
        NodeConfig::test()
            .with_hardfork(Some(EthereumHardfork::Shanghai.into()))
            .with_eth_rpc_url(Some(source_handle.http_endpoint()))
            .with_fork_block_number(Some(1u64)),
    )
    .await;
    let response =
        rpc_request(&fork_handle.http_endpoint(), "eth_simulateV1", json!([payload, "latest"]))
            .await;
    assert!(response.get("error").is_none(), "{response}");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_simulate_post_merge_block_context_rpc() {
    let (source_api, source_handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()))).await;
    source_api.anvil_set_next_block_prevrandao(B256::repeat_byte(0x42)).await.unwrap();
    source_api.mine_one().await.unwrap();

    let (_, fork_handle) = spawn(
        NodeConfig::test()
            .with_hardfork(Some(EthereumHardfork::Prague.into()))
            .with_eth_rpc_url(Some(source_handle.http_endpoint()))
            .with_fork_block_number(Some(1u64)),
    )
    .await;
    let contract = "0xc000000000000000000000000000000000000000";
    let response = rpc_request(
        &fork_handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    (contract): {"code": "0x445f5260205ff3"}
                },
                "calls": [{"to": contract}]
            }]
        }, "latest"]),
    )
    .await;
    assert!(response.get("error").is_none(), "{response}");

    let block = &response["result"][0];
    assert_eq!(block["difficulty"], "0x0");
    assert_eq!(block["mixHash"], B256::ZERO.to_string());
    assert_eq!(block["calls"][0]["returnData"], format!("0x{}", "00".repeat(32)));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_v1_moves_precompiles_per_block_rpc() {
    let (_, handle) = spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let source = "0x0000000000000000000000000000000000000004";
    let destination = "0x0000000000000000000000000000000000123456";
    let return_42 = "0x000000000000000000000000000000000000000000000000000000000000002a";

    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [
                {
                    "stateOverrides": {
                        source: {
                            "code": "0x602a60005260206000f3",
                            "movePrecompileToAddress": destination
                        }
                    },
                    "calls": [
                        {"to": source, "input": "0x1234"},
                        {"to": destination, "input": "0x1234"}
                    ]
                },
                {
                    "calls": [
                        {"to": source, "input": "0x1234"},
                        {"to": destination, "input": "0x1234"}
                    ]
                }
            ]
        }, "latest"]),
    )
    .await;
    assert!(response.get("error").is_none(), "{response}");

    let blocks = response["result"].as_array().unwrap();
    assert_eq!(blocks[0]["calls"][0]["returnData"], return_42);
    assert_eq!(blocks[0]["calls"][1]["returnData"], "0x1234");
    assert_eq!(blocks[1]["calls"][0]["returnData"], "0x1234");
    assert_eq!(blocks[1]["calls"][1]["returnData"], "0x");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_v1_preserves_precompile_warming_rpc() {
    let (_, handle) = spawn(NodeConfig::test()).await;
    let source = "0x0000000000000000000000000000000000000004";
    let destination = "0x0000000000000000000000000000000000123456";
    let helper = "0x0000000000000000000000000000000000001000";
    let helper_code = format!(
        "0x5a73{}31505a90035f525a73{}31505a90036020525a73{}31505a900360405260605ff3",
        &source[2..],
        &destination[2..],
        &destination[2..],
    );

    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{"blockStateCalls": [{
            "stateOverrides": {
                source: {"movePrecompileToAddress": destination},
                helper: {"code": helper_code}
            },
            "calls": [{"to": helper}]
        }]}, "latest"]),
    )
    .await;
    assert!(response.get("error").is_none(), "{response}");

    let return_data = response["result"][0]["calls"][0]["returnData"].as_str().unwrap();
    let return_data = alloy_primitives::hex::decode(&return_data[2..]).unwrap();
    let access_costs = return_data
        .as_chunks::<32>()
        .0
        .iter()
        .copied()
        .map(U256::from_be_bytes)
        .collect::<Vec<_>>();

    // The measured delta includes PUSH20, POP, and the second GAS opcode (7 gas total).
    assert_eq!(access_costs, [U256::from(107), U256::from(2_607), U256::from(107)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_v1_validates_precompile_moves_rpc() {
    let (_, handle) = spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let precompile_one = "0x0000000000000000000000000000000000000001";
    let precompile_two = "0x0000000000000000000000000000000000000002";
    let not_precompile = "0xc100000000000000000000000000000000000000";
    let destination = "0xc200000000000000000000000000000000000000";

    let cases = [
        (
            json!([{"blockStateCalls": [{"stateOverrides": {
                not_precompile: {"movePrecompileToAddress": not_precompile}
            }}]}, "latest"]),
            -32000,
        ),
        (
            json!([{"blockStateCalls": [{"stateOverrides": {
                precompile_one: {"movePrecompileToAddress": precompile_one}
            }}]}, "latest"]),
            -38022,
        ),
        (
            json!([{"blockStateCalls": [{"stateOverrides": {
                not_precompile: {"movePrecompileToAddress": destination},
                "0xc300000000000000000000000000000000000000": {
                    "movePrecompileToAddress": destination
                }
            }}]}, "latest"]),
            -32000,
        ),
        (
            json!([{"blockStateCalls": [{"stateOverrides": {
                precompile_one: {"movePrecompileToAddress": destination},
                precompile_two: {"movePrecompileToAddress": destination}
            }}]}, "latest"]),
            -38023,
        ),
    ];

    for (params, expected_code) in cases {
        let response = rpc_request(&endpoint, "eth_simulateV1", params).await;
        assert_eq!(response["error"]["code"], expected_code, "{response}");
    }

    let (_, handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Homestead.into()))).await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{"blockStateCalls": [{"stateOverrides": {
            "0x0000000000000000000000000000000000000005": {
                "movePrecompileToAddress": destination
            }
        }}]}, "latest"]),
    )
    .await;
    assert_eq!(response["error"]["code"], -32000, "{response}");
}

#[derive(Clone, Copy, Debug)]
struct LookupOnlyPrecompileFactory(Address);

impl PrecompileFactory for LookupOnlyPrecompileFactory {
    fn precompiles(&self) -> Vec<(Address, DynPrecompile)> {
        Vec::new()
    }

    fn install(&self, precompiles: &mut PrecompilesMap) {
        let lookup_address = self.0;
        precompiles.set_precompile_lookup(move |address: &Address| {
            (*address == lookup_address).then(|| {
                DynPrecompile::from(|input: PrecompileInput<'_>| {
                    Ok(PrecompileOutput {
                        status: PrecompileStatus::Success,
                        bytes: Bytes::copy_from_slice(input.data),
                        gas_used: 0,
                        gas_refunded: 0,
                        state_gas_used: 0,
                        state_gas_spilled: 0,
                        reservoir: input.reservoir,
                    })
                })
            })
        });
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_v1_rejects_lookup_only_precompile_move_rpc() {
    let source = address!("0xdead000000000000000000000000000000000071");
    let (_, handle) =
        spawn(NodeConfig::test().with_precompile_factory(LookupOnlyPrecompileFactory(source)))
            .await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{"blockStateCalls": [{"stateOverrides": {
            (source.to_string()): {
                "movePrecompileToAddress": "0x0000000000000000000000000000000000123456"
            }
        }}]}, "latest"]),
    )
    .await;

    assert_eq!(response["error"]["code"], -32000, "{response}");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_v1_rejects_precompile_moves_on_tempo_rpc() {
    let (_, handle) = spawn(NodeConfig::test_tempo()).await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{"blockStateCalls": [{"stateOverrides": {
            "0x0000000000000000000000000000000000000004": {
                "movePrecompileToAddress": "0x0000000000000000000000000000000000123456"
            }
        }}]}, "latest"]),
    )
    .await;

    assert_eq!(response["error"]["code"], -32000, "{response}");
}

#[cfg(feature = "monad")]
#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_v1_rejects_precompile_moves_on_monad_rpc() {
    let (_, handle) = spawn(NodeConfig::test_monad()).await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{"blockStateCalls": [{"stateOverrides": {
            "0x0000000000000000000000000000000000000004": {
                "movePrecompileToAddress": "0x0000000000000000000000000000000000123456"
            }
        }}]}, "latest"]),
    )
    .await;

    assert_eq!(response["error"]["code"], -32000, "{response}");
}

#[cfg(feature = "optimism")]
#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_v1_rejects_precompile_moves_on_optimism_rpc() {
    let (_, handle) = spawn(NodeConfig::test().with_optimism()).await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{"blockStateCalls": [{"stateOverrides": {
            "0x0000000000000000000000000000000000000004": {
                "movePrecompileToAddress": "0x0000000000000000000000000000000000123456"
            }
        }}]}, "latest"]),
    )
    .await;

    assert_eq!(response["error"]["code"], -32000, "{response}");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_simulate_normalizes_delegated_block_sequence_rpc() {
    let (origin_api, origin_handle) =
        spawn(NodeConfig::test().with_genesis_timestamp(Some(1_000u64))).await;
    origin_api.mine_one().await.unwrap();
    origin_api.mine_one().await.unwrap();

    let (api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
            .with_fork_block_number(Some(2u64)),
    )
    .await;
    api.evm_set_block_timestamp_interval(7).unwrap();

    let endpoint = handle.http_endpoint();
    let base = rpc_request(&endpoint, "eth_getBlockByNumber", json!(["0x1", false])).await;
    let base_number = quantity(&base["result"]["number"]);
    let base_timestamp = quantity(&base["result"]["timestamp"]);
    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "blockOverrides": {"number": format!("{:#x}", base_number + 3)}
            }]
        }, "0x1"]),
    )
    .await;
    assert!(response.get("error").is_none(), "{response}");

    let blocks = response["result"].as_array().unwrap();
    assert_eq!(blocks.len(), 3);
    for (index, block) in blocks.iter().enumerate() {
        let offset = index as u64 + 1;
        assert_eq!(quantity(&block["number"]), base_number + offset);
        assert_eq!(quantity(&block["timestamp"]), base_timestamp + 7 * offset);
    }

    let cases = [
        (
            json!([{"blockStateCalls": [{
                "blockOverrides": {"number": format!("{base_number:#x}")}
            }]}, "0x1"]),
            -38020,
        ),
        (
            json!([{"blockStateCalls": [{
                "blockOverrides": {"time": format!("{base_timestamp:#x}")}
            }]}, "0x1"]),
            -38021,
        ),
        (
            json!([{"blockStateCalls": [{
                "blockOverrides": {"number": format!("{:#x}", base_number + 257)}
            }]}, "0x1"]),
            -38026,
        ),
    ];
    for (params, code) in cases {
        let response = rpc_request(&endpoint, "eth_simulateV1", params).await;
        assert_eq!(response["error"]["code"], code, "{response}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_simulate_preserves_delegated_base_selector_rpc() {
    let (hash_api, hash_handle) =
        spawn(NodeConfig::test().with_genesis_timestamp(Some(1_000u64))).await;
    hash_api.evm_set_block_timestamp_interval(1).unwrap();
    hash_api.mine_one().await.unwrap();
    hash_api.mine_one().await.unwrap();
    let hash_endpoint = hash_handle.http_endpoint();
    let hash_base =
        rpc_request(&hash_endpoint, "eth_getBlockByNumber", json!(["0x1", false])).await;
    let hash_selector = hash_base["result"]["hash"].clone();

    let (canonical_api, canonical_handle) =
        spawn(NodeConfig::test().with_genesis_timestamp(Some(2_000u64))).await;
    canonical_api.evm_set_block_timestamp_interval(1).unwrap();
    canonical_api.mine_one().await.unwrap();
    canonical_api.mine_one().await.unwrap();
    let canonical_endpoint = canonical_handle.http_endpoint();
    let canonical_base =
        rpc_request(&canonical_endpoint, "eth_getBlockByNumber", json!(["0x1", false])).await;
    let canonical_hash = canonical_base["result"]["hash"].clone();

    let proxy_endpoint =
        spawn_hash_aware_rpc_proxy(hash_endpoint, canonical_endpoint, hash_selector.clone()).await;
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(proxy_endpoint.clone()))
            .with_fork_block_number(Some(2u64)),
    )
    .await;
    api.evm_set_block_timestamp_interval(7).unwrap();
    let endpoint = handle.http_endpoint();

    // Cache the selected block, then replace the same-height cache mapping with another block.
    rpc_request(&endpoint, "eth_getBlockByHash", json!([hash_selector, false])).await;
    rpc_request(&endpoint, "eth_getBlockByHash", json!([canonical_hash, false])).await;

    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{"blockStateCalls": [{}]}, {"blockHash": hash_selector}]),
    )
    .await;
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        quantity(&response["result"][0]["timestamp"]),
        quantity(&hash_base["result"]["timestamp"]) + 7
    );

    let (api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(proxy_endpoint))
            .with_fork_block_number(Some(2u64)),
    )
    .await;
    api.evm_set_block_timestamp_interval(7).unwrap();
    let endpoint = handle.http_endpoint();

    // Cache a noncanonical block at the selected height before using a numeric selector.
    rpc_request(&endpoint, "eth_getBlockByHash", json!([hash_selector, false])).await;
    let response =
        rpc_request(&endpoint, "eth_simulateV1", json!([{"blockStateCalls": [{}]}, "0x1"])).await;
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        quantity(&response["result"][0]["timestamp"]),
        quantity(&canonical_base["result"]["timestamp"]) + 7
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_simulate_pins_delegated_tag_selector_rpc() {
    let (origin_api, origin_handle) =
        spawn(NodeConfig::test().with_genesis_timestamp(Some(1_000u64)).with_slots_in_an_epoch(2))
            .await;
    origin_api.evm_set_block_timestamp_interval(1).unwrap();
    origin_api.anvil_mine(Some(U256::from(4)), None).await.unwrap();

    let (api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
            .with_fork_block_number(Some(4u64))
            .with_slots_in_an_epoch(2),
    )
    .await;
    api.evm_set_block_timestamp_interval(1).unwrap();

    // Advance the upstream safe tag beyond the fork while the local tag remains at block 2.
    origin_api.anvil_mine(Some(U256::from(4)), None).await.unwrap();
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{"blockStateCalls": [{}]}, "safe"]),
    )
    .await;

    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(quantity(&response["result"][0]["number"]), 3);
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
async fn test_simulate_includes_prevrandao_in_chained_hash_rpc() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let contract = "0xc000000000000000000000000000000000000000";
    let mut hashes = Vec::new();

    for random in [42, 43] {
        let random = format!("0x{random:064x}");
        let response = rpc_request(
            &endpoint,
            "eth_simulateV1",
            json!([{
                "blockStateCalls": [
                    {
                        "blockOverrides": {"prevRandao": random}
                    },
                    {
                        "stateOverrides": {
                            (contract): {"code": "0x6001405f5260205ff3"}
                        },
                        "calls": [{"to": contract}]
                    }
                ]
            }, "latest"]),
        )
        .await;
        assert!(response.get("error").is_none(), "{response}");
        let blocks = response["result"].as_array().unwrap();

        assert_eq!(blocks[0]["mixHash"], random);
        assert_eq!(blocks[1]["parentHash"], blocks[0]["hash"]);
        assert_eq!(blocks[1]["calls"][0]["returnData"], blocks[0]["hash"]);
        hashes.push(blocks[0]["hash"].clone());
    }

    assert_ne!(hashes[0], hashes[1]);
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
        assert_eq!(quantity(&block["difficulty"]), difficulty);
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
async fn test_simulate_pre_london_blocks_keep_base_fee_disabled_rpc() {
    let (_api, handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Berlin.into()))).await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{}, {}],
            "validation": true
        }, "latest"]),
    )
    .await;

    assert!(response.get("error").is_none(), "{response}");
    let blocks = response["result"].as_array().unwrap();
    assert_eq!(blocks.len(), 2);
    assert!(blocks.iter().all(|block| block.get("baseFeePerGas").is_none()));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_derives_from_historical_base_rpc() {
    let (api, handle) = spawn(NodeConfig::test().with_genesis_timestamp(Some(1_000u64))).await;
    api.mine_one().await.unwrap();
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
async fn test_simulate_returns_unchanged_state_root_rpc() {
    let (api, handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Shanghai.into()))).await;
    let endpoint = handle.http_endpoint();
    let state_root = api.state_root().await.unwrap();
    let response =
        rpc_request(&endpoint, "eth_simulateV1", json!([{"blockStateCalls": [{}]}])).await;

    assert!(response.get("error").is_none(), "{response}");
    assert_ne!(
        response["result"][0]["stateRoot"],
        "0x0000000000000000000000000000000000000000000000000000000000000000"
    );
    assert_eq!(response["result"][0]["stateRoot"], serde_json::to_value(state_root).unwrap());
    assert_eq!(response["result"][0]["withdrawals"], json!([]));
    assert_eq!(
        response["result"][0]["withdrawalsRoot"],
        serde_json::to_value(EMPTY_WITHDRAWALS).unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulated_and_mined_ethereum_transitions_match_rpc() {
    for hardfork in [EthereumHardfork::Cancun, EthereumHardfork::Prague] {
        let (api, handle) = spawn(NodeConfig::test().with_hardfork(Some(hardfork.into()))).await;
        let provider = handle.http_provider();
        let genesis = provider
            .get_block_by_number(BlockNumberOrTag::Number(0))
            .await
            .unwrap()
            .expect("genesis block should exist");
        let next_timestamp = genesis.header.timestamp + 12;

        let simulated = api
            .simulate_v1(
                SimulatePayload {
                    block_state_calls: vec![SimBlock::default()],
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap()
            .remove(0);

        api.evm_set_next_block_timestamp(next_timestamp).unwrap();
        api.mine_one().await.unwrap();
        let mined = provider
            .get_block_by_number(BlockNumberOrTag::Number(1))
            .await
            .unwrap()
            .expect("mined block should exist");

        assert_eq!(simulated.inner.header.state_root, mined.header.state_root);
        assert_eq!(
            simulated.inner.header.parent_beacon_block_root,
            mined.header.parent_beacon_block_root
        );
        assert_eq!(mined.header.parent_beacon_block_root, Some(B256::ZERO));

        if hardfork >= EthereumHardfork::Prague {
            assert_eq!(simulated.inner.header.requests_hash, mined.header.requests_hash);
            assert_eq!(mined.header.requests_hash, Some(EMPTY_REQUESTS_HASH));
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_preserves_beacon_root_override_rpc() {
    let (_, handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()))).await;
    let provider = handle.http_provider();
    let genesis = provider
        .get_block_by_number(BlockNumberOrTag::Number(0))
        .await
        .unwrap()
        .expect("genesis block should exist");
    let timestamp = genesis.header.timestamp + 12;
    let beacon_root = B256::repeat_byte(0x42);
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "blockOverrides": {
                    "time": format!("0x{timestamp:x}"),
                    "beaconRoot": beacon_root,
                }
            }]
        }]),
    )
    .await;
    let default_root_response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "blockOverrides": {"time": format!("0x{timestamp:x}")}
            }]
        }]),
    )
    .await;

    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"][0]["parentBeaconBlockRoot"], beacon_root.to_string());
    assert_ne!(response["result"][0]["stateRoot"], default_root_response["result"][0]["stateRoot"]);
}

#[derive(Clone, Copy, Debug)]
struct RequestOutputPrecompileFactory;

impl PrecompileFactory for RequestOutputPrecompileFactory {
    fn precompiles(&self) -> Vec<(Address, DynPrecompile)> {
        vec![
            (
                WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
                DynPrecompile::from(|input: PrecompileInput<'_>| {
                    Ok(PrecompileOutput {
                        status: PrecompileStatus::Success,
                        bytes: Bytes::from_static(&[0xaa]),
                        gas_used: 0,
                        gas_refunded: 0,
                        state_gas_used: 0,
                        state_gas_spilled: 0,
                        reservoir: input.reservoir,
                    })
                }),
            ),
            (
                CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
                DynPrecompile::from(|input: PrecompileInput<'_>| {
                    Ok(PrecompileOutput {
                        status: PrecompileStatus::Success,
                        bytes: Bytes::from_static(&[0xbb]),
                        gas_used: 0,
                        gas_refunded: 0,
                        state_gas_used: 0,
                        state_gas_spilled: 0,
                        reservoir: input.reservoir,
                    })
                }),
            ),
        ]
    }
}

#[derive(Clone, Debug)]
struct PerBlockRequestPrecompileFactory(Arc<AtomicUsize>);

impl PrecompileFactory for PerBlockRequestPrecompileFactory {
    fn precompiles(&self) -> Vec<(Address, DynPrecompile)> {
        let calls = Arc::clone(&self.0);
        vec![(
            WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
            DynPrecompile::from(move |input: PrecompileInput<'_>| {
                let value = calls.fetch_add(1, Ordering::SeqCst) as u8 + 1;
                Ok(PrecompileOutput {
                    status: PrecompileStatus::Success,
                    bytes: Bytes::from(vec![value]),
                    gas_used: 0,
                    gas_refunded: 0,
                    state_gas_used: 0,
                    state_gas_spilled: 0,
                    reservoir: input.reservoir,
                })
            }),
        )]
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multiblock_simulation_applies_transitions_per_block_rpc() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (api, _) = spawn(
        NodeConfig::test()
            .with_hardfork(Some(EthereumHardfork::Prague.into()))
            .with_precompile_factory(PerBlockRequestPrecompileFactory(Arc::clone(&calls))),
    )
    .await;
    let live_state_root = api.state_root().await.unwrap();
    let first_beacon_root = B256::repeat_byte(0x11);
    let second_beacon_root = B256::repeat_byte(0x22);

    let blocks = api
        .simulate_v1(
            SimulatePayload {
                block_state_calls: vec![
                    SimBlock {
                        block_overrides: Some(BlockOverrides {
                            beacon_root: Some(first_beacon_root),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    SimBlock {
                        block_overrides: Some(BlockOverrides {
                            beacon_root: Some(second_beacon_root),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();

    let first_requests =
        Requests::new(vec![Bytes::from(vec![WITHDRAWAL_REQUEST_TYPE, 1])]).requests_hash();
    let second_requests =
        Requests::new(vec![Bytes::from(vec![WITHDRAWAL_REQUEST_TYPE, 2])]).requests_hash();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(blocks[0].inner.header.parent_beacon_block_root, Some(first_beacon_root));
    assert_eq!(blocks[1].inner.header.parent_beacon_block_root, Some(second_beacon_root));
    assert_eq!(blocks[0].inner.header.requests_hash, Some(first_requests));
    assert_eq!(blocks[1].inner.header.requests_hash, Some(second_requests));
    assert_eq!(blocks[1].inner.header.parent_hash, blocks[0].inner.header.hash);
    assert_ne!(blocks[0].inner.header.state_root, blocks[1].inner.header.state_root);
    assert_eq!(api.state_root().await.unwrap(), live_state_root);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulated_and_mined_prague_request_hashes_match_rpc() {
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_hardfork(Some(EthereumHardfork::Prague.into()))
            .with_precompile_factory(RequestOutputPrecompileFactory),
    )
    .await;
    let provider = handle.http_provider();
    let genesis = provider
        .get_block_by_number(BlockNumberOrTag::Number(0))
        .await
        .unwrap()
        .expect("genesis block should exist");

    let simulated = api
        .simulate_v1(
            SimulatePayload { block_state_calls: vec![SimBlock::default()], ..Default::default() },
            None,
        )
        .await
        .unwrap()
        .remove(0);

    api.evm_set_next_block_timestamp(genesis.header.timestamp + 12).unwrap();
    api.mine_one().await.unwrap();
    let mined = provider
        .get_block_by_number(BlockNumberOrTag::Number(1))
        .await
        .unwrap()
        .expect("mined block should exist");
    let expected = Requests::new(vec![
        Bytes::from(vec![WITHDRAWAL_REQUEST_TYPE, 0xaa]),
        Bytes::from(vec![CONSOLIDATION_REQUEST_TYPE, 0xbb]),
    ])
    .requests_hash();

    assert_eq!(simulated.inner.header.state_root, mined.header.state_root);
    assert_eq!(simulated.inner.header.requests_hash, Some(expected));
    assert_eq!(mined.header.requests_hash, Some(expected));
    assert_ne!(expected, EMPTY_REQUESTS_HASH);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_prague_requests_use_genesis_deposit_contract_and_consensus_order_rpc() {
    let deposit_contract = address!("0000000000000000000000000000000000006110");
    let mut genesis = Genesis::default();
    genesis.config.chain_id = 31_337;
    genesis.config.deposit_contract_address = Some(deposit_contract);
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_genesis(Some(genesis))
            .with_hardfork(Some(EthereumHardfork::Prague.into()))
            .with_precompile_factory(RequestOutputPrecompileFactory),
    )
    .await;
    let pubkey = Bytes::from(vec![0x11; 48]);
    let withdrawal_credentials = Bytes::from(vec![0x22; 32]);
    let amount = Bytes::from(vec![0x33; 8]);
    let signature = Bytes::from(vec![0x44; 96]);
    let index = Bytes::from(vec![0x55; 8]);
    let event = DepositEvent {
        pubkey: pubkey.clone(),
        withdrawal_credentials: withdrawal_credentials.clone(),
        amount: amount.clone(),
        signature: signature.clone(),
        index: index.clone(),
    };
    api.anvil_set_code(deposit_contract, deposit_event_runtime(event)).await.unwrap();
    api.anvil_set_auto_mine(false).await.unwrap();
    let from = handle.dev_wallets().next().unwrap().address();
    let request = TransactionRequest::default().from(from).to(deposit_contract).gas_limit(200_000);

    let simulated = api
        .simulate_v1(
            SimulatePayload {
                block_state_calls: vec![SimBlock {
                    calls: vec![request.clone()],
                    ..Default::default()
                }],
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap()
        .remove(0);

    let mut deposit_request = vec![DEPOSIT_REQUEST_TYPE];
    deposit_request.extend(pubkey);
    deposit_request.extend(withdrawal_credentials);
    deposit_request.extend(amount);
    deposit_request.extend(signature);
    deposit_request.extend(index);
    let expected = Requests::new(vec![
        deposit_request.into(),
        Bytes::from(vec![WITHDRAWAL_REQUEST_TYPE, 0xaa]),
        Bytes::from(vec![CONSOLIDATION_REQUEST_TYPE, 0xbb]),
    ])
    .requests_hash();
    assert_eq!(simulated.inner.header.requests_hash, Some(expected));

    let _pending =
        handle.http_provider().send_transaction(WithOtherFields::new(request)).await.unwrap();
    api.mine_one().await.unwrap();
    let mined = handle
        .http_provider()
        .get_block_by_number(BlockNumberOrTag::Number(1))
        .await
        .unwrap()
        .expect("mined block should exist");
    assert_eq!(mined.header.requests_hash, Some(expected));
}

const SIMULATION_POST_BLOCK_ERROR: &str = "simulation post-block sentinel";

#[derive(Clone, Copy, Debug)]
struct FailingWithdrawalPrecompileFactory;

impl PrecompileFactory for FailingWithdrawalPrecompileFactory {
    fn precompiles(&self) -> Vec<(Address, DynPrecompile)> {
        vec![(
            WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
            DynPrecompile::from(|_: PrecompileInput<'_>| {
                Err(PrecompileError::Fatal(SIMULATION_POST_BLOCK_ERROR.to_string()))
            }),
        )]
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_discards_candidate_after_post_block_failure_rpc() {
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_hardfork(Some(EthereumHardfork::Prague.into()))
            .with_precompile_factory(FailingWithdrawalPrecompileFactory),
    )
    .await;
    let state_root = api.state_root().await.unwrap();
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{"blockStateCalls": [{
            "stateOverrides": {
                "0xc000000000000000000000000000000000000000": {"balance": "0x1"}
            }
        }]}]),
    )
    .await;

    assert!(response["error"]["message"].as_str().unwrap().contains(SIMULATION_POST_BLOCK_ERROR));
    assert_eq!(api.state_root().await.unwrap(), state_root);
    assert_eq!(handle.http_provider().get_block_number().await.unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_empty_state_override_preserves_root_rpc() {
    let (_, handle) = spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let account = address!("0000000000000000000000000000000000000042").to_string();
    let without_override =
        rpc_request(&endpoint, "eth_simulateV1", json!([{"blockStateCalls": [{}]}])).await;
    let with_empty_override = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{"blockStateCalls": [{"stateOverrides": {(account.clone()): {}}}]}]),
    )
    .await;
    let with_empty_state_diff = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{"blockStateCalls": [{"stateOverrides": {(account): {"stateDiff": {}}}}]}]),
    )
    .await;

    assert!(without_override.get("error").is_none(), "{without_override}");
    assert!(with_empty_override.get("error").is_none(), "{with_empty_override}");
    assert!(with_empty_state_diff.get("error").is_none(), "{with_empty_state_diff}");
    assert_eq!(
        without_override["result"][0]["stateRoot"],
        with_empty_override["result"][0]["stateRoot"]
    );
    assert_eq!(
        without_override["result"][0]["stateRoot"],
        with_empty_state_diff["result"][0]["stateRoot"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_selfdestruct_state_root_matches_mined_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::London.into()))
        .with_base_fee(Some(0));
    let (api, handle) = spawn(config).await;
    let endpoint = handle.http_endpoint();
    let sender = handle.dev_accounts().next().unwrap();
    let contract = address!("0xc000000000000000000000000000000000000000");
    let beneficiary = address!("0xc100000000000000000000000000000000000000");
    let mut code = vec![0x73];
    code.extend_from_slice(beneficiary.as_slice());
    code.push(0xff);

    api.anvil_set_code(contract, code.into()).await.unwrap();
    api.anvil_set_balance(contract, U256::from(1)).await.unwrap();
    api.anvil_set_storage_at(contract, U256::ZERO, B256::from(U256::from(42))).await.unwrap();
    api.mine_one().await.unwrap();

    let selfdestruct = json!({
        "from": sender,
        "to": contract,
        "gas": "0x186a0",
        "gasPrice": "0x0"
    });
    let transfer = json!({
        "from": sender,
        "to": contract,
        "gas": "0x5208",
        "gasPrice": "0x0",
        "value": "0x1"
    });
    let simulated = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [
                {"calls": [selfdestruct.clone()]},
                {"calls": [transfer]}
            ],
            "validation": true
        }]),
    )
    .await;
    assert!(simulated.get("error").is_none(), "{simulated}");
    assert_eq!(simulated["result"][0]["calls"][0]["status"], "0x1");
    assert_eq!(simulated["result"][1]["calls"][0]["status"], "0x1");

    handle
        .http_provider()
        .send_transaction(WithOtherFields::new(TransactionRequest {
            from: Some(sender),
            to: Some(TxKind::Call(contract)),
            gas: Some(100_000),
            gas_price: Some(0),
            ..Default::default()
        }))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let first_mined =
        rpc_request(&endpoint, "eth_getBlockByNumber", json!(["latest", false])).await;
    handle
        .http_provider()
        .send_transaction(WithOtherFields::new(TransactionRequest {
            from: Some(sender),
            to: Some(TxKind::Call(contract)),
            value: Some(U256::from(1)),
            gas: Some(21_000),
            gas_price: Some(0),
            ..Default::default()
        }))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let second_mined =
        rpc_request(&endpoint, "eth_getBlockByNumber", json!(["latest", false])).await;
    let code = rpc_request(&endpoint, "eth_getCode", json!([contract, "latest"])).await;
    let storage =
        rpc_request(&endpoint, "eth_getStorageAt", json!([contract, "0x0", "latest"])).await;

    assert_eq!(code["result"], "0x");
    assert_eq!(storage["result"], B256::ZERO.to_string());
    assert_eq!(simulated["result"][0]["stateRoot"], first_mined["result"]["stateRoot"]);
    assert_eq!(simulated["result"][1]["stateRoot"], second_mined["result"]["stateRoot"]);
    assert_eq!(
        second_mined["result"]["stateRoot"],
        serde_json::to_value(api.state_root().await.unwrap()).unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_state_override_preserves_selfdestructed_storage_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::London.into()))
        .with_base_fee(Some(0));
    let (api, handle) = spawn(config).await;
    let endpoint = handle.http_endpoint();
    let sender = handle.dev_accounts().next().unwrap();
    let contract = address!("0xc000000000000000000000000000000000000000");
    let contract_key = contract.to_string();
    let beneficiary = address!("0xc100000000000000000000000000000000000000");
    let mut selfdestruct_code = vec![0x73];
    selfdestruct_code.extend_from_slice(beneficiary.as_slice());
    selfdestruct_code.push(0xff);

    api.anvil_set_code(contract, selfdestruct_code.into()).await.unwrap();
    api.anvil_set_balance(contract, U256::from(1)).await.unwrap();
    api.anvil_set_storage_at(contract, U256::ZERO, B256::from(U256::from(42))).await.unwrap();
    api.mine_one().await.unwrap();

    let selfdestruct = json!({
        "from": sender,
        "to": contract,
        "gas": "0x186a0",
        "gasPrice": "0x0"
    });
    let read_storage = json!({
        "from": sender,
        "to": contract,
        "gas": "0x186a0",
        "gasPrice": "0x0"
    });
    let code_only = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [
                {"calls": [selfdestruct.clone()]},
                {
                    "stateOverrides": {
                        (contract_key.clone()): {"code": "0x60005460005260206000f3"}
                    },
                    "calls": [read_storage.clone()]
                }
            ],
            "validation": true
        }]),
    )
    .await;
    let cleared_state = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [
                {"calls": [selfdestruct]},
                {
                    "stateOverrides": {
                        (contract_key): {
                            "code": "0x60005460005260206000f3",
                            "state": {}
                        }
                    },
                    "calls": [read_storage]
                }
            ],
            "validation": true
        }]),
    )
    .await;

    assert!(code_only.get("error").is_none(), "{code_only}");
    assert!(cleared_state.get("error").is_none(), "{cleared_state}");
    assert_eq!(code_only["result"][1]["calls"][0]["returnData"], B256::ZERO.to_string());
    assert_eq!(cleared_state["result"][1]["calls"][0]["returnData"], B256::ZERO.to_string());
    assert_eq!(code_only["result"][1]["stateRoot"], cleared_state["result"][1]["stateRoot"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_historical_tombstone_matches_latest_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Frontier.into()))
        .with_base_fee(Some(0));
    let (api, handle) = spawn(config).await;
    let endpoint = handle.http_endpoint();
    let sender = handle.dev_accounts().next().unwrap();
    let contract = address!("0xc000000000000000000000000000000000000000");
    let beneficiary = address!("0xc100000000000000000000000000000000000000");
    let mut code = vec![0x73];
    code.extend_from_slice(beneficiary.as_slice());
    code.push(0xff);

    api.anvil_set_code(contract, code.into()).await.unwrap();
    api.anvil_set_balance(contract, U256::from(1)).await.unwrap();
    api.mine_one().await.unwrap();
    handle
        .http_provider()
        .send_transaction(WithOtherFields::new(TransactionRequest {
            from: Some(sender),
            to: Some(TxKind::Call(contract)),
            gas: Some(100_000),
            gas_price: Some(0),
            ..Default::default()
        }))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let historical_base =
        rpc_request(&endpoint, "eth_getBlockByNumber", json!(["latest", false])).await;
    api.mine_one().await.unwrap();
    let latest_base =
        rpc_request(&endpoint, "eth_getBlockByNumber", json!(["latest", false])).await;
    assert_eq!(historical_base["result"]["stateRoot"], latest_base["result"]["stateRoot"]);

    let payload = json!([{
        "blockStateCalls": [{
            "calls": [{
                "from": sender,
                "to": contract,
                "gas": "0x5208",
                "gasPrice": "0x0"
            }]
        }],
        "validation": true
    }]);
    let historical = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([payload[0].clone(), historical_base["result"]["number"].clone()]),
    )
    .await;
    let latest =
        rpc_request(&endpoint, "eth_simulateV1", json!([payload[0].clone(), "latest"])).await;

    assert!(historical.get("error").is_none(), "{historical}");
    assert!(latest.get("error").is_none(), "{latest}");
    assert_eq!(historical["result"][0]["stateRoot"], latest["result"][0]["stateRoot"]);

    handle
        .http_provider()
        .send_transaction(WithOtherFields::new(TransactionRequest {
            from: Some(sender),
            to: Some(TxKind::Call(contract)),
            gas: Some(21_000),
            gas_price: Some(0),
            ..Default::default()
        }))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let mined = rpc_request(&endpoint, "eth_getBlockByNumber", json!(["latest", false])).await;
    assert_eq!(latest["result"][0]["stateRoot"], mined["result"]["stateRoot"]);
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
async fn test_simulate_executes_on_pending_state_rpc() {
    let (_, handle) = spawn(NodeConfig::test().with_no_mining(true)).await;
    let sender = handle.dev_accounts().next().unwrap();
    let receiver = address!("c100000000000000000000000000000000000000");
    let _pending = handle
        .http_provider()
        .send_transaction(WithOtherFields::new(TransactionRequest {
            from: Some(sender),
            to: Some(TxKind::Call(receiver)),
            value: Some(U256::from(1)),
            ..Default::default()
        }))
        .await
        .unwrap();

    let mut code = vec![0x73];
    code.extend_from_slice(receiver.as_slice());
    code.extend_from_slice(&[0x31, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);
    let reader = address!("c200000000000000000000000000000000000000");
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {(reader.to_string()): {"code": Bytes::from(code)}},
                "calls": [{"to": reader}]
            }]
        }, "pending"]),
    )
    .await;

    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        response["result"][0]["calls"][0]["returnData"],
        B256::from(U256::from(1)).to_string()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_finalizes_call_and_transaction_metadata_rpc() {
    let (_, handle) = spawn(NodeConfig::test()).await;
    let sender = handle.dev_accounts().next().unwrap();
    let gas_price = "0x3b9aca00";
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "calls": [
                    {
                        "from": sender,
                        "to": "0xc100000000000000000000000000000000000000",
                        "gasPrice": gas_price
                    },
                    {
                        "from": sender,
                        "to": "0xc200000000000000000000000000000000000000",
                        "gasPrice": gas_price
                    }
                ]
            }],
            "returnFullTransactions": true
        }, "latest"]),
    )
    .await;

    assert!(response.get("error").is_none(), "{response}");
    let block = &response["result"][0];
    let calls = block["calls"].as_array().unwrap();
    let transactions = block["transactions"].as_array().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(transactions.len(), 2);
    for (index, (call, transaction)) in calls.iter().zip(transactions).enumerate() {
        assert!(quantity(&call["maxUsedGas"]) >= quantity(&call["gasUsed"]));
        assert_eq!(transaction["blockHash"], block["hash"]);
        assert_eq!(transaction["blockNumber"], block["number"]);
        assert_eq!(quantity(&transaction["transactionIndex"]), index as u64);
        assert_eq!(transaction["blockTimestamp"], block["timestamp"]);
        assert_eq!(transaction["gasPrice"], gas_price);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_prague_max_used_gas_includes_calldata_floor_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Prague.into()))
        .with_base_fee(Some(0));
    let (_, handle) = spawn(config).await;
    let input = format!("0x{}", "ff".repeat(1_000));
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{"blockStateCalls": [{"calls": [{
            "to": "0xc100000000000000000000000000000000000000",
            "input": input
        }]}]}, "latest"]),
    )
    .await;
    assert!(response.get("error").is_none(), "{response}");
    let call = &response["result"][0]["calls"][0];

    assert_eq!(call["gasUsed"], "0xee48");
    assert_eq!(call["maxUsedGas"], "0xee48");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_max_used_gas_before_refund_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_base_fee(Some(0));
    let (_, handle) = spawn(config).await;
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
async fn test_simulate_reports_non_gas_halts_rpc() {
    let (_, handle) = spawn(NodeConfig::test()).await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc100000000000000000000000000000000000000": {"code": "0xfe"}
                },
                "calls": [{"to": "0xc100000000000000000000000000000000000000"}]
            }]
        }, "latest"]),
    )
    .await;

    assert!(response.get("error").is_none(), "{response}");
    let error = &response["result"][0]["calls"][0]["error"];
    assert_eq!(error["code"], -32015);
    assert!(error["message"].as_str().unwrap().starts_with("vm execution error:"));
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
async fn test_simulate_trace_transfer_response_logs_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_base_fee(Some(0));
    let (_api, handle) = spawn(config).await;
    let endpoint = handle.http_endpoint();
    let payload = json!({
        "blockStateCalls": [{
            "stateOverrides": {
                "0xc000000000000000000000000000000000000000": {"balance": "0x2"},
                "0xc100000000000000000000000000000000000000": {
                    "code": "0x5f5f5f5f600173c2000000000000000000000000000000000000005af15000"
                }
            },
            "calls": [{
                "from": "0xc000000000000000000000000000000000000000",
                "to": "0xc100000000000000000000000000000000000000",
                "value": "0x2"
            }]
        }],
        "traceTransfers": true
    });
    let response =
        rpc_request(&endpoint, "eth_simulateV1", json!([payload.clone(), "latest"])).await;
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"][0]["logsBloom"], format!("0x{}", "00".repeat(256)));
    let logs = response["result"][0]["calls"][0]["logs"].as_array().unwrap();
    assert_eq!(logs.len(), 2);
    assert_eq!(
        logs.iter()
            .map(|log| {
                json!({
                    "address": log["address"],
                    "from": log["topics"][1],
                    "to": log["topics"][2],
                    "value": log["data"],
                    "logIndex": log["logIndex"],
                })
            })
            .collect::<Vec<_>>(),
        vec![
            json!({
                "address": "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "from": "0x000000000000000000000000c000000000000000000000000000000000000000",
                "to": "0x000000000000000000000000c100000000000000000000000000000000000000",
                "value": "0x0000000000000000000000000000000000000000000000000000000000000002",
                "logIndex": "0x0",
            }),
            json!({
                "address": "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "from": "0x000000000000000000000000c100000000000000000000000000000000000000",
                "to": "0x000000000000000000000000c200000000000000000000000000000000000000",
                "value": "0x0000000000000000000000000000000000000000000000000000000000000001",
                "logIndex": "0x1",
            }),
        ]
    );

    let mut payload = payload;
    payload["traceTransfers"] = json!(false);
    let response = rpc_request(&endpoint, "eth_simulateV1", json!([payload, "latest"])).await;
    assert_eq!(response["result"][0]["calls"][0]["logs"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_preserves_amsterdam_transfer_logs_rpc() {
    let config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Amsterdam.into()))
        .with_base_fee(Some(0));
    let (_api, handle) = spawn(config).await;
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": {"balance": "0x1"}
                },
                "calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc100000000000000000000000000000000000000",
                    "value": "0x1"
                }]
            }],
            "traceTransfers": false
        }, "latest"]),
    )
    .await;

    assert!(response.get("error").is_none(), "{response}");
    assert_ne!(response["result"][0]["logsBloom"], format!("0x{}", "00".repeat(256)));
    let logs = response["result"][0]["calls"][0]["logs"].as_array().unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0]["address"], "0xfffffffffffffffffffffffffffffffffffffffe");
    assert_eq!(logs[0]["logIndex"], "0x0");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_trace_transfer_excludes_callcode_rpc() {
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
                    "0xc100000000000000000000000000000000000000": {
                        "balance": "0x1",
                        "code": "0x5f5f5f5f600173c2000000000000000000000000000000000000005af200"
                    }
                },
                "calls": [{"to": "0xc100000000000000000000000000000000000000"}]
            }],
            "traceTransfers": true
        }, "latest"]),
    )
    .await;

    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"][0]["calls"][0]["status"], "0x1");
    assert_eq!(response["result"][0]["calls"][0]["logs"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_trace_transfer_create_nonce_overflow_rpc() {
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
                    "0xc100000000000000000000000000000000000000": {
                        "balance": "0x1",
                        "nonce": "0xffffffffffffffff",
                        "code": "0x600060006001f0505f5fa000"
                    }
                },
                "calls": [{"to": "0xc100000000000000000000000000000000000000"}]
            }],
            "traceTransfers": true
        }, "latest"]),
    )
    .await;

    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"][0]["calls"][0]["status"], "0x1");
    let logs = response["result"][0]["calls"][0]["logs"].as_array().unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0]["address"], "0xc100000000000000000000000000000000000000");
    assert_eq!(logs[0]["logIndex"], "0x1");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_trace_create_and_selfdestruct_transfers_rpc() {
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
                    "0xc100000000000000000000000000000000000000": {
                        "balance": "0x1",
                        "code": "0x600060006001f000"
                    },
                    "0xc200000000000000000000000000000000000000": {
                        "balance": "0x1",
                        "code": "0x6000600060006001f500"
                    },
                    "0xc300000000000000000000000000000000000000": {
                        "balance": "0x2",
                        "code": "0x73c400000000000000000000000000000000000000ff"
                    }
                },
                "calls": [
                    {"to": "0xc100000000000000000000000000000000000000"},
                    {"to": "0xc200000000000000000000000000000000000000"},
                    {"to": "0xc300000000000000000000000000000000000000"}
                ]
            }],
            "traceTransfers": true
        }, "latest"]),
    )
    .await;
    assert!(response.get("error").is_none(), "{response}");
    let calls = response["result"][0]["calls"].as_array().unwrap();
    assert!(calls.iter().all(|call| call["status"] == "0x1"));
    for (index, sender) in [
        "0x000000000000000000000000c100000000000000000000000000000000000000",
        "0x000000000000000000000000c200000000000000000000000000000000000000",
    ]
    .into_iter()
    .enumerate()
    {
        let logs = calls[index]["logs"].as_array().unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0]["topics"][1], sender);
        assert_ne!(
            logs[0]["topics"][2],
            "0x0000000000000000000000000000000000000000000000000000000000000000"
        );
        assert_eq!(
            logs[0]["data"],
            "0x0000000000000000000000000000000000000000000000000000000000000001"
        );
    }
    let selfdestruct = &calls[2]["logs"][0];
    assert_eq!(
        json!({
            "from": selfdestruct["topics"][1],
            "to": selfdestruct["topics"][2],
            "value": selfdestruct["data"],
        }),
        json!({
            "from": "0x000000000000000000000000c300000000000000000000000000000000000000",
            "to": "0x000000000000000000000000c400000000000000000000000000000000000000",
            "value": "0x0000000000000000000000000000000000000000000000000000000000000002",
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_trace_transfer_reverts_and_log_order_rpc() {
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
                        "balance": "0x2",
                        "code": "0x730000000000000000000000000000000000000000ff"
                    },
                    "0xc400000000000000000000000000000000000000": {
                        "code": "0x5f5fa05f5f5f5f5f73c2000000000000000000000000000000000000005af1505f5fa000"
                    }
                },
                "calls": [{"to": "0xc400000000000000000000000000000000000000"}]
            }],
            "traceTransfers": true
        }, "latest"]),
    )
    .await;
    assert!(response.get("error").is_none(), "{response}");
    let logs = response["result"][0]["calls"][0]["logs"].as_array().unwrap();
    assert_eq!(
        logs.iter()
            .map(|log| json!({"address": log["address"], "logIndex": log["logIndex"]}))
            .collect::<Vec<_>>(),
        vec![
            json!({
                "address": "0xc400000000000000000000000000000000000000",
                "logIndex": "0x0"
            }),
            json!({
                "address": "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "logIndex": "0x1"
            }),
            json!({
                "address": "0xc400000000000000000000000000000000000000",
                "logIndex": "0x2"
            }),
        ]
    );

    let reverted_log_payload = json!({
        "blockStateCalls": [{
            "stateOverrides": {
                "0xc300000000000000000000000000000000000000": {
                    "code": "0x5f5fa05f5ffd"
                },
                "0xc400000000000000000000000000000000000000": {
                    "code": "0x5f5fa05f5f5f5f5f73c3000000000000000000000000000000000000005af1505f5fa000"
                }
            },
            "calls": [{"to": "0xc400000000000000000000000000000000000000"}]
        }],
        "traceTransfers": true
    });
    let response =
        rpc_request(&endpoint, "eth_simulateV1", json!([reverted_log_payload.clone(), "latest"]))
            .await;
    assert!(response.get("error").is_none(), "{response}");
    let logs = response["result"][0]["calls"][0]["logs"].as_array().unwrap();
    assert_eq!(
        logs.iter()
            .map(|log| json!({"address": log["address"], "logIndex": log["logIndex"]}))
            .collect::<Vec<_>>(),
        vec![
            json!({
                "address": "0xc400000000000000000000000000000000000000",
                "logIndex": "0x0"
            }),
            json!({
                "address": "0xc400000000000000000000000000000000000000",
                "logIndex": "0x2"
            }),
        ]
    );

    let mut reverted_log_payload = reverted_log_payload;
    reverted_log_payload["traceTransfers"] = json!(false);
    let response =
        rpc_request(&endpoint, "eth_simulateV1", json!([reverted_log_payload, "latest"])).await;
    let logs = response["result"][0]["calls"][0]["logs"].as_array().unwrap();
    assert_eq!(
        logs.iter().map(|log| log["logIndex"].clone()).collect::<Vec<_>>(),
        vec![json!("0x0"), json!("0x2")]
    );

    let response = rpc_request(
        &endpoint,
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": {"balance": "0x2"},
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
    assert_eq!(calls[1]["logs"][0]["logIndex"], "0x1");
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
async fn test_simulate_transfer_traces_do_not_change_header_bloom() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let sender = handle.dev_accounts().next().unwrap();
    let response = rpc_request(
        &handle.http_endpoint(),
        "eth_simulateV1",
        json!([{
            "blockStateCalls": [{
                "calls": [{
                    "from": sender,
                    "to": address!("0xc100000000000000000000000000000000000000"),
                    "value": "0x1"
                }]
            }],
            "traceTransfers": true
        }, "latest"]),
    )
    .await;

    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        response["result"][0]["calls"][0]["logs"][0]["address"],
        "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
    );
    assert_eq!(
        serde_json::from_value::<Bloom>(response["result"][0]["logsBloom"].clone()).unwrap(),
        Bloom::ZERO
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulate_preserves_canonical_transfer_emitter_log() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let mut payload = json!({
        "blockStateCalls": [{
            "stateOverrides": {
                "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee": {
                    "code": concat!(
                        "0x7f",
                        "ddf252ad1be2c89b69c2b068fc378daa",
                        "952ba7f163c4a11628f55a4df523b3ef",
                        "5f5fa100"
                    )
                }
            },
            "calls": [{
                "to": "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
            }]
        }],
        "traceTransfers": true
    });
    let traced = rpc_request(&endpoint, "eth_simulateV1", json!([payload.clone(), "latest"])).await;
    assert!(traced.get("error").is_none(), "{traced}");
    assert_eq!(traced["result"][0]["calls"][0]["logs"].as_array().unwrap().len(), 1);
    assert_ne!(
        serde_json::from_value::<Bloom>(traced["result"][0]["logsBloom"].clone()).unwrap(),
        Bloom::ZERO
    );

    payload["traceTransfers"] = json!(false);
    let untraced = rpc_request(&endpoint, "eth_simulateV1", json!([payload, "latest"])).await;
    assert_eq!(traced["result"][0]["logsBloom"], untraced["result"][0]["logsBloom"]);
    assert_eq!(traced["result"][0]["receiptsRoot"], untraced["result"][0]["receiptsRoot"]);
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

async fn spawn_hash_aware_rpc_proxy(
    hash_endpoint: String,
    canonical_endpoint: String,
    hash_selector: Value,
) -> String {
    let router = Router::new().route(
        "/",
        post(move |Json(request): Json<Value>| {
            let hash_endpoint = hash_endpoint.clone();
            let canonical_endpoint = canonical_endpoint.clone();
            let hash_selector = hash_selector.clone();
            async move {
                let method = request["method"].as_str().unwrap();
                let endpoint = if method == "eth_getBlockByHash"
                    && request["params"].get(0) == Some(&hash_selector)
                    || method == "eth_simulateV1"
                        && request["params"].get(1) == Some(&hash_selector)
                {
                    hash_endpoint
                } else {
                    canonical_endpoint
                };
                let response = reqwest::Client::new()
                    .post(endpoint)
                    .json(&request)
                    .send()
                    .await
                    .unwrap()
                    .json::<Value>()
                    .await
                    .unwrap();
                Json(response)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{address}")
}

fn quantity(value: &Value) -> u64 {
    u64::from_str_radix(value.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
}
