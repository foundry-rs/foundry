use alloy_primitives::{Address, U256};
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, spawn};

/// Test that offline mode works with a pre-existing state file
/// and basic RPC calls return cached state without making network requests.
#[tokio::test(flavor = "multi_thread")]
async fn test_offline_fork_basic_rpcs() {
    // Use the pre-existing test state file
    let state_path = concat!(env!("CARGO_MANIFEST_DIR"), "/test-data/offline_fork_state.json");

    // Load the state from file
    let state_file = std::fs::read_to_string(state_path).expect("Failed to read state file");
    let state: anvil::eth::backend::db::SerializableState =
        serde_json::from_str(&state_file).expect("Failed to deserialize state");

    // Set up node config with offline mode and an invalid RPC URL.
    // If offline mode works correctly, it should not try to connect to this URL.
    let config = NodeConfig::default()
        .with_eth_rpc_url(Some("https://invalid-url-that-should-not-be-called.com".to_string()))
        .with_fork_block_number(Some(20000000u64))
        .with_init_state(Some(state))
        .with_offline(true)
        .with_port(0);

    // This should succeed without making RPC calls
    let (api, _handle) = spawn(config).await;

    // eth_chainId
    let chain_id = api.chain_id();
    assert_eq!(chain_id, 31337); // Default anvil chain id

    // eth_blockNumber
    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, U256::from(20000000));

    // eth_getBalance - test with an account from the state
    let test_address: Address = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266".parse().unwrap();
    let balance = api.balance(test_address, None).await.unwrap();
    assert_eq!(balance, U256::from_str_radix("21e19e0c9bab2400000", 16).unwrap());

    // eth_getTransactionCount
    let nonce = api.transaction_count(test_address, None).await.unwrap();
    assert_eq!(nonce, U256::ZERO);

    // eth_gasPrice
    let gas_price = api.gas_price();
    assert!(gas_price > 0);

    // eth_getCode - should return empty for EOA
    let code = api.get_code(test_address, None).await.unwrap();
    assert_eq!(code.len(), 0);
}

/// Test that offline mode works with state-modifying operations.
#[tokio::test(flavor = "multi_thread")]
async fn test_offline_fork_state_modifications() {
    let state_path = concat!(env!("CARGO_MANIFEST_DIR"), "/test-data/offline_fork_state.json");

    let state_file = std::fs::read_to_string(state_path).expect("Failed to read state file");
    let state: anvil::eth::backend::db::SerializableState =
        serde_json::from_str(&state_file).expect("Failed to deserialize state");

    let config = NodeConfig::default()
        .with_eth_rpc_url(Some("https://does-not-exist.com".to_string()))
        .with_init_state(Some(state))
        .with_offline(true)
        .with_port(0);

    let (api, _handle) = spawn(config).await;

    let new_address: Address = "0x0000000000000000000000000000000000000001".parse().unwrap();

    // anvil_setBalance
    let new_balance = U256::from(42);
    api.anvil_set_balance(new_address, new_balance).await.unwrap();

    // Verify the balance was set
    let balance = api.balance(new_address, None).await.unwrap();
    assert_eq!(balance, new_balance);

    // anvil_setCode
    let code = vec![0x60, 0x00, 0x60, 0x00, 0xfd]; // PUSH1 0x00 PUSH1 0x00 REVERT
    api.anvil_set_code(new_address, code.clone().into()).await.unwrap();

    // Verify the code was set
    let stored_code = api.get_code(new_address, None).await.unwrap();
    assert_eq!(stored_code.as_ref(), &code);

    // anvil_mine
    api.anvil_mine(Some(U256::from(3)), None).await.unwrap();

    // Verify blocks were mined
    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, U256::from(20000003));
}

/// Test that offline mode doesn't make RPC calls for missing data.
/// Uses an invalid URL that would fail if contacted.
#[tokio::test(flavor = "multi_thread")]
async fn test_offline_fork_missing_data_no_rpc() {
    let state_path = concat!(env!("CARGO_MANIFEST_DIR"), "/test-data/offline_fork_state.json");

    let state_file = std::fs::read_to_string(state_path).expect("Failed to read state file");
    let state: anvil::eth::backend::db::SerializableState =
        serde_json::from_str(&state_file).expect("Failed to deserialize state");

    let config = NodeConfig::default()
        .with_eth_rpc_url(Some(
            "https://this-url-does-not-exist-and-should-never-be-called.invalid".to_string(),
        ))
        .with_init_state(Some(state))
        .with_offline(true)
        .with_port(0);

    let (api, _handle) = spawn(config).await;

    // Try to access an account that's NOT in the loaded state.
    // In offline mode this should return default values quickly without RPC calls.
    let missing_address: Address = "0x0000000000000000000000000000000000000042".parse().unwrap();

    let start = std::time::Instant::now();
    let balance = api.balance(missing_address, None).await.unwrap();
    let elapsed = start.elapsed();

    // Should return default balance (0) for unknown accounts
    assert_eq!(balance, U256::ZERO);

    // Should complete quickly (< 5s) if no RPC call is made
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "Operation took {:?}, which suggests an RPC call may have been attempted",
        elapsed
    );

    // Test storage access for missing data
    let start = std::time::Instant::now();
    let storage = api.storage_at(missing_address, U256::ZERO.into(), None).await.unwrap();
    let elapsed = start.elapsed();

    assert_eq!(storage, alloy_primitives::B256::ZERO);
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "Storage operation took {:?}, which suggests an RPC call may have been attempted",
        elapsed
    );

    // Test code access for missing data
    let start = std::time::Instant::now();
    let code = api.get_code(missing_address, None).await.unwrap();
    let elapsed = start.elapsed();

    assert_eq!(code.len(), 0);
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "Code operation took {:?}, which suggests an RPC call may have been attempted",
        elapsed
    );
}

/// Test that a transaction can be sent and mined in offline mode.
#[tokio::test(flavor = "multi_thread")]
async fn test_offline_fork_send_transaction() {
    let state_path = concat!(env!("CARGO_MANIFEST_DIR"), "/test-data/offline_fork_state.json");

    let state_file = std::fs::read_to_string(state_path).expect("Failed to read state file");
    let state: anvil::eth::backend::db::SerializableState =
        serde_json::from_str(&state_file).expect("Failed to deserialize state");

    let config = NodeConfig::default()
        .with_eth_rpc_url(Some("https://invalid-url.invalid".to_string()))
        .with_fork_block_number(Some(20000000u64))
        .with_init_state(Some(state))
        .with_offline(true)
        .with_port(0);

    let (api, _handle) = spawn(config).await;

    let test_address: Address = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266".parse().unwrap();

    // Send a simple value transfer
    let tx = alloy_rpc_types::TransactionRequest {
        from: Some(test_address),
        to: Some(test_address.into()),
        value: Some(U256::from(1000)),
        ..Default::default()
    };

    let tx_hash = api.send_transaction(WithOtherFields::new(tx)).await.unwrap();
    assert!(!tx_hash.is_zero());

    // Mine a block to include the transaction
    api.mine_one().await;

    // Verify the new block was created
    let new_block_number = api.block_number().unwrap();
    assert_eq!(new_block_number, U256::from(20000001));
}
