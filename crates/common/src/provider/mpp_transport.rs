//! MPP (Machine Payments Protocol) HTTP transport.
//!
//! Wraps a standard reqwest HTTP transport with automatic 402 Payment Required
//! handling via the MPP protocol. When the RPC endpoint returns a 402 response,
//! this transport automatically pays the challenge and retries the request.

use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_transport::{TransportError, TransportErrorKind, TransportFut, TransportResult};
use mpp::{
    client::PaymentProvider,
    protocol::core::{
        AUTHORIZATION_HEADER, WWW_AUTHENTICATE_HEADER, format_authorization, parse_www_authenticate,
    },
};
use reqwest::StatusCode;
use std::task;
use tower::Service;
use tracing::{Instrument, debug, debug_span, trace};
use url::Url;

/// HTTP transport that automatically handles MPP 402 challenges.
///
/// When an RPC endpoint is 402-gated, this transport:
/// 1. Sends the initial JSON-RPC request
/// 2. If 402 is returned, parses the challenge from `WWW-Authenticate`
/// 3. Pays via the configured payment provider
/// 4. Retries the request with the payment credential
#[derive(Clone)]
pub struct MppHttpTransport<P> {
    client: reqwest::Client,
    url: Url,
    provider: P,
}

impl<P> std::fmt::Debug for MppHttpTransport<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MppHttpTransport").field("url", &self.url).finish_non_exhaustive()
    }
}

impl<P: PaymentProvider> MppHttpTransport<P> {
    /// Create a new MPP HTTP transport.
    pub fn new(client: reqwest::Client, url: Url, provider: P) -> Self {
        Self { client, url, provider }
    }

    /// Send a JSON-RPC request with automatic 402 payment handling.
    async fn do_request(self, req: RequestPacket) -> TransportResult<ResponsePacket> {
        let body = serde_json::to_vec(&req).map_err(TransportErrorKind::custom)?;
        let headers = req.headers();

        let resp = self
            .client
            .post(self.url.clone())
            .headers(headers.clone())
            .header("content-type", "application/json")
            .body(body.clone())
            .send()
            .await
            .map_err(TransportErrorKind::custom)?;

        // If not 402, handle normally
        if resp.status() != StatusCode::PAYMENT_REQUIRED {
            return Self::handle_response(resp).await;
        }

        // Parse the 402 challenge
        let www_auth = resp
            .headers()
            .get(WWW_AUTHENTICATE_HEADER)
            .or_else(|| resp.headers().get("www-authenticate"))
            .ok_or_else(|| {
                TransportErrorKind::custom(std::io::Error::other(
                    "402 response missing WWW-Authenticate header",
                ))
            })?
            .to_str()
            .map_err(|e| {
                TransportErrorKind::custom(std::io::Error::other(format!(
                    "invalid WWW-Authenticate header: {e}"
                )))
            })?;

        let challenge = parse_www_authenticate(www_auth).map_err(|e| {
            TransportErrorKind::custom(std::io::Error::other(format!("invalid MPP challenge: {e}")))
        })?;

        debug!(id = %challenge.id, method = %challenge.method, "received MPP 402 challenge, paying");

        // Pay the challenge
        let credential = self.provider.pay(&challenge).await.map_err(|e| {
            TransportErrorKind::custom(std::io::Error::other(format!("MPP payment failed: {e}")))
        })?;

        let auth_header = format_authorization(&credential).map_err(|e| {
            TransportErrorKind::custom(std::io::Error::other(format!(
                "failed to format MPP credential: {e}"
            )))
        })?;

        // Retry with payment credential
        let retry_resp = self
            .client
            .post(self.url.clone())
            .headers(headers)
            .header("content-type", "application/json")
            .header(AUTHORIZATION_HEADER, auth_header)
            .body(body)
            .send()
            .await
            .map_err(TransportErrorKind::custom)?;

        Self::handle_response(retry_resp).await
    }

    async fn handle_response(resp: reqwest::Response) -> TransportResult<ResponsePacket> {
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

impl<P: PaymentProvider + 'static> Service<RequestPacket> for MppHttpTransport<P> {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    #[inline]
    fn poll_ready(&mut self, _cx: &mut task::Context<'_>) -> task::Poll<Result<(), Self::Error>> {
        task::Poll::Ready(Ok(()))
    }

    #[inline]
    fn call(&mut self, req: RequestPacket) -> Self::Future {
        let this = self.clone();
        let span = debug_span!("MppHttpTransport", url = %this.url);
        Box::pin(this.do_request(req).instrument(span.or_current()))
    }
}

/// A minimal EVM payment provider that signs MPP challenges.
#[derive(Clone)]
pub struct EvmSigningProvider {
    signer: alloy_signer_local::PrivateKeySigner,
    /// RPC URL for future use (e.g., nonce/gas queries for full tx signing).
    _rpc_url: Url,
}

impl EvmSigningProvider {
    /// Create a new EVM signing provider.
    pub fn new(signer: alloy_signer_local::PrivateKeySigner, rpc_url: Url) -> Self {
        Self { signer, _rpc_url: rpc_url }
    }
}

impl PaymentProvider for EvmSigningProvider {
    fn supports(&self, _method: &str, _intent: &str) -> bool {
        true
    }

    async fn pay(
        &self,
        challenge: &mpp::PaymentChallenge,
    ) -> Result<mpp::PaymentCredential, mpp::MppError> {
        use alloy_signer::Signer;
        use mpp::protocol::core::PaymentPayload;

        let message = format!("MPP Payment: {}", challenge.id);
        let signature = self
            .signer
            .sign_message(message.as_bytes())
            .await
            .map_err(|e| mpp::MppError::Http(format!("failed to sign: {e}")))?;

        let addr = self.signer.address();
        let credential = mpp::PaymentCredential::with_source(
            challenge.to_echo(),
            format!("did:pkh:eip155:1:{addr}"),
            PaymentPayload::hash(format!(
                "0x{}",
                alloy_primitives::hex::encode(signature.as_bytes())
            )),
        );

        Ok(credential)
    }
}
