//! general eth api tests

use alloy_primitives::{TxKind, U256, address};
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
