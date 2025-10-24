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
pub(crate) struct BrowserWalletServer {
    port: u16,
    state: Arc<BrowserWalletState>,
    shutdown_tx: Option<Arc<Mutex<Option<oneshot::Sender<()>>>>>,
    open_browser: bool,
}

impl BrowserWalletServer {
    /// Create a new browser wallet server.
    pub fn new(port: u16, open_browser: bool) -> Self {
        Self { port, state: Arc::new(BrowserWalletState::new()), shutdown_tx: None, open_browser }
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

    /// Check if a wallet is connected.
    pub fn is_connected(&self) -> bool {
        self.state.get_connected_address().is_some()
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

        let tx_id = request.id.clone();

        self.state.add_transaction_request(request);

        let timeout = Duration::from_secs(300);
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

            if start.elapsed() > timeout {
                self.state.remove_transaction_request(&tx_id);
                return Err(BrowserWalletError::Timeout { operation: "Transaction" });
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
