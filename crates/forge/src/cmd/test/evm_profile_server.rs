//! Local HTTP server for serving EVM profiles to browser-based viewers.
//!
//! Supports multiple profile formats:
//! - Speedscope (speedscope.app): Uses `#profileURL=` hash parameter
//! - Chrome/Perfetto (ui.perfetto.dev): Uses `url=` query parameter
//!
//! This module implements a temporary local HTTP server that:
//! 1. Serves the profile JSON at `/{token}/profile.json`
//! 2. Sets CORS headers to allow the viewer to fetch it
//! 3. Constructs the proper URL and opens it in the browser

use super::EvmProfileFormat;
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
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};

/// Serves a profile on a local HTTP server and opens it in the browser.
///
/// Takes the already-serialized profile JSON bytes.
/// The server runs until Ctrl+C is pressed.
pub async fn serve_and_open(
    profile_json: Vec<u8>,
    test_name: &str,
    contract_name: &str,
    format: EvmProfileFormat,
) -> Result<()> {
    let token = generate_token();

    let state = ServerState { profile_json: Bytes::from(profile_json) };

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
    let title = format!("{contract_name}::{test_name}");

    // Build viewer URL based on format.
    let (viewer_name, viewer_url) = match format {
        EvmProfileFormat::Speedscope => {
            let encoded_url = percent_encode(&profile_url);
            let encoded_title = percent_encode(&title);
            (
                "speedscope",
                format!(
                    "https://www.speedscope.app/#profileURL={encoded_url}&title={encoded_title}"
                ),
            )
        }
        EvmProfileFormat::Chrome => {
            let encoded_url = percent_encode(&profile_url);
            ("Perfetto", format!("https://ui.perfetto.dev/#!/?url={encoded_url}"))
        }
    };

    sh_println!("Profile server running at http://127.0.0.1:{port}")?;
    sh_println!("Opening {viewer_name} for {title}...")?;

    if let Err(e) = opener::open(&viewer_url) {
        sh_err!("Failed to open browser: {e}")?;
        sh_println!("Please open this URL manually:\n{viewer_url}")?;
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

/// Percent-encode a URL for embedding in viewer URL parameters.
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
    profile_json: Bytes,
}

async fn serve_profile(State(state): State<ServerState>) -> Response {
    (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], state.profile_json)
        .into_response()
}
