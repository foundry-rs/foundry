//! MPP (Machine Payments Protocol) HTTP transport.
//!
//! Wraps a standard reqwest HTTP transport with automatic 402 Payment Required
//! handling via the MPP protocol. When the RPC endpoint returns a 402 response,
//! this transport automatically pays the challenge and retries the request.

use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_transport::{TransportError, TransportErrorKind, TransportFut, TransportResult};
use mpp::client::tempo::TempoProvider;
use mpp::client::Fetch;
use std::task;
use tower::Service;
use tracing::{debug, debug_span, trace, Instrument};
use url::Url;

/// HTTP transport that automatically handles MPP 402 challenges.
///
/// When an RPC endpoint is 402-gated, this transport:
/// 1. Sends the initial JSON-RPC request
/// 2. If 402 is returned, parses the challenge from `WWW-Authenticate`
/// 3. Pays via the configured `TempoProvider`
/// 4. Retries the request with the payment credential
#[derive(Clone)]
pub struct MppHttpTransport {
    client: reqwest::Client,
    url: Url,
    provider: TempoProvider,
}

impl std::fmt::Debug for MppHttpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MppHttpTransport")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl MppHttpTransport {
    /// Create a new MPP HTTP transport.
    pub fn new(client: reqwest::Client, url: Url, provider: TempoProvider) -> Self {
        Self { client, url, provider }
    }

    /// Send a JSON-RPC request with automatic 402 payment handling.
    async fn do_request(self, req: RequestPacket) -> TransportResult<ResponsePacket> {
        let resp = self
            .client
            .post(self.url.clone())
            .json(&req)
            .headers(req.headers())
            .send_with_payment(&self.provider)
            .await
            .map_err(TransportErrorKind::custom)?;

        let status = resp.status();
        debug!(%status, "received response from MPP transport");

        let body = resp.bytes().await.map_err(TransportErrorKind::custom)?;

        if tracing::enabled!(tracing::Level::TRACE) {
            trace!(body = %String::from_utf8_lossy(&body), "response body");
        } else {
            debug!(bytes = body.len(), "retrieved response body");
        }

        if !status.is_success() {
            return Err(TransportErrorKind::http_error(
                status.as_u16(),
                String::from_utf8_lossy(&body).into_owned(),
            ));
        }

        serde_json::from_slice(&body)
            .map_err(|err| TransportError::deser_err(err, String::from_utf8_lossy(&body)))
    }
}

impl Service<RequestPacket> for MppHttpTransport {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    #[inline]
    fn poll_ready(
        &mut self,
        _cx: &mut task::Context<'_>,
    ) -> task::Poll<Result<(), Self::Error>> {
        task::Poll::Ready(Ok(()))
    }

    #[inline]
    fn call(&mut self, req: RequestPacket) -> Self::Future {
        let this = self.clone();
        let span = debug_span!("MppHttpTransport", url = %this.url);
        Box::pin(this.do_request(req).instrument(span.or_current()))
    }
}
