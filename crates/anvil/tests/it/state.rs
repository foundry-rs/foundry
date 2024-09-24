//! general eth api tests

use crate::abi::Greeter;
use alloy_primitives::{Bytes, Uint};
use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
use anvil::{spawn, NodeConfig};

#[tokio::test(flavor = "multi_thread")]
async fn can_load_state() {
    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");

    let (api, _handle) = spawn(NodeConfig::test()).await;

    api.mine_one().await;

    let num = api.block_number().unwrap();

    let state = api.serialized_state(false).await.unwrap();
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
async fn can_load_existing_state_legacy_stress() {
    let state_file = "test-data/state-dump-legacy-stress.json";

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, Uint::from(5));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_load_existing_state() {
    let state_file = "test-data/state-dump.json";

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, Uint::from(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_make_sure_historical_state_is_not_cleared_on_dump() {
    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");

    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = handle.http_provider();

    let greeter = Greeter::deploy(&provider, "Hello".to_string()).await.unwrap();

    let address = greeter.address();

    let _tx = greeter
        .setGreeting("World!".to_string())
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    api.mine_one().await;

    let ser_state = api.serialized_state(true).await.unwrap();
    foundry_common::fs::write_json_file(&state_file, &ser_state).unwrap();

    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, Uint::from(3));

    // Makes sure historical states of the new instance are not cleared.
    let code = provider.get_code_at(*address).block_id(BlockId::number(2)).await.unwrap();

    assert_ne!(code, Bytes::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_preserve_historical_states_between_dump_and_load() {
    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");

    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = handle.http_provider();

    let greeter = Greeter::deploy(&provider, "Hello".to_string()).await.unwrap();

    let address = greeter.address();

    let deploy_blk_num = provider.get_block_number().await.unwrap();

    let tx = greeter
        .setGreeting("World!".to_string())
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let change_greeting_blk_num = tx.block_number.unwrap();

    api.mine_one().await;

    let ser_state = api.serialized_state(true).await.unwrap();
    foundry_common::fs::write_json_file(&state_file, &ser_state).unwrap();

    let (api, handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, Uint::from(3));

    let provider = handle.http_provider();

    let greeter = Greeter::new(*address, provider);

    let greeting_at_init =
        greeter.greet().block(BlockId::number(deploy_blk_num)).call().await.unwrap()._0;

    assert_eq!(greeting_at_init, "Hello");

    let greeting_after_change =
        greeter.greet().block(BlockId::number(change_greeting_blk_num)).call().await.unwrap()._0;

    assert_eq!(greeting_after_change, "World!");
}
