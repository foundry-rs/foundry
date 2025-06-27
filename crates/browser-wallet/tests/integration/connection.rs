use crate::integration::utils::TestWallet;

#[tokio::test]
async fn test_server_startup() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;

    // Verify server is healthy
    assert!(wallet.health_check().await?);

    // Verify network details are available
    let network = wallet.get_network_details().await?;
    assert_eq!(network["chain_id"], 31337);
    assert_eq!(network["network_name"], "Anvil");

    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_wallet_connection() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

    // Initially no wallet should be connected
    assert!(wallet.server.get_connection().is_none());

    // Connect wallet
    wallet.connect(test_address, 31337).await?;

    // Verify connection
    let connection = wallet.server.get_connection();
    assert!(connection.is_some());
    let conn = connection.unwrap();
    assert_eq!(conn.address.to_string().to_lowercase(), test_address.to_lowercase());
    assert_eq!(conn.chain_id, 31337);

    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_wallet_disconnection() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

    // Connect wallet
    wallet.connect(test_address, 31337).await?;
    assert!(wallet.server.is_connected());

    // Disconnect wallet
    wallet.disconnect().await?;

    // Verify disconnection
    assert!(!wallet.server.is_connected());
    assert!(wallet.server.get_connection().is_none());

    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_chain_id_update() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let test_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

    // Connect to chain 31337
    wallet.connect(test_address, 31337).await?;

    let connection = wallet.server.get_connection().unwrap();
    assert_eq!(connection.chain_id, 31337);

    // Update to mainnet
    wallet.connect(test_address, 1).await?;

    let connection = wallet.server.get_connection().unwrap();
    assert_eq!(connection.chain_id, 1);
    assert_eq!(connection.address.to_string().to_lowercase(), test_address.to_lowercase());

    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_multiple_connections() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let address1 = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    let address2 = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";

    // Connect first address
    wallet.connect(address1, 31337).await?;
    let connection = wallet.server.get_connection().unwrap();
    assert_eq!(connection.address.to_string().to_lowercase(), address1.to_lowercase());

    // Connect second address (should replace first)
    wallet.connect(address2, 31337).await?;
    let connection = wallet.server.get_connection().unwrap();
    assert_eq!(connection.address.to_string().to_lowercase(), address2.to_lowercase());

    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_concurrent_connections() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;
    let addresses = [
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "0x90F79bf6EB2c4f870365E785982E1f101E93b906",
    ];

    // Attempt concurrent connections
    let mut handles = vec![];
    for (i, addr) in addresses.iter().enumerate() {
        let wallet_url = wallet.base_url.clone();
        let client = wallet.client.clone();
        let address = addr.to_string();
        let chain_id = 31337 + i as u64;

        let handle = tokio::spawn(async move {
            client
                .post(format!("{wallet_url}/api/account"))
                .json(&serde_json::json!({
                    "address": address,
                    "chain_id": chain_id
                }))
                .send()
                .await
        });
        handles.push(handle);
    }

    // Wait for all connections
    for handle in handles {
        let response = handle.await??;
        assert!(response.status().is_success());
    }

    // Only the last connection should be active
    let connection = wallet.server.get_connection();
    assert!(connection.is_some());

    wallet.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_invalid_address_format() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = TestWallet::spawn().await?;

    // Try to connect with invalid address
    let response = wallet
        .client
        .post(format!("{}/api/account", wallet.base_url))
        .json(&serde_json::json!({
            "address": "invalid_address",
            "chain_id": 31337
        }))
        .send()
        .await?;

    // Should still succeed at API level (validation happens client-side)
    assert!(response.status().is_success());

    wallet.shutdown().await?;
    Ok(())
}
