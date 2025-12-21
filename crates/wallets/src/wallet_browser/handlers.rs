use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{
        HeaderMap, HeaderValue,
        header::{CACHE_CONTROL, CONTENT_TYPE, EXPIRES, PRAGMA},
    },
    response::Html,
};

use crate::wallet_browser::{
    app::contents,
    state::BrowserWalletState,
    types::{
        BrowserApiResponse, BrowserSignRequest, BrowserSignResponse, BrowserTransactionRequest,
        BrowserTransactionResponse, Connection,
    },
};

/// Serve index.html
pub(crate) async fn serve_index() -> impl axum::response::IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("no-store, no-cache, must-revalidate, max-age=0"),
    );
    headers.insert(PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(EXPIRES, HeaderValue::from_static("0"));
    (headers, Html(contents::INDEX_HTML))
}

/// Serve styles.css
pub(crate) async fn serve_css() -> impl axum::response::IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/css; charset=utf-8"));
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("no-store, no-cache, must-revalidate, max-age=0"),
    );
    headers.insert(PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(EXPIRES, HeaderValue::from_static("0"));
    (headers, contents::STYLES_CSS)
}

/// Serve main.js with injected session token.
pub(crate) async fn serve_js(
    State(state): State<Arc<BrowserWalletState>>,
) -> impl axum::response::IntoResponse {
    let token = state.session_token();
    let js = format!("window.__SESSION_TOKEN__ = \"{}\";\n{}", token, contents::MAIN_JS);

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/javascript; charset=utf-8"));
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("no-store, no-cache, must-revalidate, max-age=0"),
    );
    headers.insert(PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(EXPIRES, HeaderValue::from_static("0"));
    (headers, js)
}

/// Serve banner.png
pub(crate) async fn serve_banner_png() -> impl axum::response::IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("image/png"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
    (headers, contents::BANNER_PNG)
}

/// Serve logo.png
pub(crate) async fn serve_logo_png() -> impl axum::response::IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("image/png"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
    (headers, contents::LOGO_PNG)
}

/// Get the next pending transaction request.
/// Route: GET /api/transaction/request
pub(crate) async fn get_next_transaction_request(
    State(state): State<Arc<BrowserWalletState>>,
) -> Json<BrowserApiResponse<BrowserTransactionRequest>> {
    match state.read_next_transaction_request().await {
        Some(tx) => Json(BrowserApiResponse::with_data(tx)),
        None => Json(BrowserApiResponse::error("No pending transaction request")),
    }
}

/// Post a transaction response (signed or error).
/// Route: POST /api/transaction/response
pub(crate) async fn post_transaction_response(
    State(state): State<Arc<BrowserWalletState>>,
    Json(body): Json<BrowserTransactionResponse>,
) -> Json<BrowserApiResponse> {
    // Ensure that the transaction request exists.
    if !state.has_transaction_request(&body.id).await {
        return Json(BrowserApiResponse::error("Unknown transaction id"));
    }

    // Ensure that exactly one of hash or error is provided.
    match (&body.hash, &body.error) {
        (None, None) => {
            return Json(BrowserApiResponse::error("Either hash or error must be provided"));
        }
        (Some(_), Some(_)) => {
            return Json(BrowserApiResponse::error("Only one of hash or error can be provided"));
        }
        _ => {}
    }

    // Validate transaction hash if provided.
    if let Some(hash) = &body.hash {
        // Check for all-zero hash
        if hash.is_zero() {
            return Json(BrowserApiResponse::error("Invalid (zero) transaction hash"));
        }

        // Sanity check: ensure the hash is exactly 32 bytes
        if hash.as_slice().len() != 32 {
            return Json(BrowserApiResponse::error(
                "Malformed transaction hash (expected 32 bytes)",
            ));
        }
    }

    state.add_transaction_response(body).await;

    Json(BrowserApiResponse::ok())
}

/// Get the next pending signing request.
/// Route: GET /api/signing/request
pub(crate) async fn get_next_signing_request(
    State(state): State<Arc<BrowserWalletState>>,
) -> Json<BrowserApiResponse<BrowserSignRequest>> {
    match state.read_next_signing_request().await {
        Some(req) => Json(BrowserApiResponse::with_data(req)),
        None => Json(BrowserApiResponse::error("No pending signing request")),
    }
}

/// Post a signing response (signature or error).
/// Route: POST /api/signing/response
pub(crate) async fn post_signing_response(
    State(state): State<Arc<BrowserWalletState>>,
    Json(body): Json<BrowserSignResponse>,
) -> Json<BrowserApiResponse> {
    // Ensure that the signing request exists.
    if !state.has_signing_request(&body.id).await {
        return Json(BrowserApiResponse::error("Unknown signing request id"));
    }

    // Ensure that exactly one of signature or error is provided.
    match (&body.signature, &body.error) {
        (None, None) => {
            return Json(BrowserApiResponse::error("Either signature or error must be provided"));
        }
        (Some(_), Some(_)) => {
            return Json(BrowserApiResponse::error(
                "Only one of signature or error can be provided",
            ));
        }
        _ => {}
    }

    state.add_signing_response(body).await;

    Json(BrowserApiResponse::ok())
}

/// Get current connection information.
/// Route: GET /api/connection
pub(crate) async fn get_connection_info(
    State(state): State<Arc<BrowserWalletState>>,
) -> Json<BrowserApiResponse<Option<Connection>>> {
    let connection = state.get_connection().await;

    Json(BrowserApiResponse::with_data(connection))
}

/// Post connection update (connect or disconnect).
/// Route: POST /api/connection
pub(crate) async fn post_connection_update(
    State(state): State<Arc<BrowserWalletState>>,
    Json(body): Json<Option<Connection>>,
) -> Json<BrowserApiResponse> {
    state.set_connection(body).await;

    Json(BrowserApiResponse::ok())
}
