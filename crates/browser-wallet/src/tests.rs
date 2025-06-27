#[cfg(test)]
use crate::{state::BrowserWalletState, BrowserTransaction, TransactionResponse, WalletConnection};
#[cfg(test)]
use alloy_primitives::{address, B256, U256};
#[cfg(test)]
use alloy_rpc_types::TransactionRequest;

#[test]
fn test_browser_transaction_serialization() {
    let tx = BrowserTransaction {
        id: "test-123".to_string(),
        request: TransactionRequest {
            from: Some(address!("0000000000000000000000000000000000000001")),
            to: Some(address!("0000000000000000000000000000000000000002").into()),
            value: Some(U256::from(1000)),
            ..Default::default()
        },
    };

    let json = serde_json::to_string_pretty(&tx).unwrap();

    let deserialized: BrowserTransaction = serde_json::from_str(&json).unwrap();

    assert_eq!(tx.id, deserialized.id);
    assert_eq!(tx.request.from, deserialized.request.from);
    assert_eq!(tx.request.to, deserialized.request.to);
    assert_eq!(tx.request.value, deserialized.request.value);
}

#[test]
fn test_wallet_connection() {
    let connection = WalletConnection {
        address: address!("0000000000000000000000000000000000000001"),
        chain_id: 1,
        wallet_name: Some("MetaMask".to_string()),
    };

    let json = serde_json::to_string(&connection).unwrap();
    let deserialized: WalletConnection = serde_json::from_str(&json).unwrap();

    assert_eq!(connection.address, deserialized.address);
    assert_eq!(connection.chain_id, deserialized.chain_id);
    assert_eq!(connection.wallet_name, deserialized.wallet_name);
}

#[test]
fn test_transaction_hex_serialization() {
    use alloy_primitives::U256;

    let tx = TransactionRequest {
        from: Some(address!("0000000000000000000000000000000000000001")),
        to: Some(address!("0000000000000000000000000000000000000002").into()),
        value: Some(U256::from(1_000_000_000_000_000_000u64)), // 1 ETH
        chain_id: Some(31337),
        ..Default::default()
    };

    let browser_tx = BrowserTransaction { id: "test-1".to_string(), request: tx };

    let json = serde_json::to_string_pretty(&browser_tx).unwrap();

    // Check that hex values are properly formatted
    assert!(json.contains("\"value\": \"0x"));
    assert!(json.contains("\"chainId\": \"0x"));

    // Ensure no double hex encoding
    assert!(!json.contains("\"0x0x"));
    assert!(!json.contains("0x0x0x"));
}

#[test]
fn test_transaction_with_empty_data() {
    use alloy_primitives::U256;

    let tx = TransactionRequest {
        from: Some(address!("0000000000000000000000000000000000000001")),
        to: Some(address!("0000000000000000000000000000000000000002").into()),
        value: Some(U256::ZERO),
        chain_id: Some(31337),
        ..Default::default()
    };
    // tx.data is None by default

    let browser_tx = BrowserTransaction { id: "test-2".to_string(), request: tx };

    let json = serde_json::to_string_pretty(&browser_tx).unwrap();

    // Check that empty values are handled correctly
    assert!(json.contains("\"value\": \"0x0\""));
    // data should be absent if None
    assert!(!json.contains("\"data\"") || json.contains("\"data\": null"));
}

#[tokio::test]
async fn test_state_management() {
    let state = BrowserWalletState::new();

    // Test wallet connection
    assert!(state.get_connected_address().is_none());

    state.set_connected_address(Some("0x0000000000000000000000000000000000000001".to_string()));
    assert_eq!(
        state.get_connected_address(),
        Some("0x0000000000000000000000000000000000000001".to_string())
    );

    // Test transaction queue
    let tx = BrowserTransaction { id: "tx-1".to_string(), request: TransactionRequest::default() };

    state.add_transaction_request(tx);
    assert_eq!(state.get_pending_transaction().unwrap().id, "tx-1");

    // Test response handling
    let response =
        TransactionResponse { id: "tx-1".to_string(), hash: Some(B256::default()), error: None };

    state.add_transaction_response(response);
    assert!(state.get_pending_transaction().is_none());
    assert!(state.get_transaction_response("tx-1").is_some());
}
