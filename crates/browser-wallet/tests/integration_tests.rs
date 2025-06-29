mod integration;

#[tokio::test]
async fn test_server_can_start() {
    use foundry_browser_wallet::BrowserWalletServer;

    // Set test mode to skip browser opening
    std::env::set_var("BROWSER_WALLET_TEST_MODE", "1");

    let mut server = BrowserWalletServer::new(0); // Use random port

    // Start server
    server.start().await.expect("Failed to start server");

    // Verify port was assigned
    assert!(server.port() > 0);

    // Stop server
    server.stop().await.expect("Failed to stop server");
}
