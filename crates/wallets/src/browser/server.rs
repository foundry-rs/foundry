use super::{
    communication::{NetworkDetails, SigningRequest, SigningResponse, TransactionRequest, TransactionResponse},
    state::BrowserWalletState,
    BrowserWalletError,
};
use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, Json},
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::oneshot;

/// Browser wallet HTTP server
#[derive(Debug)]
pub struct BrowserWalletServer {
    port: u16,
    state: Arc<BrowserWalletState>,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl BrowserWalletServer {
    /// Create a new browser wallet server
    pub fn new(port: u16) -> Self {
        Self {
            port,
            state: Arc::new(BrowserWalletState::new()),
            shutdown_tx: None,
        }
    }

    /// Start the server and open browser
    pub async fn start(&mut self) -> Result<(), BrowserWalletError> {
        // Set up network details (this would come from the actual network config)
        {
            let mut network = self.state.communication.network_details.lock();
            network.chain_id = 31337; // Default to Anvil
            network.network_name = "Anvil".to_string();
            network.rpc_url = "http://localhost:8545".to_string();
        }

        let router = self.create_router();
        
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| BrowserWalletError::ServerError(e.to_string()))?;
        
        self.port = listener.local_addr().unwrap().port();
        
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);
        
        tokio::spawn(async move {
            let server = axum::serve(listener, router);
            let _ = server
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
        
        // Open browser
        self.open_browser()?;
        
        Ok(())
    }

    /// Stop the server
    pub async fn stop(&mut self) -> Result<(), BrowserWalletError> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        Ok(())
    }

    /// Check if a wallet is connected
    pub fn is_connected(&self) -> bool {
        self.state.get_connected_address().is_some()
    }

    /// Get current wallet connection
    pub fn get_connection(&self) -> Option<super::WalletConnection> {
        self.state.get_connected_address().map(|address| {
            let network = self.state.communication.network_details.lock();
            super::WalletConnection {
                address: address.parse().unwrap_or_default(),
                chain_id: network.chain_id,
                wallet_name: None,
            }
        })
    }

    /// Request a transaction
    pub async fn request_transaction(
        &self,
        request: TransactionRequest,
    ) -> Result<String, BrowserWalletError> {
        let tx_id = request.id.clone();
        
        // Add to request queue
        self.state.add_transaction_request(request);
        
        // Wait for response (with timeout)
        let timeout = std::time::Duration::from_secs(300); // 5 minutes
        let start = std::time::Instant::now();
        
        loop {
            // Check for response
            if let Some(response) = self.state.get_transaction_response(&tx_id) {
                if response.status == "success" {
                    if let Some(hash) = response.hash {
                        return Ok(hash);
                    } else {
                        return Err(BrowserWalletError::InvalidResponse(
                            "Transaction succeeded but no hash provided".to_string()
                        ));
                    }
                } else {
                    return Err(BrowserWalletError::TransactionRejected(
                        response.error.unwrap_or_else(|| "Unknown error".to_string())
                    ));
                }
            }
            
            // Check timeout
            if start.elapsed() > timeout {
                // Remove from queue
                self.state.remove_transaction_request(&tx_id);
                return Err(BrowserWalletError::TransactionTimeout);
            }
            
            // Sleep briefly
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    
    /// Request a message signature
    pub async fn request_signing(
        &self,
        request: super::communication::SigningRequest,
    ) -> Result<String, BrowserWalletError> {
        let request_id = request.id.clone();
        
        // Add to request queue
        self.state.add_signing_request(request);
        
        // Wait for response (with timeout)
        let timeout = std::time::Duration::from_secs(300); // 5 minutes
        let start = std::time::Instant::now();
        
        loop {
            // Check for response
            if let Some(response) = self.state.get_signing_response(&request_id) {
                if response.status == "success" {
                    if let Some(signature) = response.signature {
                        return Ok(signature);
                    } else {
                        return Err(BrowserWalletError::InvalidResponse(
                            "Signing succeeded but no signature provided".to_string()
                        ));
                    }
                } else {
                    return Err(BrowserWalletError::SigningRejected(
                        response.error.unwrap_or_else(|| "Unknown error".to_string())
                    ));
                }
            }
            
            // Check timeout
            if start.elapsed() > timeout {
                // Remove from queue
                self.state.remove_signing_request(&request_id);
                return Err(BrowserWalletError::SigningTimeout);
            }
            
            // Sleep briefly
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Get the state for sharing with the signer
    pub fn state(&self) -> Arc<BrowserWalletState> {
        Arc::clone(&self.state)
    }

    /// Create the Axum router
    fn create_router(&self) -> Router {
        Router::new()
            // Serve the main page
            .route("/", get(serve_index))
            // API endpoints matching Moccasin
            .route("/heartbeat", get(heartbeat))
            .route("/get_pending_transaction", get(get_pending_transaction))
            .route("/report_transaction_result", post(report_transaction_result))
            .route("/get_pending_signing", get(get_pending_signing))
            .route("/report_signing_result", post(report_signing_result))
            .route("/get_boa_network_details", get(get_network_details))
            .route("/update_account_status", post(update_account_status))
            .route("/shutdown", post(shutdown))
            // Serve static files
            .route("/js/main.js", get(serve_main_js))
            .route("/js/wallet.js", get(serve_wallet_js))
            .route("/js/polling.js", get(serve_polling_js))
            .route("/js/utils.js", get(serve_utils_js))
            .route("/css/styles.css", get(serve_styles_css))
            .with_state(Arc::clone(&self.state))
    }

    /// Open the browser
    fn open_browser(&self) -> Result<(), BrowserWalletError> {
        let url = format!("http://localhost:{}", self.port);
        
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .arg(&url)
                .spawn()
                .map_err(|e| BrowserWalletError::ServerError(e.to_string()))?;
        }
        
        #[cfg(target_os = "linux")]
        {
            std::process::Command::new("xdg-open")
                .arg(&url)
                .spawn()
                .map_err(|e| BrowserWalletError::ServerError(e.to_string()))?;
        }
        
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("cmd")
                .args(["/C", "start", "", &url])
                .spawn()
                .map_err(|e| BrowserWalletError::ServerError(e.to_string()))?;
        }
        
        Ok(())
    }
}

// Handler functions

async fn serve_index() -> Html<&'static str> {
    Html(super::assets::web::INDEX_HTML)
}

async fn serve_main_js() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        super::assets::web::js::MAIN_JS,
    )
}

async fn serve_wallet_js() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        super::assets::web::js::WALLET_JS,
    )
}

async fn serve_polling_js() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        super::assets::web::js::POLLING_JS,
    )
}

async fn serve_utils_js() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        super::assets::web::js::UTILS_JS,
    )
}

async fn serve_styles_css() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css")],
        super::assets::web::css::STYLES_CSS,
    )
}

async fn heartbeat() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "alive"}))
}

async fn get_pending_transaction(State(state): State<Arc<BrowserWalletState>>) -> Json<serde_json::Value> {
    if let Some(tx) = state.get_pending_transaction() {
        Json(serde_json::to_value(tx).unwrap())
    } else {
        Json(serde_json::json!(null))
    }
}

async fn report_transaction_result(
    State(state): State<Arc<BrowserWalletState>>,
    Json(response): Json<TransactionResponse>,
) -> Json<serde_json::Value> {
    state.add_transaction_response(response);
    Json(serde_json::json!({"status": "ok"}))
}

async fn get_pending_signing(State(state): State<Arc<BrowserWalletState>>) -> Json<Option<SigningRequest>> {
    Json(state.get_pending_signing())
}

async fn report_signing_result(
    State(state): State<Arc<BrowserWalletState>>,
    Json(response): Json<SigningResponse>,
) -> Json<serde_json::Value> {
    state.add_signing_response(response);
    Json(serde_json::json!({"status": "ok"}))
}

async fn get_network_details(State(state): State<Arc<BrowserWalletState>>) -> Json<NetworkDetails> {
    let network = state.communication.network_details.lock();
    Json(network.clone())
}

async fn update_account_status(
    State(state): State<Arc<BrowserWalletState>>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    if let Some(address) = body.get("address").and_then(|a| a.as_str()) {
        state.set_connected_address(Some(address.to_string()));
    }
    Json(serde_json::json!({"status": "ok"}))
}

async fn shutdown() -> StatusCode {
    // In a real implementation, this would trigger server shutdown
    StatusCode::OK
}