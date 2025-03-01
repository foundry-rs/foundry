use crate::hyper::{header::AUTHORIZATION, Request, Response};
use alloy_rpc_types_engine::{Claims, JwtSecret};
use alloy_transport::{TransportError, TransportErrorKind};
use hyper::header::HeaderValue;
use jsonwebtoken::get_current_timestamp;
use std::{
    future::Future,
    pin::Pin,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tower::{Layer, Service};

/// The [`AuthLayer`] uses the provided [`JwtSecret`] to generate and validate the jwt token
/// in the requests.
///
/// The generated token is inserted into the [`AUTHORIZATION`] header of the request.
#[derive(Clone, Debug)]
pub struct AuthLayer {
    secret: JwtSecret,
    latency_buffer: u64,
}

impl AuthLayer {
    /// Create a new [`AuthLayer`].
    pub const fn new(secret: JwtSecret) -> Self {
        Self { secret, latency_buffer: 5000 }
    }

    /// We use this buffer to perfom an extra check on the `iat` field to prevent sending any
    /// requests with tokens that are valid now but may not be upon reaching the server.
    ///
    /// In milliseconds. Default is 5s.
    pub const fn with_latency_buffer(self, latency_buffer: u64) -> Self {
        Self { latency_buffer, ..self }
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthService::new(inner, self.secret, self.latency_buffer)
    }
}

/// A service that generates and validates the jwt token in the requests using the provided secret.
#[derive(Clone, Debug)]
pub struct AuthService<S> {
    inner: S,
    secret: JwtSecret,
    /// In milliseconds.
    latency_buffer: u64,
    most_recent_claim: Option<Claims>,
}

impl<S> AuthService<S> {
    /// Create a new [`AuthService`] with the given inner service.
    pub const fn new(inner: S, secret: JwtSecret, latency_buffer: u64) -> Self {
        Self { inner, secret, latency_buffer, most_recent_claim: None }
    }

    /// Validate the token in the request headers.
    ///
    /// Returns `true` if the token is still valid and `iat` is beyond the grace buffer.
    fn validate(&self) -> bool {
        if let Some(claim) = self.most_recent_claim.as_ref() {
            let curr_secs = get_current_timestamp();
            if claim.iat.abs_diff(curr_secs) * 1000 > self.latency_buffer {
                return true;
            }
        }

        false
    }

    /// Create a new token from the secret.
    ///
    /// Updates the most_recent_claim with the new claim.
    fn create_token_from_secret(&mut self) -> Result<String, jsonwebtoken::errors::Error> {
        let claims = Claims {
            iat: (SystemTime::now().duration_since(UNIX_EPOCH).unwrap() + Duration::from_secs(60))
                .as_secs(),
            exp: None,
        };

        self.most_recent_claim = Some(claims);

        let token = self.secret.encode(&claims)?;

        Ok(format!("Bearer {}", token))
    }
}

impl<S, B, ResBody> Service<Request<B>> for AuthService<S>
where
    S: Service<hyper::Request<B>, Response = Response<ResBody>> + Clone + Send + Sync + 'static,
    S::Future: Send,
    S::Error: std::error::Error + Send + Sync + 'static,
    B: From<Vec<u8>> + Send + 'static + Clone + Sync,
    ResBody: hyper::body::Body + Send + 'static,
    ResBody::Error: std::error::Error + Send + Sync + 'static,
    ResBody::Data: Send,
{
    type Response = Response<ResBody>;
    type Error = TransportError;
    type Future =
        Pin<Box<dyn Future<Output = Result<Response<ResBody>, Self::Error>> + Send + 'static>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(TransportErrorKind::custom)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        let mut req = req;
        let res = if self.validate() {
            // Encodes the most recent claim into a token.
            self.secret.encode(self.most_recent_claim.as_ref().unwrap())
        } else {
            // Creates a new Claim and encodes it into a token.
            self.create_token_from_secret()
        };

        match res {
            Ok(token) => {
                req.headers_mut().insert(AUTHORIZATION, HeaderValue::from_str(&token).unwrap());

                let mut this = self.clone();

                Box::pin(
                    async move { this.inner.call(req).await.map_err(TransportErrorKind::custom) },
                )
            }
            Err(e) => {
                let e = TransportErrorKind::custom(e);
                Box::pin(async move { Err(e) })
            }
        }
    }
}
