use crate::integration::utils::{TestWallet, wait_for, create_test_transaction};
use std::time::Duration;

#[tokio::test] 
async fn test_connection_persistence_across_restarts() -> Result<(), Box<dyn std::error::Error>> {
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    let chain_id = 31337;
    
    // First session - connect wallet
    {
        let wallet = TestWallet::spawn().await?;
        
        // Connect wallet
        wallet.connect(test_address, chain_id).await?;
        
        // Verify connection
        assert!(wallet.server.is_connected());
        let conn = wallet.server.get_connection().unwrap();
        assert_eq!(conn.address.to_string().to_lowercase(), test_address.to_lowercase());
        
        // Shutdown server (simulating browser close)
        wallet.shutdown().await?;
    }
    
    // Second session - should auto-reconnect
    {
        let wallet = TestWallet::spawn().await?;
        
        // The frontend would normally handle auto-reconnection
        // For testing, we simulate the frontend checking stored state
        // In a real scenario, the JS would read localStorage and reconnect
        
        // Initially not connected (server side doesn't persist)
        assert!(!wallet.server.is_connected());
        
        // Simulate frontend auto-reconnection
        wallet.connect(test_address, chain_id).await?;
        
        // Verify reconnection
        assert!(wallet.server.is_connected());
        
        wallet.shutdown().await?;
    }
    
    Ok(())
}

#[tokio::test]
async fn test_chain_switch_persistence() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    
    // Connect to Anvil
    wallet.connect(test_address, 31337).await?;
    assert_eq!(wallet.server.get_connection().unwrap().chain_id, 31337);
    
    // Switch to mainnet
    wallet.connect(test_address, 1).await?;
    assert_eq!(wallet.server.get_connection().unwrap().chain_id, 1);
    
    // Switch to Polygon
    wallet.connect(test_address, 137).await?;
    assert_eq!(wallet.server.get_connection().unwrap().chain_id, 137);
    
    // Address should remain the same
    assert_eq!(
        wallet.server.get_connection().unwrap().address.to_string().to_lowercase(),
        test_address.to_lowercase()
    );
    
    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_multiple_account_switches() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let accounts = vec![
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC", 
        "0x90F79bf6EB2c4f870365E785982E1f101E93b906",
    ];
    
    // Switch between accounts
    for account in &accounts {
        wallet.connect(account, 31337).await?;
        
        let connection = wallet.server.get_connection().unwrap();
        assert_eq!(connection.address.to_string().to_lowercase(), account.to_lowercase());
        assert_eq!(connection.chain_id, 31337);
        
        // Small delay to simulate user interaction
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    // Final connection should be the last account
    let final_conn = wallet.server.get_connection().unwrap();
    assert_eq!(
        final_conn.address.to_string().to_lowercase(),
        accounts.last().unwrap().to_lowercase()
    );
    
    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_connection_state_during_transactions() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    
    // Connect wallet
    wallet.connect(test_address, 31337).await?;
    
    // Submit transaction
    let tx = create_test_transaction(
        "state-test",
        alloy_primitives::address!("70997970C51812dc3A010C7d01b50e0d17dc79C8"),
        alloy_primitives::address!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"),
        alloy_primitives::U256::from(1000),
    );
    
    let wallet_server = wallet.server.clone();
    let handle = tokio::spawn(async move {
        wallet_server.request_transaction(tx).await
    });
    
    // Connection should remain stable during transaction
    wait_for(
        || async { wallet.get_pending_transaction().await.map(|opt| opt.is_some()).unwrap_or(false) },
        Duration::from_secs(5),
    ).await?;
    
    // Verify connection is maintained
    assert!(wallet.server.is_connected());
    assert_eq!(
        wallet.server.get_connection().unwrap().address.to_string().to_lowercase(),
        test_address.to_lowercase()
    );
    
    // Complete transaction
    wallet.report_transaction_result(foundry_browser_wallet::TransactionResponse {
        id: "state-test".to_string(),
        hash: Some(alloy_primitives::B256::random()),
        error: None,
    }).await?;
    
    handle.await??;
    
    // Connection should still be active
    assert!(wallet.server.is_connected());
    
    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_disconnection_clears_state() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    
    // Connect wallet
    wallet.connect(test_address, 31337).await?;
    assert!(wallet.server.is_connected());
    
    // Submit a transaction
    let tx = create_test_transaction(
        "disconnect-test",
        alloy_primitives::address!("70997970C51812dc3A010C7d01b50e0d17dc79C8"),
        alloy_primitives::address!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"),
        alloy_primitives::U256::from(1000),
    );
    
    let wallet_server = wallet.server.clone();
    let tx_handle = tokio::spawn(async move {
        wallet_server.request_transaction(tx).await
    });
    
    // Wait for transaction to be pending
    wait_for(
        || async { wallet.get_pending_transaction().await.map(|opt| opt.is_some()).unwrap_or(false) },
        Duration::from_secs(5),
    ).await?;
    
    // Disconnect wallet
    wallet.disconnect().await?;
    
    // Verify disconnection
    assert!(!wallet.server.is_connected());
    assert!(wallet.server.get_connection().is_none());
    
    // Transaction should fail or be cancelled
    // Report error to complete the flow
    wallet.report_transaction_result(foundry_browser_wallet::TransactionResponse {
        id: "disconnect-test".to_string(),
        hash: None,
        error: Some("Wallet disconnected".to_string()),
    }).await?;
    
    let result = tx_handle.await?;
    assert!(result.is_err());
    
    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_rapid_connect_disconnect_cycles() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    
    // Perform rapid connect/disconnect cycles
    for _i in 0..5 {
        // Connect
        wallet.connect(test_address, 31337).await?;
        assert!(wallet.server.is_connected());
        
        // Small operation
        let network = wallet.get_network_details().await?;
        assert_eq!(network["chain_id"], 31337);
        
        // Disconnect
        wallet.disconnect().await?;
        assert!(!wallet.server.is_connected());
        
        // Minimal delay
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    
    // Final state should be disconnected
    assert!(!wallet.server.is_connected());
    
    wallet.shutdown().await?;
    Ok(())
}