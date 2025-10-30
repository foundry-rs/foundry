use std::sync::Arc;

use axum::{
    Router,
    http::{HeaderValue, Method, header},
    routing::{get, post},
};
use tower::ServiceBuilder;
use tower_http::{cors::CorsLayer, set_header::SetResponseHeaderLayer};

use crate::wallet_browser::{handlers, state::BrowserWalletState};

pub async fn build_router(state: Arc<BrowserWalletState>, port: u16) -> Router {
    let api = Router::new()
        .route("/transaction/request", get(handlers::get_next_transaction_request))
        .route("/transaction/response", post(handlers::post_transaction_response))
        .route("/connection", get(handlers::get_connection_info))
        .route("/connection", post(handlers::post_connection_update))
        .with_state(state.clone());

    let security_headers = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(concat!(
                "default-src 'none'; ",
                "object-src 'none'; ",
                "base-uri 'none'; ",
                "frame-ancestors 'none'; ",
                "img-src 'self'; ",
                "font-src 'none'; ",
                "connect-src 'self' https: http: wss: ws:;",
                "style-src 'self'; ",
                "script-src 'self'; ",
                "form-action 'none'; ",
                "worker-src 'none'; ",
                "frame-src https://id.porto.sh;"
            )),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(
            CorsLayer::new()
                .allow_origin([
                    format!("http://127.0.0.1:{port}").parse().unwrap(),
                    // TODO(zerosnacks): Remove this in production.
                    "https://localhost:5173".to_string().parse().unwrap(),
                ])
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_headers([header::CONTENT_TYPE])
                .allow_credentials(false),
        );

    Router::new()
        .route("/", get(handlers::serve_index))
        .route("/styles.css", get(handlers::serve_css))
        .route("/main.js", get(handlers::serve_js))
        .route("/banner.png", get(handlers::serve_banner_png))
        .route("/logo.png", get(handlers::serve_logo_png))
        .nest("/api", api)
        .layer(security_headers)
        .with_state(state)
}
