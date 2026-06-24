//! Transport that outputs equivalent curl commands instead of making RPC requests.

use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_rpc_types_engine::{Claims, JwtSecret};
use alloy_transport::{TransportError, TransportFut};
use eyre::Context;
use serde_json::Value;
use tower::Service;
use url::Url;

/// Escapes a string for use in a single-quoted shell argument.
fn shell_escape(s: &str) -> String {
    s.replace('\'', "'\"'\"'")
}

/// Build a JWT using a secret
fn build_jwt(jwt_secret: &str) -> eyre::Result<String> {
    // Decode jwt from hex, then generate claims (iat with current timestamp)
    let secret = JwtSecret::from_hex(jwt_secret)?;
    let claims = Claims::default();
    let token = secret.encode(&claims)?;
    Ok(token)
}

/// Appends a JWT Authorization header to a curl command.
fn append_jwt_auth(cmd: &mut String, jwt_secret: &str) -> eyre::Result<()> {
    let jwt = build_jwt(jwt_secret).wrap_err("Invalid --jwt-secret provided")?;

    cmd.push_str(&format!(" -H 'Authorization: Bearer {}'", shell_escape(jwt.as_str())));

    Ok(())
}

/// Generates a curl command for an RPC request.
///
/// This is a standalone helper that can be used to generate curl commands
/// without going through the transport layer.
pub fn generate_curl_command(
    url: &str,
    method: &str,
    params: Value,
    headers: Option<&[String]>,
    jwt_secret: Option<&str>,
) -> eyre::Result<String> {
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    });
    let payload_str = serde_json::to_string(&payload).unwrap_or_default();
    let escaped_payload = shell_escape(&payload_str);

    let mut cmd = String::from("curl -X POST");
    cmd.push_str(" -H 'Content-Type: application/json'");

    if let Some(secret) = jwt_secret {
        append_jwt_auth(&mut cmd, secret)?;
    }

    if let Some(hdrs) = headers {
        for h in hdrs {
            cmd.push_str(&format!(" -H '{}'", shell_escape(h)));
        }
    }

    cmd.push_str(&format!(" --data-raw '{escaped_payload}'"));
    cmd.push_str(&format!(" '{}'", shell_escape(url)));

    Ok(cmd)
}

/// A transport that prints curl commands instead of executing RPC requests.
///
/// When a request is made through this transport, it will print the equivalent
/// curl command to stderr and return a dummy successful response.
#[derive(Clone, Debug)]
pub struct CurlTransport {
    /// The URL to connect to.
    url: Url,
    /// The headers to use for requests.
    headers: Vec<String>,
    /// The JWT to use for requests.
    jwt: Option<String>,
}

impl CurlTransport {
    /// Create a new curl transport with the given URL.
    pub const fn new(url: Url) -> Self {
        Self { url, headers: vec![], jwt: None }
    }

    /// Set the headers for the transport.
    pub fn with_headers(mut self, headers: Vec<String>) -> Self {
        self.headers = headers;
        self
    }

    /// Set the JWT Secret for the transport.
    pub fn with_jwt(mut self, jwt: Option<String>) -> Self {
        self.jwt = jwt;
        self
    }

    /// Generate a curl command for a request.
    fn generate_curl_command(&self, req: &RequestPacket) -> eyre::Result<String> {
        let payload_str = serde_json::to_string(req).unwrap_or_default();
        let escaped_payload = shell_escape(&payload_str);

        let mut cmd = String::from("curl -X POST");
        cmd.push_str(" -H 'Content-Type: application/json'");

        if let Some(jwt_secret) = &self.jwt {
            append_jwt_auth(&mut cmd, jwt_secret)?;
        }

        for h in &self.headers {
            cmd.push_str(&format!(" -H '{}'", shell_escape(h)));
        }

        cmd.push_str(&format!(" --data-raw '{escaped_payload}'"));
        cmd.push_str(&format!(" '{}'", shell_escape(self.url.as_str())));

        Ok(cmd)
    }

    /// Handle a request by printing the curl command.
    pub fn request(&self, req: RequestPacket) -> TransportFut<'static> {
        let curl_cmd_result = self.generate_curl_command(&req);

        Box::pin(async move {
            match curl_cmd_result {
                Ok(curl_cmd) => {
                    let _ = crate::sh_println!("{curl_cmd}");
                    std::process::exit(0);
                }
                Err(e) => {
                    let _ = crate::sh_eprintln!("Error: {e:?}");
                    std::process::exit(1);
                }
            }
        })
    }
}

impl Service<RequestPacket> for CurlTransport {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    #[inline]
    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    #[inline]
    fn call(&mut self, req: RequestPacket) -> Self::Future {
        self.request(req)
    }
}

impl Service<RequestPacket> for &CurlTransport {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    #[inline]
    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    #[inline]
    fn call(&mut self, req: RequestPacket) -> Self::Future {
        self.request(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_json_rpc::{Id, Request};

    fn make_test_request() -> RequestPacket {
        let req: Request<Vec<()>> = Request::new("eth_blockNumber", Id::Number(1), vec![]);
        let serialized = req.serialize().unwrap();
        RequestPacket::Single(serialized)
    }

    #[test]
    fn test_basic_curl_command() {
        let transport = CurlTransport::new("https://eth.example.com".parse().unwrap());
        let req = make_test_request();
        let cmd = transport.generate_curl_command(&req).unwrap();
        assert!(cmd.contains("eth_blockNumber"));
        assert!(cmd.contains("https://eth.example.com"));
        assert!(cmd.contains("jsonrpc"));
    }

    #[test]
    fn test_curl_with_headers() {
        let transport = CurlTransport::new("https://eth.example.com".parse().unwrap())
            .with_headers(vec!["X-Custom: value".to_string()]);
        let req = make_test_request();
        let cmd = transport.generate_curl_command(&req).unwrap();
        assert!(cmd.contains("X-Custom: value"));
    }

    #[test]
    fn test_curl_with_jwt() {
        let jwt_secret = "5c43996d0d150a81f06ae452fce38120d97a4156650aec7487b3384bfe32edae";
        let transport = CurlTransport::new("https://eth.example.com".parse().unwrap())
            .with_jwt(Some(jwt_secret.to_string()));
        let req = make_test_request();
        let cmd = transport.generate_curl_command(&req).unwrap();

        let jwt = cmd
            .split("Authorization: Bearer ")
            .nth(1)
            .expect("missing Authorization header")
            .split('\'')
            .next()
            .expect("malformed Authorization header");

        let secret = JwtSecret::from_hex(jwt_secret).unwrap();
        secret.validate(jwt).unwrap();
    }

    #[test]
    fn test_shell_escape() {
        let escaped = shell_escape("it's a test");
        assert_eq!(escaped, "it'\"'\"'s a test");
    }
}
