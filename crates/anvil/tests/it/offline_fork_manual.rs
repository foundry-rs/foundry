use alloy_primitives::{Address, U256};
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, spawn};
use foundry_test_utils::rpc::next_http_archive_rpc_url;

/// Test offline mode with a real fork
#[tokio::test(flavor = "multi_thread")]
async fn test_offline_fork_from_saved_state() {
    // Step 1: Create a fork and save state
    let fork_url = next_http_archive_rpc_url();
    let fork_config = NodeConfig::default()
        .with_eth_rpc_url(Some(fork_url.clone()))
        .with_fork_block_number(Some(20_000_000u64))
        .with_port(0); // Use random port

    let (api, handle) = spawn(fork_config).await;

    // Get some initial data
    let test_address: Address = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266".parse().unwrap();
    let initial_balance = api.balance(test_address, None).await.unwrap();
    let block_number = api.block_number().unwrap();

    // Save the state
    let state = api.serialized_state(false).await.unwrap();

    // Shutdown the first node
    drop(handle);

    // Step 2: Start a new node in offline mode with the saved state
    let offline_config = NodeConfig::default()
        .with_eth_rpc_url(Some("https://this-should-not-be-called.com".to_string()))
        .with_init_state(Some(state))
        .with_offline(true)
        .with_port(0); // Use random port

    let (api, _handle) = spawn(offline_config).await;

    // Verify we can access the data without network calls
    let balance = api.balance(test_address, None).await.unwrap();
    assert_eq!(balance, initial_balance);

    let current_block = api.block_number().unwrap();
    assert_eq!(current_block, block_number);

    // Test that we can send transactions in offline mode
    let tx = TransactionRequest {
        from: Some(test_address),
        to: Some(test_address.into()),
        value: Some(U256::from(1000)),
        ..Default::default()
    };

    let tx_hash = api.send_transaction(WithOtherFields::new(tx)).await.unwrap();
    assert!(!tx_hash.is_zero());

    // Mine a block
    api.mine_one().await;

    // Verify block was mined
    let new_block_number = api.block_number().unwrap();
    assert_eq!(new_block_number, block_number + U256::from(1));
}
