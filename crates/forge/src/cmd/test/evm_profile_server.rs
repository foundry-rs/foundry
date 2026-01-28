//! Local HTTP server for serving EVM profiles to Firefox Profiler.
//!
//! Firefox Profiler (profiler.firefox.com) can load profiles from a URL via its `/from-url/`
//! endpoint. This module implements a temporary local HTTP server that:
//! 1. Serves the profile JSON at `/{token}/profile.json`
//! 2. Sets CORS headers to allow profiler.firefox.com to fetch it
//! 3. Constructs the proper URL and opens it in the browser

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use eyre::Result;
use foundry_common::{sh_err, sh_println};
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};

/// Serves a Firefox Profiler profile on a local HTTP server and opens it in the browser.
///
/// Takes the already-serialized profile JSON bytes.
/// The server runs until Ctrl+C is pressed.
pub async fn serve_and_open(
    profile_json: Vec<u8>,
    test_name: &str,
    contract_name: &str,
) -> Result<()> {
    let token = generate_token();

    let state = ServerState { profile_json: Arc::new(profile_json) };

    let app = Router::new()
        .route(&format!("/{token}/profile.json"), get(serve_profile))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::OPTIONS])
                .allow_headers(Any),
        )
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    let profile_url = format!("http://127.0.0.1:{port}/{token}/profile.json");
    let encoded_profile_url = percent_encode(&profile_url);
    let profiler_url = format!("https://profiler.firefox.com/from-url/{encoded_profile_url}/");

    sh_println!("Profile server running at http://127.0.0.1:{port}")?;
    sh_println!("Opening Firefox Profiler for {contract_name}::{test_name}...")?;

    if let Err(e) = opener::open(&profiler_url) {
        sh_err!("Failed to open browser: {e}")?;
        sh_println!("Please open this URL manually:\n{profiler_url}")?;
    }

    sh_println!("\nPress Ctrl+C to stop the server.")?;

    // Run the server until interrupted.
    axum::serve(listener, app).await?;

    Ok(())
}

/// Generates a random token for the URL path (32 hex characters).
fn generate_token() -> String {
    use std::{
        hash::{DefaultHasher, Hasher},
        time::{SystemTime, UNIX_EPOCH},
    };

    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut hasher = DefaultHasher::new();
    hasher.write_u128(nanos);
    hasher.write_usize(std::process::id() as usize);
    let random_part = hasher.finish();
    format!("{nanos:016x}{random_part:016x}")
}

/// Percent-encode a URL for embedding in Firefox Profiler's `/from-url/` path.
fn percent_encode(url: &str) -> String {
    let mut result = String::with_capacity(url.len() * 3);
    for byte in url.bytes() {
        match byte {
            // Unreserved characters (RFC 3986)
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                result.push(byte as char);
            }
            // Everything else gets percent-encoded
            _ => {
                result.push('%');
                result.push_str(&format!("{byte:02X}"));
            }
        }
    }
    result
}

#[derive(Clone)]
struct ServerState {
    profile_json: Arc<Vec<u8>>,
}

async fn serve_profile(State(state): State<ServerState>) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Bytes::from(state.profile_json.as_ref().clone()),
    )
        .into_response()
}
