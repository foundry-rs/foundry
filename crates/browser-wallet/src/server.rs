use crate::{
    state::BrowserWalletState, BrowserTransaction, BrowserWalletError, SignRequest, SignResponse,
    TransactionResponse, TypedDataRequest, WalletConnection,
};
use alloy_dyn_abi::TypedData;
use alloy_primitives::{Address, Bytes, B256};
use axum::{
    extract::State,
    http::HeaderMap,
    response::{Html, Json},
    routing::{get, post},
    Router,
};
use std::{net::SocketAddr, sync::Arc};
use tokio::sync::{oneshot, Mutex};
use tower::ServiceBuilder;
use tower_http::{cors::CorsLayer, set_header::SetResponseHeaderLayer};

/// Browser wallet HTTP server
#[derive(Debug, Clone)]
pub struct BrowserWalletServer {
    port: u16,
    state: Arc<BrowserWalletState>,
    shutdown_tx: Option<Arc<Mutex<Option<oneshot::Sender<()>>>>>,
}

impl BrowserWalletServer {
    /// Create a new browser wallet server
    pub fn new(port: u16) -> Self {
        Self { port, state: Arc::new(BrowserWalletState::new()), shutdown_tx: None }
    }

    /// Start the server and open browser
    pub async fn start(&mut self) -> Result<(), BrowserWalletError> {
        let router = self.create_router();

        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = tokio::net::TcpListener::bind(addr)
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

        // Open browser (skip in test mode)
        if std::env::var("BROWSER_WALLET_TEST_MODE").is_err() {
            self.open_browser()?;
        }

        Ok(())
    }

    /// Get the server port
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Stop the server
    pub async fn stop(&mut self) -> Result<(), BrowserWalletError> {
        if let Some(shutdown_arc) = self.shutdown_tx.take() {
            if let Some(tx) = shutdown_arc.lock().await.take() {
                let _ = tx.send(());
            }
        }
        Ok(())
    }

    /// Check if a wallet is connected
    pub fn is_connected(&self) -> bool {
        self.state.get_connected_address().is_some()
    }

    /// Get current wallet connection
    pub fn get_connection(&self) -> Option<WalletConnection> {
        self.state.get_connected_address().map(|address| {
            let chain_id = self.state.get_connected_chain_id().unwrap_or(31337); // Default to Anvil
            WalletConnection {
                address: address.parse().unwrap_or_default(),
                chain_id,
                wallet_name: None,
            }
        })
    }

    /// Request a transaction
    pub async fn request_transaction(
        &self,
        request: BrowserTransaction,
    ) -> Result<B256, BrowserWalletError> {
        // Check if wallet is connected
        if self.state.get_connected_address().is_none() {
            return Err(BrowserWalletError::NotConnected);
        }

        let tx_id = request.id.clone();

        // Add to request queue
        self.state.add_transaction_request(request);

        // Wait for response (with timeout)
        let timeout = if std::env::var("BROWSER_WALLET_TIMEOUT").is_ok() {
            std::time::Duration::from_secs(
                std::env::var("BROWSER_WALLET_TIMEOUT").unwrap_or_default().parse().unwrap_or(300),
            )
        } else {
            std::time::Duration::from_secs(300) // 5 minutes default
        };
        let start = std::time::Instant::now();

        loop {
            // Check for response
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

            // Check timeout
            if start.elapsed() > timeout {
                // Remove from queue
                self.state.remove_transaction_request(&tx_id);
                return Err(BrowserWalletError::Timeout { operation: "Transaction" });
            }

            // Sleep briefly
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Request a message signature
    pub async fn request_signing(&self, request: SignRequest) -> Result<Bytes, BrowserWalletError> {
        // Check if wallet is connected
        if self.state.get_connected_address().is_none() {
            return Err(BrowserWalletError::NotConnected);
        }

        let request_id = request.id.clone();

        // Add to request queue
        self.state.add_signing_request(request);

        // Wait for response (with timeout)
        let timeout = if std::env::var("BROWSER_WALLET_TIMEOUT").is_ok() {
            std::time::Duration::from_secs(
                std::env::var("BROWSER_WALLET_TIMEOUT").unwrap_or_default().parse().unwrap_or(300),
            )
        } else {
            std::time::Duration::from_secs(300) // 5 minutes default
        };
        let start = std::time::Instant::now();

        loop {
            // Check for response
            if let Some(response) = self.state.get_signing_response(&request_id) {
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

            // Check timeout
            if start.elapsed() > timeout {
                // Remove from queue
                self.state.remove_signing_request(&request_id);
                return Err(BrowserWalletError::Timeout { operation: "Signing" });
            }

            // Sleep briefly
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Request typed data signature
    pub async fn request_typed_data_signing(
        &self,
        address: Address,
        typed_data: TypedData,
    ) -> Result<Bytes, BrowserWalletError> {
        let request =
            TypedDataRequest { id: uuid::Uuid::new_v4().to_string(), address, typed_data };

        // For now, convert to regular sign request with JSON data
        let sign_request = SignRequest {
            id: request.id,
            address: request.address,
            message: serde_json::to_string(&request.typed_data)
                .map_err(|e| BrowserWalletError::ServerError(e.to_string()))?,
            sign_type: crate::SignType::SignTypedData,
        };

        self.request_signing(sign_request).await
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
            // API endpoints
            .route("/api/heartbeat", get(heartbeat))
            .route("/api/transaction/pending", get(get_pending_transaction))
            .route("/api/transaction/response", post(report_transaction_result))
            .route("/api/sign/pending", get(get_pending_signing))
            .route("/api/sign/response", post(report_signing_result))
            .route("/api/network", get(get_network_details))
            .route("/api/account", post(update_account_status))
            // Serve static files
            .route("/js/main.js", get(serve_main_js))
            .route("/js/wallet.js", get(serve_wallet_js))
            .route("/js/polling.js", get(serve_polling_js))
            .route("/js/utils.js", get(serve_utils_js))
            .route("/css/styles.css", get(serve_styles_css))
            .layer(
                ServiceBuilder::new()
                    // Security headers
                    .layer(SetResponseHeaderLayer::overriding(
                        axum::http::header::CONTENT_SECURITY_POLICY,
                        axum::http::HeaderValue::from_static(
                            "default-src 'self'; script-src 'self' 'unsafe-inline'; connect-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; frame-ancestors 'none';"
                        ),
                    ))
                    .layer(SetResponseHeaderLayer::overriding(
                        axum::http::header::X_FRAME_OPTIONS,
                        axum::http::HeaderValue::from_static("DENY"),
                    ))
                    .layer(SetResponseHeaderLayer::overriding(
                        axum::http::header::X_CONTENT_TYPE_OPTIONS,
                        axum::http::HeaderValue::from_static("nosniff"),
                    ))
                    .layer(SetResponseHeaderLayer::overriding(
                        axum::http::header::REFERRER_POLICY,
                        axum::http::HeaderValue::from_static("no-referrer"),
                    ))
                    // Restrictive CORS - only allow same-origin
                    .layer(
                        CorsLayer::new()
                            .allow_origin(["http://localhost:*".parse().unwrap()])
                            .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
                            .allow_headers([axum::http::header::CONTENT_TYPE])
                            .allow_credentials(true),
                    ),
            )
            .with_state(Arc::clone(&self.state))
    }

    /// Open the browser
    fn open_browser(&self) -> Result<(), BrowserWalletError> {
        // Skip browser opening in test mode
        if std::env::var("BROWSER_WALLET_TEST_MODE").is_ok() {
            return Ok(());
        }

        let url = format!("http://localhost:{}", self.port);

        webbrowser::open(&url)
            .map_err(|e| BrowserWalletError::ServerError(format!("Failed to open browser: {e}")))
    }
}

// Handler functions

async fn serve_index() -> impl axum::response::IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    (headers, Html(crate::assets::web::INDEX_HTML))
}

async fn serve_main_js() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        crate::assets::web::js::MAIN_JS,
    )
}

async fn serve_wallet_js() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        crate::assets::web::js::WALLET_JS,
    )
}

async fn serve_polling_js() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        crate::assets::web::js::POLLING_JS,
    )
}

async fn serve_utils_js() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        crate::assets::web::js::UTILS_JS,
    )
}

async fn serve_styles_css() -> impl axum::response::IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "text/css")], crate::assets::web::css::STYLES_CSS)
}

async fn heartbeat(State(state): State<Arc<BrowserWalletState>>) -> Json<serde_json::Value> {
    state.update_heartbeat();
    Json(serde_json::json!({"status": "alive"}))
}

async fn get_pending_transaction(
    State(state): State<Arc<BrowserWalletState>>,
) -> Json<Option<BrowserTransaction>> {
    Json(state.get_pending_transaction())
}

async fn report_transaction_result(
    State(state): State<Arc<BrowserWalletState>>,
    Json(js_response): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    // Parse the JavaScript response format
    if let Some(id) = js_response.get("id").and_then(|v| v.as_str()) {
        let status = js_response.get("status").and_then(|v| v.as_str()).unwrap_or("");

        let response = if status == "success" {
            if let Some(hash_str) = js_response.get("hash").and_then(|v| v.as_str()) {
                // Try to parse the hash
                if let Ok(hash) = hash_str.parse::<B256>() {
                    TransactionResponse { id: id.to_string(), hash: Some(hash), error: None }
                } else {
                    TransactionResponse {
                        id: id.to_string(),
                        hash: None,
                        error: Some(format!("Invalid transaction hash: {hash_str}")),
                    }
                }
            } else {
                TransactionResponse {
                    id: id.to_string(),
                    hash: None,
                    error: Some("No transaction hash provided".to_string()),
                }
            }
        } else {
            TransactionResponse {
                id: id.to_string(),
                hash: None,
                error: js_response.get("error").and_then(|v| v.as_str()).map(String::from),
            }
        };

        state.add_transaction_response(response);
    }

    Json(serde_json::json!({"status": "ok"}))
}

async fn get_pending_signing(
    State(state): State<Arc<BrowserWalletState>>,
) -> Json<Option<SignRequest>> {
    Json(state.get_pending_signing())
}

async fn report_signing_result(
    State(state): State<Arc<BrowserWalletState>>,
    Json(js_response): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    // Parse the JavaScript response format
    if let Some(id) = js_response.get("id").and_then(|v| v.as_str()) {
        let status = js_response.get("status").and_then(|v| v.as_str()).unwrap_or("");

        let response = if status == "success" {
            if let Some(sig_str) = js_response.get("signature").and_then(|v| v.as_str()) {
                // Parse the signature as hex bytes
                if let Ok(sig_bytes) = hex::decode(sig_str.trim_start_matches("0x")) {
                    SignResponse {
                        id: id.to_string(),
                        signature: Some(sig_bytes.into()),
                        error: None,
                    }
                } else {
                    SignResponse {
                        id: id.to_string(),
                        signature: None,
                        error: Some(format!("Invalid signature format: {sig_str}")),
                    }
                }
            } else {
                SignResponse {
                    id: id.to_string(),
                    signature: None,
                    error: Some("No signature provided".to_string()),
                }
            }
        } else {
            SignResponse {
                id: id.to_string(),
                signature: None,
                error: js_response.get("error").and_then(|v| v.as_str()).map(String::from),
            }
        };

        state.add_signing_response(response);
    }

    Json(serde_json::json!({"status": "ok"}))
}

async fn get_network_details(
    State(state): State<Arc<BrowserWalletState>>,
) -> Json<serde_json::Value> {
    // Return static Anvil network details for now
    Json(serde_json::json!({
        "chain_id": state.get_connected_chain_id().unwrap_or(31337),
        "network_name": "Anvil",
        "rpc_url": "http://localhost:8545"
    }))
}

async fn update_account_status(
    State(state): State<Arc<BrowserWalletState>>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    if let Some(address) = body.get("address").and_then(|a| a.as_str()) {
        state.set_connected_address(Some(address.to_string()));

        // Update chain ID if provided
        if let Some(chain_id) = body.get("chain_id").and_then(|c| c.as_u64()) {
            state.set_connected_chain_id(Some(chain_id));
        }
    } else if body.get("address").is_some() {
        // Handle null address (disconnection)
        state.set_connected_address(None);
    }

    Json(serde_json::json!({"status": "ok"}))
}
