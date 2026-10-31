//! Implementations of the HTTP cheatcodes, in the [`Filesystem`](spec::Group::Filesystem) group.

use crate::{Cheatcode, CheatcodesCore, Result, Vm::*};
use alloy_primitives::U256;
use alloy_sol_types::SolValue;
use reqwest::{Method, Url, header::HeaderMap};
use std::time::Duration;

/// How long a request may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The largest response body accepted, in bytes.
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

impl Cheatcode for httpRequestCall {
    fn apply_context(&self, state: &mut CheatcodesCore) -> Result {
        let Self { request } = self;
        send(state, &request.method, &request.url, &request.headers, &request.body)
    }
}

impl Cheatcode for httpGetCall {
    fn apply_context(&self, state: &mut CheatcodesCore) -> Result {
        let Self { url } = self;
        send(state, "GET", url, &[], &[])
    }
}

fn send(
    state: &CheatcodesCore,
    method: &str,
    url: &str,
    headers: &[String],
    body: &[u8],
) -> Result {
    let parsed = Url::parse(url).map_err(|e| fmt_err!("invalid URL: {e}"))?;
    // The path and query may hold credentials, so errors only name the origin.
    let origin = parsed.origin().ascii_serialization();
    if !is_allowed(&state.config.http_allowlist, &parsed) {
        bail!("{origin} is not allowed; add the URL to `http_allowlist` in foundry.toml");
    }
    let method = Method::from_bytes(method.to_ascii_uppercase().as_bytes())
        .map_err(|_| fmt_err!("invalid HTTP method `{method}`"))?;
    let mut header_map = HeaderMap::new();
    for (i, header) in headers.iter().enumerate() {
        // Errors name the header by index: a malformed header may hold a credential.
        let (name, value) =
            header.split_once(':').ok_or_else(|| fmt_err!("header {i} must be `Name: value`"))?;
        let name = reqwest::header::HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| fmt_err!("header {i} has an invalid name"))?;
        // The host comes from the allowed URL only.
        if name == reqwest::header::HOST {
            bail!("the `Host` header cannot be set");
        }
        let value = reqwest::header::HeaderValue::from_str(value.trim())
            .map_err(|_| fmt_err!("header {i} has an invalid value"))?;
        header_map.append(name, value);
    }

    let allowlist = state.config.http_allowlist.clone();
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        // Follow redirects only to allowed URLs of the same origin, as the request headers, which
        // may hold credentials, are sent again.
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            let same_origin = attempt
                .previous()
                .first()
                .is_some_and(|first| first.origin() == attempt.url().origin());
            if attempt.previous().len() > 10 {
                attempt.error("too many redirects")
            } else if same_origin && is_allowed(&allowlist, attempt.url()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|e| fmt_err!("failed to build the HTTP client: {e}"))?;
    let request = client.request(method, parsed).headers(header_map).body(body.to_vec());

    let response = foundry_common::block_on(async {
        let response = request.send().await?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| format!("{name}: {}", String::from_utf8_lossy(value.as_bytes())))
            .collect::<Vec<_>>();
        if response.content_length().is_some_and(|len| len > MAX_BODY_SIZE as u64) {
            return Ok(Err(status));
        }
        // Read in chunks, so a response without a length cannot exceed the limit in memory.
        let mut response = response;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > MAX_BODY_SIZE {
                return Ok(Err(status));
            }
            body.extend_from_slice(&chunk);
        }
        Ok::<_, reqwest::Error>(Ok(HttpResponse {
            status: U256::from(status),
            headers,
            body: body.into(),
        }))
    })
    .map_err(|e| {
        let kind = if e.is_timeout() { "timed out" } else { "failed" };
        fmt_err!("request to {origin} {kind}: {}", e.without_url())
    })?
    .map_err(|_| fmt_err!("response from {origin} is larger than {MAX_BODY_SIZE} bytes"))?;
    Ok(response.abi_encode())
}

/// Whether `url` matches an entry of `allowlist`: same scheme, host and port, and a path under
/// the entry's path. An entry whose host starts with `*.` matches the subdomains of the rest of
/// the host. Entries with a query or fragment match nothing.
fn is_allowed(allowlist: &[String], url: &Url) -> bool {
    allowlist.iter().any(|entry| {
        let Ok(allowed) = Url::parse(&entry.replacen("://*.", "://", 1)) else { return false };
        if allowed.query().is_some() || allowed.fragment().is_some() {
            return false;
        }
        let wildcard = entry.contains("://*.");
        let (Some(host), Some(allowed_host)) = (url.host_str(), allowed.host_str()) else {
            return false;
        };
        let host_matches = if wildcard {
            host.strip_suffix(allowed_host).is_some_and(|sub| sub.ends_with('.'))
        } else {
            host == allowed_host
        };
        url.scheme() == allowed.scheme()
            && host_matches
            && url.port_or_known_default() == allowed.port_or_known_default()
            && path_under(url.path(), allowed.path())
    })
}

/// Whether `path` is `prefix` or below it: `/v1` matches `/v1` and `/v1/x`, not `/v1beta`.
fn path_under(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|rest| prefix.ends_with('/') || rest.is_empty() || rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_matching() {
        let allowlist = [
            "https://api.example.com/v1/".to_string(),
            "https://*.chain.link".to_string(),
            "http://127.0.0.1:8080".to_string(),
        ];
        let allowed = |url: &str| is_allowed(&allowlist, &Url::parse(url).unwrap());

        assert!(allowed("https://api.example.com/v1/prices?id=1"));
        assert!(!allowed("https://api.example.com/v2/prices"));
        assert!(!allowed("http://api.example.com/v1/prices"));
        assert!(!allowed("https://api.example.com.evil.io/v1/"));
        assert!(allowed("https://docs.chain.link/ccip"));
        assert!(!allowed("https://chain.link/"));
        assert!(!allowed("https://evilchain.link/"));
        assert!(allowed("http://127.0.0.1:8080/anything"));
        assert!(!allowed("http://127.0.0.1:8081/"));
        assert!(!is_allowed(&[], &Url::parse("https://api.example.com/").unwrap()));

        let allowlist = ["https://api.example.com/v1".to_string(), "https://q.io/?k=a".to_string()];
        let allowed = |url: &str| is_allowed(&allowlist, &Url::parse(url).unwrap());
        assert!(allowed("https://api.example.com/v1"));
        assert!(allowed("https://api.example.com/v1/x"));
        assert!(!allowed("https://api.example.com/v1beta"));
        assert!(!allowed("https://api.example.com/v1-internal/x"));
        assert!(!allowed("https://q.io/?k=a"));
    }
}
