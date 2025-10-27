use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, HeaderValue, header::CONTENT_TYPE},
    response::Html,
};

use crate::wallet_browser::{
    app::contents,
    state::BrowserWalletState,
    types::{BrowserApiResponse, BrowserTransaction, Connection, TransactionResponse},
};

pub(crate) async fn serve_index() -> impl axum::response::IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    (headers, Html(contents::INDEX_HTML))
}

pub(crate) async fn get_next_transaction_request(
    State(state): State<Arc<BrowserWalletState>>,
) -> Json<BrowserApiResponse<BrowserTransaction>> {
    match state.read_next_transaction_request() {
        Some(tx) => Json(BrowserApiResponse::with_data(tx)),
        None => Json(BrowserApiResponse::error("No pending transaction")),
    }
}

pub(crate) async fn post_transaction_response(
    State(state): State<Arc<BrowserWalletState>>,
    Json(body): Json<TransactionResponse>,
) -> Json<BrowserApiResponse> {
    // Ensure that the transaction request exists.
    if !state.has_transaction_request(&body.id) {
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

    state.add_transaction_response(body);

    Json(BrowserApiResponse::ok())
}

pub(crate) async fn post_connection_update(
    State(state): State<Arc<BrowserWalletState>>,
    Json(body): Json<Option<Connection>>,
) -> Json<BrowserApiResponse> {
    // Update the connection state, setting it to None if disconnected.
    state.set_connection(body);

    Json(BrowserApiResponse::ok())
}
