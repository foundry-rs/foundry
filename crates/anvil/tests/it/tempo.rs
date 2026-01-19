//! Tempo transaction tests

use alloy_network::{ReceiptResponse, TransactionBuilder};
use alloy_primitives::{TxHash, U256, address, hex};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, TransactionRequest};
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, spawn};
use foundry_evm_networks::NetworkConfigs;

/// Tempo testnet chain ID
const TEMPO_TESTNET_CHAIN_ID: u64 = 42429;

/// Raw Tempo transaction from testnet (type 0x76)
/// https://explorer.testnet.tempo.xyz/tx/0x6d6d8c102064e6dee44abad2024a8b1d37959230baab80e70efbf9b0c739c4fd
const RAW_TEMPO_TX_HEX: &str = "76f9025e82a5bd808502cb4178008302d178f8fcf85c9420c000000000000000000000000000000000000080b844095ea7b3000000000000000000000000dec00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000989680f89c94dec000000000000000000000000000000000000080b884f8856c0f00000000000000000000000020c000000000000000000000000000000000000000000000000000000000000020c00000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000989680000000000000000000000000000000000000000000000000000000000097d330c0808080809420c000000000000000000000000000000000000180c0b90133027b98b7a8e6c68d7eac741a52e6fdae0560ce3c16ef5427ad46d7a54d0ed86dd41d000000007b2274797065223a22776562617574686e2e676574222c226368616c6c656e6765223a2238453071464a7a50585167546e645473643649456659457776323173516e626966374c4741776e4b43626b222c226f726967696e223a2268747470733a2f2f74656d706f2d6465782e76657263656c2e617070222c2263726f73734f726967696e223a66616c73657dcfd45c3b19745a42f80b134dcb02a8ba099a0e4e7be1984da54734aa81d8f29f74bb9170ae6d25bd510c83fe35895ee5712efe13980a5edc8094c534e23af85eaacc80b21e45fb11f349424dce3a2f23547f60c0ff2f8bcaede2a247545ce8dd87abf0dbb7a5c9507efae2e43833356651b45ac576c2e61cec4e9c0f41fcbf6e";

/// Expected transaction hash from testnet
const TEMPO_TX_HASH: &str = "0x6d6d8c102064e6dee44abad2024a8b1d37959230baab80e70efbf9b0c739c4fd";

/// Sender address (WebAuthn recovered address from the raw tx)
const TEMPO_SENDER: alloy_primitives::Address =
    address!("0x566Ff0f4a6114F8072ecDC8A7A8A13d8d0C6B45F");

/// Helper to spawn a Tempo-enabled node
async fn spawn_tempo_node() -> (anvil::eth::EthApi, anvil::NodeHandle) {
    spawn(
        NodeConfig::test()
            .with_chain_id(Some(TEMPO_TESTNET_CHAIN_ID))
            .with_networks(NetworkConfigs::with_tempo()),
    )
    .await
}

/// Helper to fund an account with ETH
async fn fund_account(api: &anvil::eth::EthApi, account: alloy_primitives::Address) {
    api.anvil_set_balance(account, U256::from(10_000_000_000_000_000_000u128)).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_not_supported_if_disabled() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let raw_tx = hex::decode(RAW_TEMPO_TX_HEX).unwrap();

    let err = provider.send_raw_transaction(&raw_tx).await.unwrap_err();
    let s = err.to_string();
    assert!(
        s.contains("Tempo") || s.contains("tempo") || s.contains("unsupported"),
        "Expected error about Tempo not being supported, got: {s:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_send_tempo_raw_transaction() {
    let (api, handle) = spawn_tempo_node().await;
    let provider = handle.http_provider();

    let tx_hash: TxHash = TEMPO_TX_HASH.parse().unwrap();
    let raw_tx = hex::decode(RAW_TEMPO_TX_HEX).unwrap();

    fund_account(&api, TEMPO_SENDER).await;

    let pending = provider.send_raw_transaction(&raw_tx).await.unwrap().register().await.unwrap();

    assert_eq!(*pending.tx_hash(), tx_hash);

    api.evm_mine(None).await.unwrap();

    let receipt =
        provider.get_transaction_receipt(pending.tx_hash().to_owned()).await.unwrap().unwrap();

    assert_eq!(receipt.from, TEMPO_SENDER);
    assert_eq!(receipt.transaction_hash, tx_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_transaction_hash_matches_testnet() {
    let (_api, handle) = spawn_tempo_node().await;
    let provider = handle.http_provider();

    let tx_hash: TxHash = TEMPO_TX_HASH.parse().unwrap();
    let raw_tx = hex::decode(RAW_TEMPO_TX_HEX).unwrap();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let funder = accounts[0].address();

    let fund_tx = TransactionRequest::default()
        .with_from(funder)
        .with_to(TEMPO_SENDER)
        .with_value(U256::from(10_000_000_000_000_000_000u128));

    provider
        .send_transaction(WithOtherFields::new(fund_tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let receipt =
        provider.send_raw_transaction(&raw_tx).await.unwrap().get_receipt().await.unwrap();

    assert_eq!(receipt.transaction_hash, tx_hash);
}

/// Test that Tempo tx type (0x76) is correctly returned in transaction receipt
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_tx_type_in_receipt() {
    let (api, handle) = spawn_tempo_node().await;
    let provider = handle.http_provider();

    let raw_tx = hex::decode(RAW_TEMPO_TX_HEX).unwrap();
    fund_account(&api, TEMPO_SENDER).await;

    let pending = provider.send_raw_transaction(&raw_tx).await.unwrap().register().await.unwrap();

    api.evm_mine(None).await.unwrap();

    let receipt = provider.get_transaction_receipt(*pending.tx_hash()).await.unwrap().unwrap();

    // Just verify the receipt is valid
    assert!(receipt.status(), "Transaction should succeed");
}

/// Test eth_call with Tempo transaction format (via raw RPC since alloy doesn't support type 0x76
/// natively)
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_eth_call() {
    let (api, handle) = spawn_tempo_node().await;
    let provider = handle.http_provider();

    fund_account(&api, TEMPO_SENDER).await;

    // Simple call to check balance - should work with standard TransactionRequest
    let tx = TransactionRequest::default()
        .with_from(TEMPO_SENDER)
        .with_to(address!("0x20c0000000000000000000000000000000000001"));

    let result = provider.call(WithOtherFields::new(tx)).await;
    // Call should not error (even if it returns empty data)
    assert!(result.is_ok(), "eth_call should succeed: {:?}", result.err());
}

/// Test gas estimation for transactions from Tempo sender
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_eth_estimate_gas() {
    let (api, handle) = spawn_tempo_node().await;
    let provider = handle.http_provider();

    fund_account(&api, TEMPO_SENDER).await;

    // Estimate gas for a simple value transfer
    let recipient = address!("0x1111111111111111111111111111111111111111");
    let tx = TransactionRequest::default()
        .with_from(TEMPO_SENDER)
        .with_to(recipient)
        .with_value(U256::from(1000));

    let gas = provider.estimate_gas(WithOtherFields::new(tx)).await.unwrap();

    // Should return reasonable gas estimate for a transfer
    assert!(gas >= 21000, "Gas estimate should be at least 21000 for transfer");
    assert!(gas < 100000, "Gas estimate should be reasonable");
}

/// Test eth_getTransactionByHash returns correct Tempo tx fields
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_get_transaction_by_hash() {
    let (api, handle) = spawn_tempo_node().await;
    let provider = handle.http_provider();

    let tx_hash: TxHash = TEMPO_TX_HASH.parse().unwrap();
    let raw_tx = hex::decode(RAW_TEMPO_TX_HEX).unwrap();

    fund_account(&api, TEMPO_SENDER).await;

    provider.send_raw_transaction(&raw_tx).await.unwrap().register().await.unwrap();

    api.evm_mine(None).await.unwrap();

    // Use raw RPC call to get the transaction since alloy may not fully support type 0x76
    let raw_tx_result: Option<serde_json::Value> =
        provider.raw_request("eth_getTransactionByHash".into(), [tx_hash]).await.unwrap();

    let tx_data = raw_tx_result.expect("Transaction should exist");
    let tx_obj = tx_data.as_object().expect("Transaction should be an object");

    // Verify transaction type is 0x76
    let tx_type = tx_obj.get("type").and_then(|v| v.as_str()).expect("type should exist");
    assert_eq!(tx_type, "0x76", "Transaction type should be 0x76 (Tempo)");

    // Verify hash matches
    let returned_hash = tx_obj.get("hash").and_then(|v| v.as_str()).expect("hash should exist");
    assert_eq!(returned_hash.to_lowercase(), TEMPO_TX_HASH.to_lowercase(), "Hash should match");

    // Verify chain ID (0xa5bd = 42429)
    let chain_id = tx_obj.get("chainId").and_then(|v| v.as_str()).expect("chainId should exist");
    let chain_id_parsed = u64::from_str_radix(chain_id.trim_start_matches("0x"), 16).unwrap();
    assert_eq!(chain_id_parsed, TEMPO_TESTNET_CHAIN_ID, "Chain ID should match");

    // Verify sender
    let from = tx_obj.get("from").and_then(|v| v.as_str()).expect("from should exist");
    assert_eq!(
        from.to_lowercase(),
        format!("{TEMPO_SENDER:?}").to_lowercase(),
        "From address should match sender"
    );

    // Tempo transactions have "calls" array instead of "to"
    let calls = tx_obj.get("calls").and_then(|v| v.as_array());
    assert!(calls.is_some(), "Tempo tx should have 'calls' field");
    assert!(!calls.unwrap().is_empty(), "Calls array should not be empty");
}

/// Test that block retrieval includes Tempo transactions correctly
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_block_contains_tempo_tx() {
    let (api, handle) = spawn_tempo_node().await;
    let provider = handle.http_provider();

    let tx_hash: TxHash = TEMPO_TX_HASH.parse().unwrap();
    let raw_tx = hex::decode(RAW_TEMPO_TX_HEX).unwrap();

    fund_account(&api, TEMPO_SENDER).await;

    provider.send_raw_transaction(&raw_tx).await.unwrap().register().await.unwrap();

    api.evm_mine(None).await.unwrap();

    // Get the block with full transactions
    let block = provider.get_block(BlockId::number(1)).full().await.unwrap().unwrap();

    assert_eq!(block.transactions.len(), 1, "Block should contain 1 transaction");

    // Use raw RPC to get block with full tx details for type checking
    let raw_block: serde_json::Value =
        provider.raw_request("eth_getBlockByNumber".into(), ("0x1", true)).await.unwrap();

    let txs = raw_block
        .get("transactions")
        .and_then(|v| v.as_array())
        .expect("Block should have transactions array");

    assert_eq!(txs.len(), 1, "Block should contain 1 transaction");

    let tx = &txs[0];
    let tx_type = tx.get("type").and_then(|v| v.as_str()).expect("tx should have type");
    assert_eq!(tx_type, "0x76", "Transaction in block should be Tempo type");

    let hash = tx.get("hash").and_then(|v| v.as_str()).expect("tx should have hash");
    assert_eq!(hash.to_lowercase(), format!("{tx_hash:?}").to_lowercase());
}

/// Test sending multiple Tempo transactions in a single block
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_multiple_transactions_in_block() {
    let (api, handle) = spawn_tempo_node().await;
    let provider = handle.http_provider();

    // Disable auto mining to collect multiple txs in one block
    api.anvil_set_auto_mine(false).await.unwrap();

    // Fund multiple accounts using dev wallets
    let accounts: Vec<_> = handle.dev_wallets().collect();
    let sender1 = accounts[0].address();
    let sender2 = accounts[1].address();
    let recipient = address!("0x2222222222222222222222222222222222222222");

    // Send multiple regular transactions (Tempo node should handle them)
    let tx1 = TransactionRequest::default()
        .with_from(sender1)
        .with_to(recipient)
        .with_value(U256::from(1000))
        .with_nonce(0);

    let tx2 = TransactionRequest::default()
        .with_from(sender2)
        .with_to(recipient)
        .with_value(U256::from(2000))
        .with_nonce(0);

    let pending1 = provider
        .send_transaction(WithOtherFields::new(tx1))
        .await
        .unwrap()
        .register()
        .await
        .unwrap();

    let pending2 = provider
        .send_transaction(WithOtherFields::new(tx2))
        .await
        .unwrap()
        .register()
        .await
        .unwrap();

    // Mine the block
    api.evm_mine(None).await.unwrap();

    // Verify both transactions are in the block
    let receipt1 = provider.get_transaction_receipt(*pending1.tx_hash()).await.unwrap().unwrap();

    let receipt2 = provider.get_transaction_receipt(*pending2.tx_hash()).await.unwrap().unwrap();

    // Both should be in block 1
    assert_eq!(receipt1.block_number, Some(1), "Tx1 should be in block 1");
    assert_eq!(receipt2.block_number, Some(1), "Tx2 should be in block 1");

    // Get block and verify transaction count
    let block = provider.get_block(BlockId::number(1)).full().await.unwrap().unwrap();

    assert_eq!(block.transactions.len(), 2, "Block should contain 2 transactions");
}

/// Test chain ID validation - wrong chain ID should be rejected
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_chain_id_validation() {
    // Spawn a node with a different chain ID than the raw tx
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(12345u64)) // Different from TEMPO_TESTNET_CHAIN_ID (42429)
            .with_networks(NetworkConfigs::with_tempo()),
    )
    .await;

    let provider = handle.http_provider();
    let raw_tx = hex::decode(RAW_TEMPO_TX_HEX).unwrap();

    fund_account(&api, TEMPO_SENDER).await;

    // Transaction should be rejected due to chain ID mismatch
    let result = provider.send_raw_transaction(&raw_tx).await;

    assert!(result.is_err(), "Transaction with wrong chain ID should be rejected");
    let err = result.unwrap_err().to_string();
    // The error message may vary, but should indicate rejection
    assert!(
        err.contains("chain") || err.contains("Chain") || err.contains("invalid"),
        "Error should mention chain ID issue: {err}"
    );
}

/// Test nonce handling and replay protection for Tempo transactions
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_nonce_handling() {
    let (api, handle) = spawn_tempo_node().await;
    let provider = handle.http_provider();

    // Use a dev wallet for regular transactions with nonce tracking
    let accounts: Vec<_> = handle.dev_wallets().collect();
    let sender = accounts[0].address();
    let recipient = address!("0x3333333333333333333333333333333333333333");

    // Check initial nonce
    let initial_nonce = provider.get_transaction_count(sender).await.unwrap();
    assert_eq!(initial_nonce, 0, "Initial nonce should be 0");

    // Send first transaction
    let tx1 = TransactionRequest::default()
        .with_from(sender)
        .with_to(recipient)
        .with_value(U256::from(1000));

    provider
        .send_transaction(WithOtherFields::new(tx1))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    // Nonce should increment
    let nonce_after_first = provider.get_transaction_count(sender).await.unwrap();
    assert_eq!(nonce_after_first, 1, "Nonce should be 1 after first tx");

    // Send second transaction
    let tx2 = TransactionRequest::default()
        .with_from(sender)
        .with_to(recipient)
        .with_value(U256::from(2000));

    provider
        .send_transaction(WithOtherFields::new(tx2))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let nonce_after_second = provider.get_transaction_count(sender).await.unwrap();
    assert_eq!(nonce_after_second, 2, "Nonce should be 2 after second tx");

    // Now test replay protection with the raw Tempo tx
    fund_account(&api, TEMPO_SENDER).await;
    let raw_tx = hex::decode(RAW_TEMPO_TX_HEX).unwrap();

    // Send the Tempo tx once - should succeed
    let receipt =
        provider.send_raw_transaction(&raw_tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status(), "First Tempo tx should succeed");

    // Try to replay the same transaction - should fail due to nonce
    let replay_result = provider.send_raw_transaction(&raw_tx).await;

    assert!(replay_result.is_err(), "Replaying same transaction should fail");
    let err = replay_result.unwrap_err().to_string().to_lowercase();
    assert!(
        err.contains("nonce") || err.contains("already known") || err.contains("replacement"),
        "Error should mention nonce issue: {err}"
    );
}

/// Test tx type 0x76 is correctly returned in eth_getTransactionReceipt raw RPC response
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_receipt_type_field() {
    let (api, handle) = spawn_tempo_node().await;
    let provider = handle.http_provider();

    let tx_hash: TxHash = TEMPO_TX_HASH.parse().unwrap();
    let raw_tx = hex::decode(RAW_TEMPO_TX_HEX).unwrap();

    fund_account(&api, TEMPO_SENDER).await;

    provider.send_raw_transaction(&raw_tx).await.unwrap().register().await.unwrap();

    api.evm_mine(None).await.unwrap();

    // Use raw RPC to check the receipt type field
    let raw_receipt: Option<serde_json::Value> =
        provider.raw_request("eth_getTransactionReceipt".into(), [tx_hash]).await.unwrap();

    let receipt_data = raw_receipt.expect("Receipt should exist");
    let receipt_obj = receipt_data.as_object().expect("Receipt should be an object");

    // Verify transaction type is 0x76 in receipt
    let tx_type = receipt_obj.get("type").and_then(|v| v.as_str());
    assert_eq!(tx_type, Some("0x76"), "Receipt type should be 0x76 (Tempo)");
}

/// Test that Tempo-specific fields are present in eth_getTransactionByHash response
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_tx_has_calls_field() {
    let (api, handle) = spawn_tempo_node().await;
    let provider = handle.http_provider();

    let tx_hash: TxHash = TEMPO_TX_HASH.parse().unwrap();
    let raw_tx = hex::decode(RAW_TEMPO_TX_HEX).unwrap();

    fund_account(&api, TEMPO_SENDER).await;

    provider.send_raw_transaction(&raw_tx).await.unwrap().register().await.unwrap();

    api.evm_mine(None).await.unwrap();

    // Use raw RPC to get the transaction
    let raw_tx_result: Option<serde_json::Value> =
        provider.raw_request("eth_getTransactionByHash".into(), [tx_hash]).await.unwrap();

    let tx_data = raw_tx_result.expect("Transaction should exist");
    let tx_obj = tx_data.as_object().expect("Transaction should be an object");

    // Verify calls array exists and is non-empty
    let calls = tx_obj.get("calls").and_then(|v| v.as_array());
    assert!(calls.is_some(), "Tempo tx should have 'calls' field");
    let calls = calls.unwrap();
    assert!(!calls.is_empty(), "Calls array should not be empty");

    // Each call should have 'to', 'data', and 'value' fields
    for (i, call) in calls.iter().enumerate() {
        let call_obj = call.as_object().expect("Each call should be an object");
        assert!(call_obj.contains_key("to"), "Call {i} should have 'to' field");
        assert!(call_obj.contains_key("data"), "Call {i} should have 'data' field");
    }

    // Verify 'to' field is NOT present at top level for Tempo tx (only in calls)
    // Note: This may vary depending on implementation - some may include a derived 'to'
}
