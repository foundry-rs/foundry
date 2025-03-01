/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Test connectors that never return data

use aws_smithy_async::future::never::Never;
use aws_smithy_runtime_api::client::connector_metadata::ConnectorMetadata;
use aws_smithy_runtime_api::client::http::{
    HttpClient, HttpConnector, HttpConnectorFuture, HttpConnectorSettings, SharedHttpConnector,
};
use aws_smithy_runtime_api::client::orchestrator::HttpRequest;
use aws_smithy_runtime_api::client::runtime_components::RuntimeComponents;
use aws_smithy_runtime_api::shared::IntoShared;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// A client that will never respond.
///
/// Returned futures will return `Pending` forever
#[derive(Clone, Debug, Default)]
pub struct NeverClient {
    invocations: Arc<AtomicUsize>,
}

impl NeverClient {
    /// Create a new never connector.
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns the number of invocations made to this connector.
    pub fn num_calls(&self) -> usize {
        self.invocations.load(Ordering::SeqCst)
    }
}

impl HttpConnector for NeverClient {
    fn call(&self, _request: HttpRequest) -> HttpConnectorFuture {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        HttpConnectorFuture::new(async move {
            Never::new().await;
            unreachable!()
        })
    }
}

impl HttpClient for NeverClient {
    fn http_connector(
        &self,
        _: &HttpConnectorSettings,
        _: &RuntimeComponents,
    ) -> SharedHttpConnector {
        self.clone().into_shared()
    }

    fn connector_metadata(&self) -> Option<ConnectorMetadata> {
        Some(ConnectorMetadata::new("never-client", None))
    }
}

/// A TCP connector that never connects.
// In the future, this can be available for multiple hyper version feature flags, with the impls gated between individual features
#[cfg(feature = "connector-hyper-0-14-x")]
#[derive(Clone, Debug, Default)]
pub struct NeverTcpConnector;

#[cfg(feature = "connector-hyper-0-14-x")]
impl NeverTcpConnector {
    /// Creates a new `NeverTcpConnector`.
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "connector-hyper-0-14-x")]
impl hyper_0_14::service::Service<http_02x::Uri> for NeverTcpConnector {
    type Response = connection::NeverTcpConnection;
    type Error = aws_smithy_runtime_api::box_error::BoxError;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send + Sync>,
    >;

    fn poll_ready(
        &mut self,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, _: http_02x::Uri) -> Self::Future {
        Box::pin(async {
            Never::new().await;
            unreachable!()
        })
    }
}

#[cfg(feature = "connector-hyper-0-14-x")]
mod connection {
    use hyper_0_14::client::connect::{Connected, Connection};
    use std::io::Error;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    /// A connection type that appeases hyper's trait bounds for a TCP connector, but will panic if any of its traits are used.
    #[non_exhaustive]
    #[derive(Debug, Default)]
    pub struct NeverTcpConnection;

    impl Connection for NeverTcpConnection {
        fn connected(&self) -> Connected {
            unreachable!()
        }
    }

    impl AsyncRead for NeverTcpConnection {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            unreachable!()
        }
    }

    impl AsyncWrite for NeverTcpConnection {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &[u8],
        ) -> Poll<Result<usize, Error>> {
            unreachable!()
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
            unreachable!()
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
            unreachable!()
        }
    }
}

#[cfg(all(test, feature = "connector-hyper-0-14-x"))]
#[tokio::test]
async fn never_tcp_connector_plugs_into_hyper_014() {
    use crate::client::http::hyper_014::HyperClientBuilder;
    use aws_smithy_async::rt::sleep::TokioSleep;
    use aws_smithy_async::time::SystemTimeSource;
    use aws_smithy_runtime_api::client::runtime_components::RuntimeComponentsBuilder;
    use std::time::Duration;

    // it should compile
    let client = HyperClientBuilder::new().build(NeverTcpConnector::new());
    let components = RuntimeComponentsBuilder::for_tests()
        .with_sleep_impl(Some(TokioSleep::new()))
        .with_time_source(Some(SystemTimeSource::new()))
        .build()
        .unwrap();
    let http_connector = client.http_connector(
        &HttpConnectorSettings::builder()
            .connect_timeout(Duration::from_millis(100))
            .build(),
        &components,
    );

    let err = http_connector
        .call(HttpRequest::get("http://fakeuri.com").unwrap())
        .await
        .expect_err("it should time out");
    assert!(dbg!(err).is_timeout());
}
