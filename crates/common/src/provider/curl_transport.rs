//! Transport that outputs equivalent curl commands instead of making RPC requests.

use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_transport::{TransportError, TransportFut};
use tower::Service;
use url::Url;

/// Escapes a string for use in a single-quoted shell argument.
fn shell_escape(s: &str) -> String {
    s.replace('\'', "'\"'\"'")
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
    pub fn new(url: Url) -> Self {
        Self { url, headers: vec![], jwt: None }
    }

    /// Set the headers for the transport.
    pub fn with_headers(mut self, headers: Vec<String>) -> Self {
        self.headers = headers;
        self
    }

    /// Set the JWT for the transport.
    pub fn with_jwt(mut self, jwt: Option<String>) -> Self {
        self.jwt = jwt;
        self
    }

    /// Generate a curl command for a request.
    fn generate_curl_command(&self, req: &RequestPacket) -> String {
        let payload_str = serde_json::to_string(req).unwrap_or_default();
        let escaped_payload = shell_escape(&payload_str);

        let mut cmd = String::from("curl -X POST");
        cmd.push_str(" -H 'Content-Type: application/json'");

        if let Some(jwt) = &self.jwt {
            cmd.push_str(&format!(" -H 'Authorization: Bearer {}'", shell_escape(jwt)));
        }

        for h in &self.headers {
            cmd.push_str(&format!(" -H '{}'", shell_escape(h)));
        }

        cmd.push_str(&format!(" --data-raw '{escaped_payload}'"));
        cmd.push_str(&format!(" '{}'", shell_escape(self.url.as_str())));

        cmd
    }

    /// Handle a request by printing the curl command.
    pub fn request(&self, req: RequestPacket) -> TransportFut<'static> {
        let curl_cmd = self.generate_curl_command(&req);

        Box::pin(async move {
            // Print the curl command to stdout
            let _ = crate::sh_println!("{curl_cmd}");

            // Exit cleanly after printing the curl command
            std::process::exit(0);
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
        let cmd = transport.generate_curl_command(&req);
        assert!(cmd.contains("eth_blockNumber"));
        assert!(cmd.contains("https://eth.example.com"));
        assert!(cmd.contains("jsonrpc"));
    }

    #[test]
    fn test_curl_with_headers() {
        let transport = CurlTransport::new("https://eth.example.com".parse().unwrap())
            .with_headers(vec!["X-Custom: value".to_string()]);
        let req = make_test_request();
        let cmd = transport.generate_curl_command(&req);
        assert!(cmd.contains("X-Custom: value"));
    }

    #[test]
    fn test_curl_with_jwt() {
        let transport = CurlTransport::new("https://eth.example.com".parse().unwrap())
            .with_jwt(Some("my-jwt-token".to_string()));
        let req = make_test_request();
        let cmd = transport.generate_curl_command(&req);
        assert!(cmd.contains("Authorization: Bearer my-jwt-token"));
    }

    #[test]
    fn test_shell_escape() {
        let escaped = shell_escape("it's a test");
        assert_eq!(escaped, "it'\"'\"'s a test");
    }
}
