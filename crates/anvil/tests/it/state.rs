//! general eth api tests

use crate::abi::Greeter;
use alloy_network::{ReceiptResponse, TransactionBuilder};
use alloy_primitives::{address, utils::Unit, Bytes, Uint, U256, U64};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, TransactionRequest};
use alloy_serde::WithOtherFields;
use anvil::{spawn, NodeConfig};
use foundry_test_utils::rpc::next_http_rpc_endpoint;

#[tokio::test(flavor = "multi_thread")]
async fn can_load_state() {
    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");

    let (api, _handle) = spawn(NodeConfig::test()).await;

    api.mine_one().await;
    api.mine_one().await;

    let num = api.block_number().unwrap();

    let state = api.serialized_state(false).await.unwrap();
    foundry_common::fs::write_json_file(&state_file, &state).unwrap();

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let num2 = api.block_number().unwrap();

    // Ref: https://github.com/foundry-rs/foundry/issues/9017
    // Check responses of eth_blockNumber and eth_getBlockByNumber don't deviate after loading state
    let num_from_tag = api
        .block_by_number(alloy_eips::BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap()
        .header
        .number;
    assert_eq!(num, num2);

    assert_eq!(num, U256::from(num_from_tag));
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

// <https://github.com/foundry-rs/foundry/issues/9053>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_load_state() {
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(next_http_rpc_endpoint()))
            .with_fork_block_number(Some(21070682u64)),
    )
    .await;

    let bob = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let alice = address!("9276449EaC5b4f7Bc17cFC6700f7BeeB86F9bCd0");

    let provider = handle.http_provider();

    let init_nonce_bob = provider.get_transaction_count(bob).await.unwrap();

    let init_balance_alice = provider.get_balance(alice).await.unwrap();

    let value = Unit::ETHER.wei().saturating_mul(U256::from(1)); // 1 ether
    let tx = TransactionRequest::default().with_to(alice).with_value(value).with_from(bob);
    let tx = WithOtherFields::new(tx);

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());

    let serialized_state = api.serialized_state(false).await.unwrap();

    let state_dump_block = api.block_number().unwrap();

    let (api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(next_http_rpc_endpoint()))
            .with_fork_block_number(Some(21070686u64)) // Forked chain has moved forward
            .with_init_state(Some(serialized_state)),
    )
    .await;

    // Ensure the initial block number is the fork_block_number and not the state_dump_block
    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, U256::from(21070686u64));
    assert_ne!(block_number, state_dump_block);

    let provider = handle.http_provider();

    let restart_nonce_bob = provider.get_transaction_count(bob).await.unwrap();

    let restart_balance_alice = provider.get_balance(alice).await.unwrap();

    assert_eq!(init_nonce_bob + 1, restart_nonce_bob);

    assert_eq!(init_balance_alice + value, restart_balance_alice);

    // Send another tx to check if the state is preserved

    let tx = TransactionRequest::default().with_to(alice).with_value(value).with_from(bob);
    let tx = WithOtherFields::new(tx);

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());

    let nonce_bob = provider.get_transaction_count(bob).await.unwrap();

    let balance_alice = provider.get_balance(alice).await.unwrap();

    let tx = TransactionRequest::default()
        .with_to(alice)
        .with_value(value)
        .with_from(bob)
        .with_nonce(nonce_bob);
    let tx = WithOtherFields::new(tx);

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());

    let latest_nonce_bob = provider.get_transaction_count(bob).await.unwrap();

    let latest_balance_alice = provider.get_balance(alice).await.unwrap();

    assert_eq!(nonce_bob + 1, latest_nonce_bob);

    assert_eq!(balance_alice + value, latest_balance_alice);
}

// <https://github.com/foundry-rs/foundry/issues/9539>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_load_state_with_greater_state_block() {
    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(next_http_rpc_endpoint()))
            .with_fork_block_number(Some(21070682u64)),
    )
    .await;

    api.mine_one().await;

    let block_number = api.block_number().unwrap();

    let serialized_state = api.serialized_state(false).await.unwrap();

    assert_eq!(serialized_state.best_block_number, Some(block_number.to::<U64>()));

    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(next_http_rpc_endpoint()))
            .with_fork_block_number(Some(21070682u64)) // Forked chain has moved forward
            .with_init_state(Some(serialized_state)),
    )
    .await;

    let new_block_number = api.block_number().unwrap();

    assert_eq!(new_block_number, block_number);
}
