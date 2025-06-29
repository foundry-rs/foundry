use crate::integration::utils::{create_test_signing_request, wait_for, TestWallet};
use alloy_primitives::{address, Bytes};
use foundry_browser_wallet::{SignRequest, SignResponse, SignType};
use std::time::Duration;

#[tokio::test]
async fn test_personal_sign_flow() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

    // Connect wallet
    wallet.connect(test_address, 31337).await?;

    // Create signing request
    let request = SignRequest {
        id: "personal-sign-test".to_string(),
        address: address!("70997970C51812dc3A010C7d01b50e0d17dc79C8"),
        message: "Hello, Foundry!".to_string(),
        sign_type: SignType::PersonalSign,
    };

    // Submit signing request
    let wallet_server = wallet.server.clone();
    let req_clone = request.clone();
    let handle = tokio::spawn(async move { wallet_server.request_signing(req_clone).await });

    // Wait for pending signing request
    wait_for(
        || async { wallet.get_pending_signing().await.map(|opt| opt.is_some()).unwrap_or(false) },
        Duration::from_secs(5),
    )
    .await?;

    // Verify request details
    let pending = wallet.get_pending_signing().await?.unwrap();
    assert_eq!(pending.id, "personal-sign-test");
    assert_eq!(pending.message, "Hello, Foundry!");
    assert_eq!(pending.sign_type, SignType::PersonalSign);

    // Approve signing
    let signature = Bytes::from(vec![0xde, 0xad, 0xbe, 0xef]);
    wallet
        .report_signing_result(SignResponse {
            id: "personal-sign-test".to_string(),
            signature: Some(signature.clone()),
            error: None,
        })
        .await?;

    // Wait for completion
    let result = handle.await??;
    assert_eq!(result, signature);

    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_typed_data_sign_flow() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

    // Connect wallet
    wallet.connect(test_address, 31337).await?;

    // Create EIP-712 typed data request
    let typed_data = r#"{
        "domain": {
            "name": "Test",
            "version": "1",
            "chainId": 31337
        },
        "types": {
            "Message": [
                {"name": "content", "type": "string"}
            ]
        },
        "message": {
            "content": "Test message"
        }
    }"#;

    let request = SignRequest {
        id: "typed-data-test".to_string(),
        address: address!("70997970C51812dc3A010C7d01b50e0d17dc79C8"),
        message: typed_data.to_string(),
        sign_type: SignType::SignTypedData,
    };

    // Submit and process
    let wallet_server = wallet.server.clone();
    let handle = tokio::spawn(async move { wallet_server.request_signing(request).await });

    wait_for(
        || async { wallet.get_pending_signing().await.map(|opt| opt.is_some()).unwrap_or(false) },
        Duration::from_secs(5),
    )
    .await?;

    // Approve
    wallet
        .report_signing_result(SignResponse {
            id: "typed-data-test".to_string(),
            signature: Some(Bytes::from(vec![0xaa, 0xbb, 0xcc, 0xdd])),
            error: None,
        })
        .await?;

    handle.await??;

    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_signing_rejection() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

    // Connect wallet
    wallet.connect(test_address, 31337).await?;

    // Create signing request
    let request = create_test_signing_request("reject-test", "Reject me!");

    // Submit request
    let wallet_server = wallet.server.clone();
    let handle = tokio::spawn(async move { wallet_server.request_signing(request).await });

    // Wait for pending
    wait_for(
        || async { wallet.get_pending_signing().await.map(|opt| opt.is_some()).unwrap_or(false) },
        Duration::from_secs(5),
    )
    .await?;

    // Reject signing
    wallet
        .report_signing_result(SignResponse {
            id: "reject-test".to_string(),
            signature: None,
            error: Some("User rejected signing".to_string()),
        })
        .await?;

    // Verify rejection
    let result = handle.await?;
    assert!(result.is_err());

    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_concurrent_signing_requests() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

    // Connect wallet
    wallet.connect(test_address, 31337).await?;

    // Submit multiple signing requests
    let mut handles = vec![];
    for i in 0..5 {
        let id = format!("concurrent-sign-{i}");
        let message = format!("Message {i}");
        let request = SignRequest {
            id: id.clone(),
            address: address!("70997970C51812dc3A010C7d01b50e0d17dc79C8"),
            message,
            sign_type: SignType::PersonalSign,
        };

        let wallet_server = wallet.server.clone();
        let handle = tokio::spawn(async move { wallet_server.request_signing(request).await });
        handles.push((id, handle));
    }

    // Process each signing request
    for (id, _) in &handles {
        // Wait for pending request
        wait_for(
            || async {
                wallet.get_pending_signing().await.map(|opt| opt.is_some()).unwrap_or(false)
            },
            Duration::from_secs(5),
        )
        .await?;

        // Approve
        wallet
            .report_signing_result(SignResponse {
                id: id.clone(),
                signature: Some(Bytes::from(format!("sig-{id}").into_bytes())),
                error: None,
            })
            .await?;

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Verify all completed
    for (_, handle) in handles {
        handle.await??;
    }

    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_signing_with_disconnected_wallet() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;

    // Try to sign without connecting first
    let request = create_test_signing_request("no-connect", "Should fail");

    // This should be handled gracefully
    let _result = wallet.server.request_signing(request).await;

    // The behavior depends on implementation - might queue or reject

    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_signing_timeout() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

    // Connect wallet
    wallet.connect(test_address, 31337).await?;

    // Create signing request
    let request = create_test_signing_request("timeout-sign", "Timeout test");

    // Submit with timeout
    let wallet_server = wallet.server.clone();
    let handle = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(2), wallet_server.request_signing(request)).await
    });

    // Don't respond, let it timeout
    let result = handle.await?;
    assert!(result.is_err() || result.unwrap().is_err());

    wallet.shutdown().await?;
    Ok(())
}
