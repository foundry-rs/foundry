use alloy_primitives::{Address, U256};
use alloy_rpc_types::TransactionRequest;
use foundry_browser_wallet::{
    BrowserTransaction, BrowserWalletServer, SignRequest, SignResponse, SignType,
    TransactionResponse,
};
use reqwest::Client;
use serde_json::json;
use std::time::Duration;

/// Test wallet wrapper following Anvil's pattern
pub struct TestWallet {
    pub server: BrowserWalletServer,
    pub client: Client,
    pub base_url: String,
}

impl TestWallet {
    /// Spawn a new test wallet server
    pub async fn spawn() -> Result<Self, Box<dyn std::error::Error>> {
        // Set test environment variable to skip browser launch
        std::env::set_var("BROWSER_WALLET_TEST_MODE", "true");
        // Set shorter timeout for tests (5 seconds)
        std::env::set_var("BROWSER_WALLET_TIMEOUT", "5");

        let mut server = BrowserWalletServer::new(0); // Use port 0 for random assignment
        server.start().await?;

        let port = server.port();
        let base_url = format!("http://localhost:{}", port);

        // Wait for server to be ready
        let client = Client::builder().timeout(Duration::from_secs(10)).build()?;

        // Health check with retries
        for _ in 0..10 {
            if client.get(&format!("{}/api/heartbeat", base_url)).send().await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        Ok(Self { server, client, base_url })
    }

    /// Simulate wallet connection
    pub async fn connect(
        &self,
        address: &str,
        chain_id: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let response = self
            .client
            .post(&format!("{}/api/account", self.base_url))
            .json(&json!({
                "address": address,
                "chain_id": chain_id
            }))
            .send()
            .await?;

        assert!(response.status().is_success(), "Failed to connect wallet: {}", response.status());
        Ok(())
    }

    /// Disconnect wallet
    pub async fn disconnect(&self) -> Result<(), Box<dyn std::error::Error>> {
        let response = self
            .client
            .post(&format!("{}/api/account", self.base_url))
            .json(&json!({
                "address": serde_json::Value::Null,
                "chain_id": serde_json::Value::Null
            }))
            .send()
            .await?;

        assert!(response.status().is_success());
        Ok(())
    }

    /// Get pending transaction (simulating frontend polling)
    pub async fn get_pending_transaction(
        &self,
    ) -> Result<Option<BrowserTransaction>, Box<dyn std::error::Error>> {
        let response =
            self.client.get(&format!("{}/api/transaction/pending", self.base_url)).send().await?;

        if response.status().is_success() {
            let text = response.text().await?;
            if text.trim() == "null" || text.is_empty() {
                Ok(None)
            } else {
                Ok(Some(serde_json::from_str(&text)?))
            }
        } else {
            Ok(None)
        }
    }

    /// Report transaction result (simulating wallet response)
    pub async fn report_transaction_result(
        &self,
        response: TransactionResponse,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Convert to JavaScript format expected by server
        let js_response = if let Some(hash) = response.hash {
            json!({
                "id": response.id,
                "status": "success",
                "hash": format!("0x{}", hex::encode(hash))
            })
        } else {
            json!({
                "id": response.id,
                "status": "error",
                "error": response.error.unwrap_or_else(|| "Unknown error".to_string())
            })
        };

        let response = self
            .client
            .post(&format!("{}/api/transaction/response", self.base_url))
            .json(&js_response)
            .send()
            .await?;

        assert!(response.status().is_success());
        Ok(())
    }

    /// Get pending signing request
    pub async fn get_pending_signing(
        &self,
    ) -> Result<Option<SignRequest>, Box<dyn std::error::Error>> {
        let response =
            self.client.get(&format!("{}/api/sign/pending", self.base_url)).send().await?;

        if response.status().is_success() {
            let text = response.text().await?;
            if text.trim() == "null" || text.is_empty() {
                Ok(None)
            } else {
                Ok(Some(serde_json::from_str(&text)?))
            }
        } else {
            Ok(None)
        }
    }

    /// Report signing result
    pub async fn report_signing_result(
        &self,
        response: SignResponse,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Convert to JavaScript format expected by server
        let js_response = if let Some(signature) = response.signature {
            json!({
                "id": response.id,
                "status": "success",
                "signature": format!("0x{}", hex::encode(&signature))
            })
        } else {
            json!({
                "id": response.id,
                "status": "error",
                "error": response.error.unwrap_or_else(|| "Unknown error".to_string())
            })
        };

        let response = self
            .client
            .post(&format!("{}/api/sign/response", self.base_url))
            .json(&js_response)
            .send()
            .await?;

        assert!(response.status().is_success());
        Ok(())
    }

    /// Check server health
    pub async fn health_check(&self) -> Result<bool, Box<dyn std::error::Error>> {
        let response = self.client.get(&format!("{}/api/heartbeat", self.base_url)).send().await?;

        Ok(response.status().is_success())
    }

    /// Get network details
    pub async fn get_network_details(
        &self,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let response = self.client.get(&format!("{}/api/network", self.base_url)).send().await?;

        Ok(response.json().await?)
    }

    /// Shutdown the server
    pub async fn shutdown(mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.server.stop().await?;
        Ok(())
    }
}

/// Create a test transaction
pub fn create_test_transaction(
    id: &str,
    from: Address,
    to: Address,
    value: U256,
) -> BrowserTransaction {
    BrowserTransaction {
        id: id.to_string(),
        request: TransactionRequest {
            from: Some(from),
            to: Some(to.into()),
            value: Some(value),
            chain_id: Some(31337), // Anvil chain ID
            ..Default::default()
        },
    }
}

/// Create a test signing request
pub fn create_test_signing_request(id: &str, message: &str) -> SignRequest {
    SignRequest {
        id: id.to_string(),
        address: Address::ZERO,
        message: message.to_string(),
        sign_type: SignType::PersonalSign,
    }
}

/// Helper to wait for a condition with timeout
pub async fn wait_for<F, Fut>(
    mut condition: F,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = tokio::time::Instant::now();

    loop {
        if condition().await {
            return Ok(());
        }

        if start.elapsed() > timeout {
            return Err("Timeout waiting for condition".into());
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Simulate frontend polling for transactions
pub async fn simulate_transaction_polling(
    wallet: &TestWallet,
    auto_approve: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Poll for pending transaction
    if let Some(tx) = wallet.get_pending_transaction().await? {
        // Simulate user action
        if auto_approve {
            wallet
                .report_transaction_result(TransactionResponse {
                    id: tx.id,
                    hash: Some(alloy_primitives::B256::random()),
                    error: None,
                })
                .await?;
        } else {
            wallet
                .report_transaction_result(TransactionResponse {
                    id: tx.id,
                    hash: None,
                    error: Some("User rejected transaction".to_string()),
                })
                .await?;
        }
    }
    Ok(())
}

/// Simulate frontend polling for signing requests
pub async fn simulate_signing_polling(
    wallet: &TestWallet,
    auto_approve: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Poll for pending signing request
    if let Some(req) = wallet.get_pending_signing().await? {
        // Simulate user action
        if auto_approve {
            wallet
                .report_signing_result(SignResponse {
                    id: req.id,
                    signature: Some(alloy_primitives::Bytes::from(vec![0xde, 0xad, 0xbe, 0xef])),
                    error: None,
                })
                .await?;
        } else {
            wallet
                .report_signing_result(SignResponse {
                    id: req.id,
                    signature: None,
                    error: Some("User rejected signature request".to_string()),
                })
                .await?;
        }
    }
    Ok(())
}
