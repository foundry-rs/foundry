use std::sync::Arc;

use axum::{Json, extract::State, http::HeaderMap, response::Html};
use serde_json::{Value, json};

use crate::wallet_browser::{
    app::contents,
    state::BrowserWalletState,
    types::{AccountUpdate, BrowserTransaction, TransactionResponse},
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
) -> Json<Option<BrowserTransaction>> {
    Json(state.get_pending_transaction())
}

pub(crate) async fn post_transaction_response(
    State(state): State<Arc<BrowserWalletState>>,
    Json(body): Json<TransactionResponse>,
) -> Json<Value> {
    state.add_transaction_response(body);

    Json(json!({ "status": "ok" }))
}

pub(crate) async fn post_account_update(
    State(state): State<Arc<BrowserWalletState>>,
    Json(body): Json<AccountUpdate>,
) -> Json<Value> {
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

    Json(json!({ "status": "ok" }))
}
