use std::sync::Arc;

use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use tower::ServiceBuilder;
use tower_http::{cors::CorsLayer, set_header::SetResponseHeaderLayer};

use crate::wallet_browser::{handlers, state::BrowserWalletState};

pub async fn build_router(state: Arc<BrowserWalletState>, port: u16) -> Router {
    let api = Router::new()
        .route("/transaction/request", get(handlers::get_next_transaction_request))
        .route("/transaction/response", post(handlers::post_transaction_response))
        .route("/signing/request", get(handlers::get_next_signing_request))
        .route("/signing/response", post(handlers::post_signing_response))
        .route("/connection", get(handlers::get_connection_info))
        .route("/connection", post(handlers::post_connection_update))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_session_token))
        .with_state(state.clone());

    let mut origins = vec![format!("http://127.0.0.1:{port}").parse().unwrap()];

    // Allow default port of 5173 in development mode.
    if state.is_development() {
        origins.push("https://localhost:5173".to_string().parse().unwrap());
    }

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
                .allow_origin(origins)
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

async fn require_session_token(
    State(state): State<Arc<BrowserWalletState>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if req.method() == Method::OPTIONS {
        return Ok(next.run(req).await);
    }

    // In development mode, skip session token check.
    if state.is_development() {
        return Ok(next.run(req).await);
    }

    let expected = state.session_token();
    let provided = req
        .headers()
        .get("X-Session-Token")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::FORBIDDEN)?;

    if provided != expected {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(next.run(req).await)
}
