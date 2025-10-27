use std::sync::Arc;

use axum::{
    Router,
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};

use crate::wallet_browser::{handlers, state::BrowserWalletState};

pub async fn build_router(state: Arc<BrowserWalletState>) -> Router {
    let api = Router::new()
        .route("/transaction/request", get(handlers::get_next_transaction_request))
        .route("/transaction/response", post(handlers::post_transaction_response))
        .route("/connection", get(handlers::get_connection_info))
        .route("/connection", post(handlers::post_connection_update))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_session_token))
        .with_state(state.clone());

    Router::new().route("/", get(handlers::serve_index)).nest("/api", api).with_state(state)
}

async fn require_session_token(
    State(state): State<Arc<BrowserWalletState>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let expected = state.session_token(); // Arc<String>
    let ok = req
        .headers()
        .get("X-Session-Token")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == expected.as_str())
        .unwrap_or(false);

    if !ok {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(next.run(req).await)
}
