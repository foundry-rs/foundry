//! Access list prefetching of fork state under `--fork-prefetch`.

use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, B256, U256, address};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use anvil::{NodeConfig, spawn};
use foundry_test_utils::rpc::spawn_rpc_proxy_recording_method;
use serde_json::Value;

/// A contract on the origin whose code sums its first [`SLOT_COUNT`] storage slots.
const SLOTS: Address = address!("0x000000000000000000000000000000000000510c");
const SLOT_COUNT: u8 = 40;

/// Runtime code that returns the sum of the first [`SLOT_COUNT`] storage slots.
fn sum_slots_code() -> Vec<u8> {
    let mut code = vec![0x60, 0x00];
    for slot in 0..SLOT_COUNT {
        // PUSH1 slot, SLOAD, ADD.
        code.extend([0x60, slot, 0x54, 0x01]);
    }
    // PUSH1 0, MSTORE, PUSH1 32, PUSH1 0, RETURN.
    code.extend([0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);
    code
}

fn is_address(value: &Value, address: Address) -> bool {
    value.as_str().is_some_and(|value| value.eq_ignore_ascii_case(&address.to_string()))
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_prefetch_simulates_calls() {
    let (origin_api, origin) = spawn(NodeConfig::test()).await;
    origin_api.anvil_set_code(SLOTS, sum_slots_code().into()).await.unwrap();
    for slot in 0..SLOT_COUNT {
        origin_api
            .anvil_set_storage_at(SLOTS, U256::from(slot), B256::with_last_byte(slot + 1))
            .await
            .unwrap();
    }
    origin_api.mine_one().await.unwrap();
    let (rpc, simulations) =
        spawn_rpc_proxy_recording_method(origin.http_endpoint(), "eth_createAccessList").await;

    let (_api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(rpc))
            .with_no_storage_caching(true)
            .with_fork_prefetch(true),
    )
    .await;
    let output = handle
        .http_provider()
        .call(TransactionRequest::default().with_to(SLOTS).into())
        .await
        .unwrap();
    assert_eq!(U256::from_be_slice(&output), U256::from(820));

    // The call is simulated on the origin.
    let simulations = simulations.lock().unwrap();
    assert!(
        simulations.iter().any(|params| is_address(&params[0]["to"], SLOTS)),
        "no simulation of the call in {simulations:?}"
    );
}
