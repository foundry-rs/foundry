use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use alloy_dyn_abi::TypedData;
use alloy_primitives::{Address, Bytes, TxHash};
use tokio::{
    net::TcpListener,
    sync::{Mutex, oneshot},
};
use uuid::Uuid;

use crate::wallet_browser::{
    error::BrowserWalletError,
    router::build_router,
    state::BrowserWalletState,
    types::{
        BrowserSignRequest, BrowserSignTypedDataRequest, BrowserTransactionRequest, Connection,
        SignRequest, SignType,
    },
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
    pub fn new(port: u16, open_browser: bool, timeout: Duration, development: bool) -> Self {
        Self {
            port,
            state: Arc::new(BrowserWalletState::new(Uuid::new_v4().to_string(), development)),
            shutdown_tx: None,
            open_browser,
            timeout,
        }
    }

    /// Start the server and open browser.
    pub async fn start(&mut self) -> Result<(), BrowserWalletError> {
        let router = build_router(self.state.clone(), self.port).await;

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
            webbrowser::open(&format!("http://127.0.0.1:{}", self.port)).map_err(|e| {
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

    /// Check if the browser should be opened.
    pub fn open_browser(&self) -> bool {
        self.open_browser
    }

    /// Get the timeout duration.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Get the session token.
    pub fn session_token(&self) -> &str {
        self.state.session_token()
    }

    /// Check if a wallet is connected.
    pub async fn is_connected(&self) -> bool {
        self.state.is_connected().await
    }

    /// Get current wallet connection.
    pub async fn get_connection(&self) -> Option<Connection> {
        self.state.get_connection().await
    }

    /// Request a transaction to be signed and sent via the browser wallet.
    pub async fn request_transaction(
        &self,
        request: BrowserTransactionRequest,
    ) -> Result<TxHash, BrowserWalletError> {
        if !self.is_connected().await {
            return Err(BrowserWalletError::NotConnected);
        }

        let tx_id = request.id;

        self.state.add_transaction_request(request).await;

        let start = Instant::now();

        loop {
            if let Some(response) = self.state.get_transaction_response(&tx_id).await {
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
                self.state.remove_transaction_request(&tx_id).await;
                return Err(BrowserWalletError::Timeout { operation: "Transaction" });
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Request a message to be signed via the browser wallet.
    pub async fn request_signing(
        &self,
        request: BrowserSignRequest,
    ) -> Result<Bytes, BrowserWalletError> {
        if !self.is_connected().await {
            return Err(BrowserWalletError::NotConnected);
        }

        let tx_id = request.id;

        self.state.add_signing_request(request).await;

        let start = Instant::now();

        loop {
            if let Some(response) = self.state.get_signing_response(&tx_id).await {
                if let Some(signature) = response.signature {
                    return Ok(signature);
                } else if let Some(error) = response.error {
                    return Err(BrowserWalletError::Rejected {
                        operation: "Signing",
                        reason: error,
                    });
                } else {
                    return Err(BrowserWalletError::ServerError(
                        "Signing response missing both signature and error".to_string(),
                    ));
                }
            }

            if start.elapsed() > self.timeout {
                self.state.remove_signing_request(&tx_id).await;
                return Err(BrowserWalletError::Timeout { operation: "Signing" });
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Request EIP-712 typed data signing via the browser wallet.
    pub async fn request_typed_data_signing(
        &self,
        address: Address,
        typed_data: TypedData,
    ) -> Result<Bytes, BrowserWalletError> {
        let request = BrowserSignTypedDataRequest { id: Uuid::new_v4(), address, typed_data };

        let sign_request = BrowserSignRequest {
            id: request.id,
            sign_type: SignType::SignTypedDataV4,
            request: SignRequest {
                message: serde_json::to_string(&request.typed_data).map_err(|e| {
                    BrowserWalletError::ServerError(format!("Failed to serialize typed data: {e}"))
                })?,
                address: request.address,
            },
        };

        self.request_signing(sign_request).await
    }
}
