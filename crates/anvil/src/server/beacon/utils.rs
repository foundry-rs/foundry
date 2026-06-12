use hyper::HeaderMap;

/// Helper function to determine if the Accept header indicates a preference for SSZ (octet-stream)
/// over JSON.
pub fn must_be_ssz(headers: &HeaderMap) -> bool {
    headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|accept_str| {
            let mut octet_stream_q = 0.0;
            let mut json_q = 0.0;

            // Parse each media type in the Accept header
            for media_type in accept_str.split(',') {
                let media_type = media_type.trim();
                let quality = media_type
                    .split(';')
                    .find_map(|param| {
                        let param = param.trim();
                        if let Some(q) = param.strip_prefix("q=") {
                            q.parse::<f32>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(1.0); // Default quality factor is 1.0

                if media_type.starts_with("application/octet-stream") {
                    octet_stream_q = quality;
                } else if media_type.starts_with("application/json") {
                    json_q = quality;
                }
            }

            // Prefer octet-stream if it has higher quality factor
            octet_stream_q > json_q
        })
        .unwrap_or(false)
}
