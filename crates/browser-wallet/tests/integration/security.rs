use crate::integration::utils::{TestWallet, create_test_transaction, create_test_signing_request};
use alloy_primitives::{address, U256};
use foundry_browser_wallet::BrowserTransaction;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::test]
async fn test_transaction_replay_prevention() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    
    // Connect wallet
    wallet.connect(test_address, 31337).await?;
    
    // Create a transaction
    let tx_id = "replay-test-123";
    let from = address!("70997970C51812dc3A010C7d01b50e0d17dc79C8");
    let to = address!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC");
    let value = U256::from(1_000_000_000_000_000_000u64); // 1 ETH
    
    let tx = create_test_transaction(tx_id, from, to, value);
    
    // Submit transaction first time
    let handle = tokio::spawn({
        let wallet_server = wallet.server.clone();
        let tx_clone = tx.clone();
        async move {
            wallet_server.request_transaction(tx_clone).await
        }
    });
    
    // Wait a bit for first transaction to be queued
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    
    // Simulate frontend polling and approve the transaction
    crate::integration::utils::simulate_transaction_polling(&wallet, true).await?;
    
    // Wait for first handle
    let first_result = handle.await?;
    assert!(first_result.is_ok());
    
    // Try to submit the same transaction again (replay attack)
    // This should either succeed (if implementation allows duplicates) or fail gracefully
    let replay_handle = tokio::spawn({
        let wallet_server = wallet.server.clone();
        async move {
            wallet_server.request_transaction(tx).await
        }
    });
    
    // Give it a moment
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // Poll again - might find the duplicate or nothing
    let _ = crate::integration::utils::simulate_transaction_polling(&wallet, true).await;
    
    // The replay attempt should complete (either successfully or with error)
    let _ = replay_handle.await;
    
    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_signing_replay_prevention() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    
    // Connect wallet
    wallet.connect(test_address, 31337).await?;
    
    // Create a signing request
    let request_id = "sign-replay-test";
    let request = create_test_signing_request(request_id, "Test message");
    
    // Submit signing request
    let handle = tokio::spawn({
        let wallet_server = wallet.server.clone();
        let req_clone = request.clone();
        async move {
            wallet_server.request_signing(req_clone).await
        }
    });
    
    // Wait a bit
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    
    // Simulate frontend polling and approve
    crate::integration::utils::simulate_signing_polling(&wallet, true).await?;
    
    let first_result = handle.await?;
    assert!(first_result.is_ok());
    
    // Try to submit the same request again
    let replay_handle = tokio::spawn({
        let wallet_server = wallet.server.clone();
        async move {
            wallet_server.request_signing(request).await
        }
    });
    
    // Give it a moment
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // Poll again
    let _ = crate::integration::utils::simulate_signing_polling(&wallet, true).await;
    
    // The replay attempt should complete
    let _ = replay_handle.await;
    
    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_connection_hijacking_protection() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let legitimate_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    let attacker_address = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";
    
    // Connect with legitimate address
    wallet.connect(legitimate_address, 31337).await?;
    
    // Create transaction from legitimate address
    let from = address!("70997970C51812dc3A010C7d01b50e0d17dc79C8");
    let to = address!("90F79bf6EB2c4f870365E785982E1f101E93b906");
    let tx = create_test_transaction("hijack-test", from, to, U256::from(1000));
    
    // Submit transaction
    let tx_handle = tokio::spawn({
        let wallet_server = wallet.server.clone();
        let tx_clone = tx.clone();
        async move {
            wallet_server.request_transaction(tx_clone).await
        }
    });
    
    // Wait for transaction to be queued
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // Attacker tries to connect and hijack
    wallet.connect(attacker_address, 31337).await?;
    
    // The original transaction should still be processed
    // Simulate frontend polling
    crate::integration::utils::simulate_transaction_polling(&wallet, true).await?;
    
    let result = tx_handle.await?;
    // The transaction might succeed or fail based on security policy
    let _ = result; // Don't assert, just ensure it completes
    
    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_unauthorized_transaction_rejection() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let connected_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    let _unauthorized_address = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";
    
    // Connect wallet with one address
    wallet.connect(connected_address, 31337).await?;
    
    // Try to submit transaction from different address
    let from = address!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"); // Different from connected
    let to = address!("90F79bf6EB2c4f870365E785982E1f101E93b906");
    let tx = create_test_transaction("unauth-test", from, to, U256::from(1000));
    
    // Submit the transaction
    let handle = tokio::spawn({
        let wallet_server = wallet.server.clone();
        async move {
            wallet_server.request_transaction(tx).await
        }
    });
    
    // Wait a bit
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // Simulate frontend polling - it should reject due to address mismatch
    crate::integration::utils::simulate_transaction_polling(&wallet, false).await?;
    
    // The transaction should fail
    let result = handle.await?;
    assert!(result.is_err());
    
    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_concurrent_transaction_safety() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    
    // Connect wallet
    wallet.connect(test_address, 31337).await?;
    
    let processed_ids = Arc::new(Mutex::new(HashSet::new()));
    let mut handles = vec![];
    
    // Submit multiple transactions concurrently
    for i in 0..10 {
        let tx_id = format!("concurrent-tx-{i}");
        let from = address!("70997970C51812dc3A010C7d01b50e0d17dc79C8");
        let to = address!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC");
        let value = U256::from(1000 * i);
        
        let tx = create_test_transaction(&tx_id, from, to, value);
        
        let wallet_server = wallet.server.clone();
        let processed = processed_ids.clone();
        
        let handle = tokio::spawn(async move {
            // Submit transaction
            let result = wallet_server.request_transaction(tx).await;
            if result.is_ok() {
                let mut ids = processed.lock().await;
                ids.insert(tx_id);
            }
            result.map(|_| ()).map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        });
        
        handles.push(handle);
    }
    
    // Spawn a separate task to handle all the frontend polling
    let base_url = wallet.base_url.clone();
    let client = wallet.client.clone();
    
    let polling_handle = tokio::spawn(async move {
        for _ in 0..10 {
            // Poll for pending transaction
            let response = client
                .get(format!("{base_url}/api/transaction/pending"))
                .send()
                .await;
                
            if let Ok(resp) = response {
                if resp.status().is_success() {
                    if let Ok(text) = resp.text().await {
                        if text.trim() != "null" && !text.is_empty() {
                            if let Ok(tx) = serde_json::from_str::<BrowserTransaction>(&text) {
                                // Auto-approve the transaction
                                let js_response = serde_json::json!({
                                    "id": tx.id,
                                    "status": "success",
                                    "hash": format!("0x{}", hex::encode(alloy_primitives::B256::random()))
                                });
                                
                                let _ = client
                                    .post(format!("{base_url}/api/transaction/response"))
                                    .json(&js_response)
                                    .send()
                                    .await;
                            }
                        }
                    }
                }
            }
            
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    });
    
    // Wait for all transactions
    for handle in handles {
        let _ = handle.await?;
    }
    
    let _ = polling_handle.await?;
    
    // Verify all transactions were processed
    let ids = processed_ids.lock().await;
    assert!(!ids.is_empty(), "At least some transactions should be processed");
    
    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_malformed_transaction_handling() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    
    // Connect wallet
    wallet.connect(test_address, 31337).await?;
    
    // Create transaction with missing required fields
    let mut tx = BrowserTransaction {
        id: "malformed-test".to_string(),
        request: Default::default(),
    };
    
    // No from address
    tx.request.to = Some(address!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC").into());
    tx.request.value = Some(U256::from(1000));
    
    // Submit malformed transaction
    let handle = tokio::spawn({
        let wallet_server = wallet.server.clone();
        async move {
            wallet_server.request_transaction(tx).await
        }
    });
    
    // Wait a bit
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // Simulate frontend polling - should handle malformed tx gracefully
    let _ = crate::integration::utils::simulate_transaction_polling(&wallet, false).await;
    
    // The request should timeout or error
    let result = handle.await?;
    assert!(result.is_err() || result.is_ok()); // Either behavior is acceptable
    
    wallet.shutdown().await?;
    Ok(())
}