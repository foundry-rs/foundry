use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use alloy_primitives::TxHash;
use axum::{
    Router,
    routing::{get, post},
};
use tokio::{
    net::TcpListener,
    sync::{Mutex, oneshot},
};

use crate::wallet_browser::{
    error::BrowserWalletError,
    handlers,
    state::BrowserWalletState,
    types::{BrowserTransaction, WalletConnection},
};

/// Browser wallet server.
#[derive(Debug, Clone)]
pub struct BrowserWalletServer {
    port: u16,
    state: Arc<BrowserWalletState>,
    shutdown_tx: Option<Arc<Mutex<Option<oneshot::Sender<()>>>>>,
    open_browser: bool,
    timeout: Duration,
}

impl BrowserWalletServer {
    /// Create a new browser wallet server.
    pub fn new(port: u16, open_browser: bool, timeout: Duration) -> Self {
        Self {
            port,
            state: Arc::new(BrowserWalletState::new()),
            shutdown_tx: None,
            open_browser,
            timeout,
        }
    }

    /// Start the server and open browser.
    pub async fn start(&mut self) -> Result<(), BrowserWalletError> {
        let router = Router::new()
            // Serve browser wallet application
            .route("/", get(handlers::serve_index))
            // API endpoints
            .route("/api/transaction/pending", get(handlers::get_pending_transaction))
            .route("/api/transaction/response", post(handlers::post_transaction_response))
            .route("/api/account", post(handlers::post_account_update))
            .with_state(Arc::clone(&self.state));

        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| BrowserWalletError::ServerError(e.to_string()))?;
        self.port = listener.local_addr().unwrap().port();

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        self.shutdown_tx = Some(Arc::new(Mutex::new(Some(shutdown_tx))));

        tokio::spawn(async move {
            let server = axum::serve(listener, router);
            let _ = server
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });

        if self.open_browser {
            webbrowser::open(&format!("http://localhost:{}", self.port)).map_err(|e| {
                BrowserWalletError::ServerError(format!("Failed to open browser: {e}"))
            })?;
        }

        Ok(())
    }

    /// Stop the server.
    pub async fn stop(&mut self) -> Result<(), BrowserWalletError> {
        if let Some(shutdown_arc) = self.shutdown_tx.take()
            && let Some(tx) = shutdown_arc.lock().await.take()
        {
            let _ = tx.send(());
        }
        Ok(())
    }

    /// Get the server port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Check if a wallet is connected.
    pub fn is_connected(&self) -> bool {
        self.state.is_connected()
    }

    /// Get current wallet connection.
    pub fn get_connection(&self) -> Option<WalletConnection> {
        self.state.get_connected_address().map(|address| {
            let chain_id = self.state.get_connected_chain_id().unwrap_or(31337);

            WalletConnection { address, chain_id }
        })
    }

    /// Request a transaction to be signed and sent via the browser wallet.
    pub async fn request_transaction(
        &self,
        request: BrowserTransaction,
    ) -> Result<TxHash, BrowserWalletError> {
        if !self.is_connected() {
            return Err(BrowserWalletError::NotConnected);
        }

        let tx_id = request.id;

        self.state.add_transaction_request(request);

        let start = Instant::now();

        loop {
            if let Some(response) = self.state.get_transaction_response(&tx_id) {
                if let Some(hash) = response.hash {
                    return Ok(hash);
                } else if let Some(error) = response.error {
                    return Err(BrowserWalletError::Rejected {
                        operation: "Transaction",
                        reason: error,
                    });
                } else {
                    return Err(BrowserWalletError::ServerError(
                        "Transaction response missing both hash and error".to_string(),
                    ));
                }
            }

            if start.elapsed() > self.timeout {
                self.state.remove_transaction_request(&tx_id);
                return Err(BrowserWalletError::Timeout { operation: "Transaction" });
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::wallet_browser::types::AccountUpdate;

    use super::*;

    use alloy_primitives::{Address, TxKind, U256, address};
    use alloy_rpc_types::TransactionRequest;
    use uuid::Uuid;

    const ALICE: Address = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    const BOB: Address = address!("0x70997970C51812dc3A010C7d01b50e0d17dc79C8");

    #[tokio::test]
    async fn test_connect_disconnect_wallet() {
        let client = reqwest::Client::new();
        let mut server = BrowserWalletServer::new(0, false, Duration::from_secs(5));

        // Check initial disconnected state
        assert!(!server.is_connected());

        // Start server
        server.start().await.unwrap();

        // Check pending transaction
        let resp =
            reqwest::get(&format!("http://localhost:{}/api/transaction/pending", server.port()))
                .await;
        assert!(resp.is_ok());
        let resp_json: Option<BrowserTransaction> = resp.unwrap().json().await.unwrap();
        assert!(resp_json.is_none());

        // Connect Alice's wallet
        let resp = client
            .post(format!("http://localhost:{}/api/account", server.port()))
            .json(&AccountUpdate { address: Some(ALICE), chain_id: Some(1) })
            .send()
            .await;
        assert!(resp.is_ok());

        // Check connection state
        let connection = server.get_connection();
        assert!(connection.is_some());
        let connection = connection.unwrap();
        assert_eq!(connection.address, ALICE);
        assert_eq!(connection.chain_id, 1);

        // Disconnect wallet
        let resp = client
            .post(format!("http://localhost:{}/api/account", server.port()))
            .json(&AccountUpdate { address: None, chain_id: None })
            .send()
            .await;
        assert!(resp.is_ok());

        // Check disconnected state
        assert!(!server.is_connected());

        // Connect Bob's wallet
        let resp = client
            .post(format!("http://localhost:{}/api/account", server.port()))
            .json(&AccountUpdate { address: Some(BOB), chain_id: Some(42) })
            .send()
            .await;
        assert!(resp.is_ok());

        // Check connection state
        let connection = server.get_connection();
        assert!(connection.is_some());
        let connection = connection.unwrap();
        assert_eq!(connection.address, BOB);
        assert_eq!(connection.chain_id, 42);

        // Stop server
        server.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_send_transaction() {
        let client = reqwest::Client::new();
        let mut server = BrowserWalletServer::new(0, false, Duration::from_secs(1));
        server.start().await.unwrap();

        // Connect Alice's wallet
        let resp = client
            .post(format!("http://localhost:{}/api/account", server.port()))
            .json(&AccountUpdate { address: Some(ALICE), chain_id: Some(1) })
            .send()
            .await;
        assert!(resp.is_ok());

        // Create a browser transaction request
        let tx_request_id = Uuid::new_v4();
        let tx_request = BrowserTransaction {
            id: tx_request_id,
            request: TransactionRequest {
                from: Some(ALICE),
                to: Some(TxKind::Call(BOB)),
                value: Some(U256::from(1000)),
                ..Default::default()
            },
        };

        // Spawn the signing flow in the background
        let browser_server = server.clone();
        let join_handle =
            tokio::spawn(async move { browser_server.request_transaction(tx_request).await });
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Check pending transaction
        let resp =
            reqwest::get(&format!("http://localhost:{}/api/transaction/pending", server.port()))
                .await
                .unwrap();
        let resp_json: Option<BrowserTransaction> = resp.json().await.unwrap();
        assert!(resp_json.is_some());
        let pending_tx = resp_json.unwrap();
        assert_eq!(pending_tx.id, tx_request_id);
        assert_eq!(pending_tx.request.from, Some(ALICE));
        assert_eq!(pending_tx.request.to, Some(TxKind::Call(BOB)));
        assert_eq!(pending_tx.request.value, Some(U256::from(1000)));

        // Simulate the wallet rejecting the tx
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&serde_json::json!({
                "id": tx_request_id,
                "hash": null,
                "error": "User rejected the transaction",
            }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        // The join handle should now return a rejection error
        let res = join_handle.await.expect("task panicked");
        match res {
            Err(BrowserWalletError::Rejected { operation, reason }) => {
                assert_eq!(operation, "Transaction");
                assert_eq!(reason, "User rejected the transaction");
            }
            other => panic!("expected rejection, got {other:?}"),
        }
    }
}
