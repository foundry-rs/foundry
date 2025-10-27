use std::sync::Arc;

use axum::{Json, extract::State, http::HeaderMap, response::Html};

use crate::wallet_browser::{
    app::contents,
    state::BrowserWalletState,
    types::{AccountUpdate, BrowserApiResponse, BrowserTransaction, TransactionResponse},
};

pub(crate) async fn serve_index() -> impl axum::response::IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    (headers, Html(contents::INDEX_HTML))
}

pub(crate) async fn get_pending_transaction(
    State(state): State<Arc<BrowserWalletState>>,
) -> Json<BrowserApiResponse<BrowserTransaction>> {
    match state.get_pending_transaction() {
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

pub(crate) async fn post_account_update(
    State(state): State<Arc<BrowserWalletState>>,
    Json(body): Json<AccountUpdate>,
) -> Json<BrowserApiResponse> {
    match body.address {
        Some(addr) => {
            state.set_connected_address(Some(addr));

            if let Some(chain_id) = body.chain_id {
                state.set_connected_chain_id(Some(chain_id));
            }
        }
        None => {
            state.set_connected_address(None);
            state.set_connected_chain_id(None);
        }
    }

    Json(BrowserApiResponse::ok())
}
