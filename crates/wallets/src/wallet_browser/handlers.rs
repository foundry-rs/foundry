use axum::{http::HeaderMap, response::Html};

use crate::wallet_browser::app::contents;

pub async fn serve_index() -> impl axum::response::IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    (headers, Html(contents::INDEX_HTML))
}
