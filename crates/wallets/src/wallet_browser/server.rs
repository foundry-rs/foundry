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
    types::{BrowserTransaction, Connection},
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
            .route("/api/transaction/request", get(handlers::get_next_transaction_request))
            .route("/api/transaction/response", post(handlers::post_transaction_response))
            .route("/api/connection", post(handlers::post_connection_update))
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
    pub fn get_connection(&self) -> Option<Connection> {
        self.state.get_connection()
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
    use super::*;

    use alloy_primitives::{Address, TxKind, U256, address};
    use alloy_rpc_types::TransactionRequest;
    use tokio::task::JoinHandle;
    use uuid::Uuid;

    use crate::wallet_browser::types::{BrowserApiResponse, TransactionResponse};

    const ALICE: Address = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    const BOB: Address = address!("0x70997970C51812dc3A010C7d01b50e0d17dc79C8");

    #[tokio::test]
    async fn test_setup_server() {
        let mut server = BrowserWalletServer::new(0, false, Duration::from_secs(5));

        // Check initial state
        assert!(!server.is_connected());
        assert!(!server.open_browser);
        assert!(server.timeout == Duration::from_secs(5));

        // Start server
        server.start().await.unwrap();

        // Check that the transaction request queue is empty
        check_transaction_request_queue_empty(&server).await;

        // Stop server
        server.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_connect_disconnect_wallet() {
        let client = reqwest::Client::new();
        let mut server = BrowserWalletServer::new(0, false, Duration::from_secs(5));
        server.start().await.unwrap();

        // Check that the transaction request queue is empty
        check_transaction_request_queue_empty(&server).await;

        // Connect Alice's wallet
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Check connection state
        let Connection(address, chain_id) =
            server.get_connection().expect("expected an active wallet connection");
        assert_eq!(address, ALICE);
        assert_eq!(chain_id, 1);

        // Disconnect wallet
        disconnect_wallet(&client, &server).await;

        // Check disconnected state
        assert!(!server.is_connected());

        // Connect Bob's wallet
        connect_wallet(&client, &server, Connection::new(BOB, 42)).await;

        // Check connection state
        let Connection(address, chain_id) =
            server.get_connection().expect("expected an active wallet connection");
        assert_eq!(address, BOB);
        assert_eq!(chain_id, 42);

        // Stop server
        server.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_send_transaction_client_accept() {
        let client = reqwest::Client::new();
        let mut server = BrowserWalletServer::new(0, false, Duration::from_secs(1));
        server.start().await.unwrap();

        // Connect Alice's wallet
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Create a browser transaction request
        let (tx_request_id, tx_request) = create_browser_transaction();

        // Spawn the signing flow in the background
        let handle = wait_for_signing(&server, tx_request).await;

        // Check transaction request
        check_transaction_request_content(&server, tx_request_id).await;

        // Simulate the wallet accepting and signing the tx
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&TransactionResponse {
                id: tx_request_id,
                hash: Some(TxHash::random()),
                error: None,
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        // The join handle should now return the tx hash
        let res = handle.await.expect("task panicked");
        match res {
            Ok(hash) => {
                assert!(hash != TxHash::new([0; 32]));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_send_transaction_client_not_requested() {
        let client = reqwest::Client::new();
        let mut server = BrowserWalletServer::new(0, false, Duration::from_secs(1));
        server.start().await.unwrap();

        // Connect Alice's wallet
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Create a random transaction response without a matching request
        let tx_request_id = Uuid::new_v4();

        // Simulate the wallet sending a response for an unknown request
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&TransactionResponse {
                id: tx_request_id,
                hash: Some(TxHash::random()),
                error: None,
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        // Assert that no transaction without a matching request is accepted
        let api: BrowserApiResponse<()> = resp.json().await.unwrap();
        match api {
            BrowserApiResponse::Error { message } => {
                assert_eq!(message, "Unknown transaction id");
            }
            _ => panic!("expected error response"),
        }
    }

    #[tokio::test]
    async fn test_send_transaction_invalid_response_format() {
        // non uuid

        let client = reqwest::Client::new();

        let mut server = BrowserWalletServer::new(0, false, Duration::from_secs(1));
        server.start().await.unwrap();

        // Connect Alice's wallet
        connect_wallet(&client, &server, Connection::new(ALICE, 1)).await;

        // Simulate the wallet sending a response with an invalid UUID
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .body(
                r#"{
                "id": "invalid-uuid",
                "hash": "invalid-hash",
                "error": null
            }"#,
            )
            .header("Content-Type", "application/json")
            .send()
            .await
            .unwrap();

        // The server should respond with a 422 Unprocessable Entity status
        assert_eq!(resp.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn test_send_transaction_client_reject() {
        let client = reqwest::Client::new();
        let mut server = BrowserWalletServer::new(0, false, Duration::from_secs(1));
        server.start().await.unwrap();

        // Connect Alice's wallet
        connect_wallet(&client, &server, Connection(ALICE, 1)).await;

        // Create a browser transaction request
        let (tx_request_id, tx_request) = create_browser_transaction();

        // Spawn the signing flow in the background
        let handle = wait_for_signing(&server, tx_request).await;

        // Check transaction request
        check_transaction_request_content(&server, tx_request_id).await;

        // Simulate the wallet rejecting the tx
        let resp = client
            .post(format!("http://localhost:{}/api/transaction/response", server.port()))
            .json(&TransactionResponse {
                id: tx_request_id,
                hash: None,
                error: Some("User rejected the transaction".into()),
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        // The join handle should now return a rejection error
        let res = handle.await.expect("task panicked");
        match res {
            Err(BrowserWalletError::Rejected { operation, reason }) => {
                assert_eq!(operation, "Transaction");
                assert_eq!(reason, "User rejected the transaction");
            }
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    /// Helper to connect a wallet to the server.
    async fn connect_wallet(
        client: &reqwest::Client,
        server: &BrowserWalletServer,
        connection: Connection,
    ) {
        let resp = client
            .post(format!("http://localhost:{}/api/connection", server.port()))
            .json(&connection)
            .send();
        assert!(resp.await.is_ok());
    }

    /// Helper to disconnect a wallet from the server.
    async fn disconnect_wallet(client: &reqwest::Client, server: &BrowserWalletServer) {
        let resp = client
            .post(format!("http://localhost:{}/api/connection", server.port()))
            .json(&Option::<Connection>::None)
            .send();
        assert!(resp.await.is_ok());
    }

    /// Spawn the signing flow in the background and return the join handle.
    async fn wait_for_signing(
        server: &BrowserWalletServer,
        tx_request: BrowserTransaction,
    ) -> JoinHandle<Result<TxHash, BrowserWalletError>> {
        // Spawn the signing flow in the background
        let browser_server = server.clone();
        let join_handle =
            tokio::spawn(async move { browser_server.request_transaction(tx_request).await });
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        join_handle
    }

    /// Create a simple browser transaction request.
    fn create_browser_transaction() -> (Uuid, BrowserTransaction) {
        let id = Uuid::new_v4();
        let tx = BrowserTransaction {
            id,
            request: TransactionRequest {
                from: Some(ALICE),
                to: Some(TxKind::Call(BOB)),
                value: Some(U256::from(1000)),
                ..Default::default()
            },
        };
        (id, tx)
    }

    /// Check that the transaction request queue is empty, if not panic.
    async fn check_transaction_request_queue_empty(server: &BrowserWalletServer) {
        let url = format!("http://localhost:{}/api/transaction/request", server.port());
        let resp = reqwest::get(&url).await.unwrap();

        let BrowserApiResponse::Error { message } =
            resp.json::<BrowserApiResponse<BrowserTransaction>>().await.unwrap()
        else {
            panic!("expected BrowserApiResponse::Error (no pending transaction), but got Ok");
        };

        assert_eq!(message, "No pending transaction");
    }

    /// Check that the transaction request matches the expected request ID and fields.
    async fn check_transaction_request_content(server: &BrowserWalletServer, tx_request_id: Uuid) {
        let url = format!("http://localhost:{}/api/transaction/request", server.port());
        let resp = reqwest::get(&url).await.unwrap();

        let BrowserApiResponse::Ok(pending_tx) =
            resp.json::<BrowserApiResponse<BrowserTransaction>>().await.unwrap()
        else {
            panic!("expected BrowserApiResponse::Ok with a pending transaction");
        };

        assert_eq!(pending_tx.id, tx_request_id);
        assert_eq!(pending_tx.request.from, Some(ALICE));
        assert_eq!(pending_tx.request.to, Some(TxKind::Call(BOB)));
        assert_eq!(pending_tx.request.value, Some(U256::from(1000)));
    }
}
