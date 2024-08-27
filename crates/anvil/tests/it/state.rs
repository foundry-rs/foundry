//! general eth api tests

use alloy_primitives::Uint;
use anvil::{spawn, NodeConfig};

#[tokio::test(flavor = "multi_thread")]
async fn can_load_state() {
    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");

    let (api, _handle) = spawn(NodeConfig::test()).await;

    api.mine_one().await;

    let num = api.block_number().unwrap();

    let state = api.serialized_state().await.unwrap();
    foundry_common::fs::write_json_file(&state_file, &state).unwrap();

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let num2 = api.block_number().unwrap();
    assert_eq!(num, num2);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_load_existing_state_legacy() {
    let state_file = "test-data/state-dump-legacy.json";

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, Uint::from(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_load_existing_state() {
    let state_file = "test-data/state-dump.json";

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, Uint::from(2));
}
