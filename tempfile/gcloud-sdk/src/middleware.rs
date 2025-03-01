use crate::token_source::auth_token_generator::GoogleAuthTokenGenerator;
use chrono::Utc;
use futures::{Future, TryFutureExt};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tonic::client::GrpcService;
use tower::Service;
use tower_layer::Layer;
use tracing::*;

#[derive(Clone)]
pub struct GoogleAuthMiddlewareService<T>
where
    T: Clone,
{
    google_service: Option<T>,
    token_generator: Arc<GoogleAuthTokenGenerator>,
    cloud_resource_prefix: Option<String>,
}

impl<T> GoogleAuthMiddlewareService<T>
where
    T: Clone,
{
    pub fn new(
        service: T,
        token_generator: Arc<GoogleAuthTokenGenerator>,
        cloud_resource_prefix: Option<String>,
    ) -> GoogleAuthMiddlewareService<T> {
        GoogleAuthMiddlewareService {
            google_service: Some(service),
            token_generator,
            cloud_resource_prefix,
        }
    }
}

impl<T, RequestBody> Service<hyper::Request<RequestBody>> for GoogleAuthMiddlewareService<T>
where
    T: GrpcService<RequestBody> + Send + Clone + 'static,
    T::Future: 'static + Send,
    RequestBody: 'static + Send,
    T::ResponseBody: 'static + Send,
    T::Error: 'static + Send,
{
    type Response = hyper::Response<T::ResponseBody>;
    type Error = Box<dyn std::error::Error + Send + Sync + 'static>;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if let Some(ref mut google_service) = self.google_service.as_mut() {
            google_service.poll_ready(cx).map_err(|e| e.into())
        } else {
            Poll::Pending
        }
    }

    fn call(&mut self, mut req: hyper::Request<RequestBody>) -> Self::Future {
        let generator = self.token_generator.clone();
        let cloud_resource_prefix = self.cloud_resource_prefix.clone();

        if let Some(mut google_service) = self.google_service.take() {
            self.google_service = Some(google_service.clone());
            Box::pin(async move {
                let begin_time = Utc::now();
                let token = generator.create_token().await.map_err(Box::new)?;
                let token_generated_time = Utc::now();
                let headers = req.headers_mut();
                headers.insert("authorization", token.header_value().parse()?);
                if let Some(cloud_resource_prefix_value) = cloud_resource_prefix {
                    headers.insert(
                        "google-cloud-resource-prefix",
                        cloud_resource_prefix_value.parse()?,
                    );
                }
                let req_uri_str = req.uri().to_string();
                google_service
                    .call(req)
                    .map_ok(|x| {
                        let finished_time = Utc::now();
                        debug!(
                            "OK: {} took {}ms (incl. token gen: {}ms)",
                            req_uri_str,
                            finished_time
                                .signed_duration_since(begin_time)
                                .num_milliseconds(),
                            token_generated_time
                                .signed_duration_since(begin_time)
                                .num_milliseconds()
                        );
                        x
                    })
                    .await
                    .map_err(|e| {
                        let finished_time = Utc::now();
                        error!(
                            "Err: {} took {}ms (incl. token gen: {}ms)",
                            req_uri_str,
                            finished_time
                                .signed_duration_since(begin_time)
                                .num_milliseconds(),
                            token_generated_time
                                .signed_duration_since(begin_time)
                                .num_milliseconds()
                        );
                        e.into()
                    })
            })
        } else {
            panic!("Should never happen, system error");
        }
    }
}

pub struct GoogleAuthMiddlewareLayer {
    token_generator: Arc<GoogleAuthTokenGenerator>,
    cloud_resource_prefix: Option<String>,
}

impl GoogleAuthMiddlewareLayer {
    pub fn new(
        token_generator: GoogleAuthTokenGenerator,
        cloud_resource_prefix: Option<String>,
    ) -> Self {
        GoogleAuthMiddlewareLayer {
            token_generator: Arc::new(token_generator),
            cloud_resource_prefix,
        }
    }
}

impl<S> Layer<S> for GoogleAuthMiddlewareLayer
where
    S: Clone,
{
    type Service = GoogleAuthMiddlewareService<S>;

    fn layer(&self, service: S) -> GoogleAuthMiddlewareService<S> {
        GoogleAuthMiddlewareService::new(
            service,
            self.token_generator.clone(),
            self.cloud_resource_prefix.clone(),
        )
    }
}
