use crate::integration::utils::{TestWallet, create_test_transaction, wait_for};
use alloy_primitives::{address, B256, U256, Bytes};
use alloy_rpc_types::TransactionRequest;
use foundry_browser_wallet::{BrowserTransaction, TransactionResponse};
use std::time::Duration;

#[tokio::test]
async fn test_transaction_approval_flow() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    
    // Connect wallet
    wallet.connect(test_address, 31337).await?;
    
    // Create transaction
    let from = address!("70997970C51812dc3A010C7d01b50e0d17dc79C8");
    let to = address!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC");
    let value = U256::from(1_000_000_000_000_000_000u64); // 1 ETH
    let tx = create_test_transaction("approval-test", from, to, value);
    
    // Submit transaction in background
    let wallet_server = wallet.server.clone();
    let tx_clone = tx.clone();
    let handle = tokio::spawn(async move {
        wallet_server.request_transaction(tx_clone).await
    });
    
    // Simulate frontend polling for pending transaction
    wait_for(
        || async { wallet.get_pending_transaction().await.map(|opt| opt.is_some()).unwrap_or(false) },
        Duration::from_secs(5),
    ).await?;
    
    // Verify transaction details
    if let Some(pending_tx) = wallet.get_pending_transaction().await? {
        assert_eq!(pending_tx.id, "approval-test");
        assert_eq!(pending_tx.request.from, Some(from));
        assert_eq!(pending_tx.request.to, Some(to.into()));
        assert_eq!(pending_tx.request.value, Some(value));
    } else {
        panic!("Expected pending transaction");
    }
    
    // Approve transaction
    let tx_hash = B256::random();
    wallet.report_transaction_result(TransactionResponse {
        id: "approval-test".to_string(),
        hash: Some(tx_hash),
        error: None,
    }).await?;
    
    // Wait for transaction completion
    let result = handle.await??;
    assert_eq!(result, tx_hash);
    
    // Verify no pending transaction remains
    assert!(wallet.get_pending_transaction().await?.is_none());
    
    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_transaction_rejection_flow() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    
    // Connect wallet
    wallet.connect(test_address, 31337).await?;
    
    // Create transaction
    let tx = create_test_transaction(
        "rejection-test",
        address!("70997970C51812dc3A010C7d01b50e0d17dc79C8"),
        address!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"),
        U256::from(1000),
    );
    
    // Submit transaction
    let wallet_server = wallet.server.clone();
    let tx_clone = tx.clone();
    let handle = tokio::spawn(async move {
        wallet_server.request_transaction(tx_clone).await
    });
    
    // Wait for pending transaction
    wait_for(
        || async { wallet.get_pending_transaction().await.map(|opt| opt.is_some()).unwrap_or(false) },
        Duration::from_secs(5),
    ).await?;
    
    // Reject transaction
    wallet.report_transaction_result(TransactionResponse {
        id: "rejection-test".to_string(),
        hash: None,
        error: Some("User rejected transaction".to_string()),
    }).await?;
    
    // Verify rejection
    let result = handle.await?;
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(e.to_string().contains("User rejected"));
    }
    
    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_transaction_with_data() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    
    // Connect wallet
    wallet.connect(test_address, 31337).await?;
    
    // Create transaction with contract data
    let tx = BrowserTransaction {
        id: "data-test".to_string(),
        request: TransactionRequest {
            from: Some(address!("70997970C51812dc3A010C7d01b50e0d17dc79C8")),
            to: Some(address!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC").into()),
            value: Some(U256::ZERO),
            input: alloy_rpc_types::TransactionInput::new(Bytes::from(vec![0x12, 0x34, 0x56, 0x78])),
            gas: Some(100000),
            ..Default::default()
        },
    };
    
    // Submit transaction
    let wallet_server = wallet.server.clone();
    let handle = tokio::spawn(async move {
        wallet_server.request_transaction(tx).await
    });
    
    // Wait and check pending transaction
    wait_for(
        || async { wallet.get_pending_transaction().await.map(|opt| opt.is_some()).unwrap_or(false) },
        Duration::from_secs(5),
    ).await?;
    
    let pending = wallet.get_pending_transaction().await?.unwrap();
    // Check that input data is present
    let input_bytes = pending.request.input.input().cloned().unwrap_or_default();
    assert!(!input_bytes.is_empty());
    
    // Approve
    wallet.report_transaction_result(TransactionResponse {
        id: "data-test".to_string(),
        hash: Some(B256::random()),
        error: None,
    }).await?;
    
    handle.await??;
    
    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_transaction_timeout() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    
    // Connect wallet
    wallet.connect(test_address, 31337).await?;
    
    // Create transaction with very short timeout
    let tx = create_test_transaction(
        "timeout-test",
        address!("70997970C51812dc3A010C7d01b50e0d17dc79C8"),
        address!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"),
        U256::from(1000),
    );
    
    // Submit transaction but don't respond
    let wallet_server = wallet.server.clone();
    let handle = tokio::spawn(async move {
        // Use a shorter timeout for testing
        tokio::time::timeout(
            Duration::from_secs(2),
            wallet_server.request_transaction(tx)
        ).await
    });
    
    // Let it timeout
    let result = handle.await?;
    assert!(result.is_err() || result.unwrap().is_err());
    
    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_multiple_transactions_queue() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    
    // Connect wallet
    wallet.connect(test_address, 31337).await?;
    
    let from = address!("70997970C51812dc3A010C7d01b50e0d17dc79C8");
    let to = address!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC");
    
    // Submit multiple transactions
    let tx1 = create_test_transaction("queue-1", from, to, U256::from(1000));
    let tx2 = create_test_transaction("queue-2", from, to, U256::from(2000));
    let tx3 = create_test_transaction("queue-3", from, to, U256::from(3000));
    
    // Submit all transactions quickly
    let wallet_server = wallet.server.clone();
    let handles = vec![
        {
            let server = wallet_server.clone();
            let tx = tx1.clone();
            tokio::spawn(async move { server.request_transaction(tx).await })
        },
        {
            let server = wallet_server.clone();
            let tx = tx2.clone();
            tokio::spawn(async move { server.request_transaction(tx).await })
        },
        {
            let server = wallet_server.clone();
            let tx = tx3.clone();
            tokio::spawn(async move { server.request_transaction(tx).await })
        },
    ];
    
    // Process transactions in order
    for _i in 1..=3 {
        // Wait for pending transaction
        wait_for(
            || async { wallet.get_pending_transaction().await.map(|opt| opt.is_some()).unwrap_or(false) },
            Duration::from_secs(5),
        ).await?;
        
        let pending = wallet.get_pending_transaction().await?.unwrap();
        
        // Approve transaction
        wallet.report_transaction_result(TransactionResponse {
            id: pending.id.clone(),
            hash: Some(B256::random()),
            error: None,
        }).await?;
        
        // Small delay to ensure processing
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    
    // Wait for all handles
    for handle in handles {
        handle.await??;
    }
    
    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_transaction_with_gas_settings() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    
    // Connect wallet
    wallet.connect(test_address, 31337).await?;
    
    // Create transaction with EIP-1559 gas settings
    let tx = BrowserTransaction {
        id: "gas-test".to_string(),
        request: TransactionRequest {
            from: Some(address!("70997970C51812dc3A010C7d01b50e0d17dc79C8")),
            to: Some(address!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC").into()),
            value: Some(U256::from(1000)),
            gas: Some(21000),
            max_fee_per_gas: Some(30_000_000_000u128), // 30 gwei
            max_priority_fee_per_gas: Some(1_000_000_000u128), // 1 gwei
            ..Default::default()
        },
    };
    
    // Submit and process
    let wallet_server = wallet.server.clone();
    let handle = tokio::spawn(async move {
        wallet_server.request_transaction(tx).await
    });
    
    wait_for(
        || async { wallet.get_pending_transaction().await.map(|opt| opt.is_some()).unwrap_or(false) },
        Duration::from_secs(5),
    ).await?;
    
    let pending = wallet.get_pending_transaction().await?.unwrap();
    assert_eq!(pending.request.gas, Some(21000));
    assert!(pending.request.max_fee_per_gas.is_some());
    assert!(pending.request.max_priority_fee_per_gas.is_some());
    
    // Approve
    wallet.report_transaction_result(TransactionResponse {
        id: "gas-test".to_string(),
        hash: Some(B256::random()),
        error: None,
    }).await?;
    
    handle.await??;
    
    wallet.shutdown().await?;
    Ok(())
}