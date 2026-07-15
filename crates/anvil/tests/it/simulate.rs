//! general eth api tests

use alloy_primitives::{Bytes, TxKind, U256, address};
use alloy_provider::Provider;
use alloy_rpc_types::{
    BlockOverrides,
    request::TransactionRequest,
    simulate::{SimBlock, SimulatePayload},
    state::{AccountOverride, StateOverridesBuilder},
};
use anvil::{NodeConfig, spawn};
use foundry_test_utils::rpc;

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
