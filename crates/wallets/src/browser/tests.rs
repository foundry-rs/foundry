#[cfg(test)]
mod tests {
    use super::super::*;
    use super::super::communication::TransactionRequest;
    use alloy_signer::Signer;
    
    #[tokio::test]
    async fn test_server_creation() {
        let server = BrowserWalletServer::new(0);
        assert!(!server.is_connected());
        assert!(server.get_connection().is_none());
    }
    
    #[tokio::test]
    async fn test_server_lifecycle() {
        let mut server = BrowserWalletServer::new(0);
        
        // Note: In a real test environment, we'd mock the browser opening
        // For now, we just test that the server can start and stop
        
        // Start would normally open a browser, so we skip in tests
        // assert!(server.start().await.is_ok());
        
        assert!(server.stop().await.is_ok());
    }
    
    #[tokio::test]
    async fn test_transaction_request_format() {
        let tx = TransactionRequest {
            id: "test-1".to_string(),
            from: "0x1234567890123456789012345678901234567890".to_string(),
            to: Some("0x0987654321098765432109876543210987654321".to_string()),
            value: "1000000000000000000".to_string(),
            data: None,
            gas: Some("21000".to_string()),
            gas_price: Some("1000000000".to_string()),
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            nonce: None,
            chain_id: 1,
        };
        
        // Verify serialization works
        let json = serde_json::to_string(&tx).unwrap();
        assert!(json.contains("test-1"));
        assert!(json.contains("1000000000000000000"));
    }
    
    #[tokio::test]
    async fn test_wallet_connection() {
        let connection = WalletConnection {
            address: "0x1234567890123456789012345678901234567890".parse().unwrap(),
            chain_id: 1,
            wallet_name: Some("MetaMask".to_string()),
        };
        
        // Verify serialization
        let json = serde_json::to_string(&connection).unwrap();
        assert!(json.contains("0x1234567890123456789012345678901234567890"));
        assert!(json.contains("\"chain_id\":1"));
    }
    
    #[test]
    fn test_browser_signer_trait_signatures() {
        // Verify that the signer trait methods exist and have correct signatures
        fn _test_signer_methods(signer: &BrowserSigner) {
            let _addr = signer.address();
            let _chain = signer.chain_id();
        }
        
        // This test passes if it compiles
        assert!(true);
    }
}