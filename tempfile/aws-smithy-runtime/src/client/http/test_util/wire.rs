/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Utilities for mocking at the socket level
//!
//! Other tools in this module actually operate at the `http::Request` / `http::Response` level. This
//! is useful, but it shortcuts the HTTP implementation (e.g. Hyper). [`WireMockServer`] binds
//! to an actual socket on the host.
//!
//! # Examples
//! ```no_run
//! use aws_smithy_runtime_api::client::http::HttpConnectorSettings;
//! use aws_smithy_runtime::client::http::test_util::wire::{check_matches, ReplayedEvent, WireMockServer};
//! use aws_smithy_runtime::{match_events, ev};
//! # async fn example() {
//!
//! // This connection binds to a local address
//! let mock = WireMockServer::start(vec![
//!     ReplayedEvent::status(503),
//!     ReplayedEvent::status(200)
//! ]).await;
//!
//! # /*
//! // Create a client using the wire mock
//! let config = my_generated_client::Config::builder()
//!     .http_client(mock.http_client())
//!     .build();
//! let client = Client::from_conf(config);
//!
//! // ... do something with <client>
//! # */
//!
//! // assert that you got the events you expected
//! match_events!(ev!(dns), ev!(connect), ev!(http(200)))(&mock.events());
//! # }
//! ```

#![allow(missing_docs)]

use crate::client::http::hyper_014::HyperClientBuilder;
use aws_smithy_async::future::never::Never;
use aws_smithy_async::future::BoxFuture;
use aws_smithy_runtime_api::client::http::SharedHttpClient;
use aws_smithy_runtime_api::shared::IntoShared;
use bytes::Bytes;
use hyper_0_14::client::connect::dns::Name;
use hyper_0_14::server::conn::AddrStream;
use hyper_0_14::service::{make_service_fn, service_fn, Service};
use std::collections::HashSet;
use std::convert::Infallible;
use std::error::Error;
use std::iter::Once;
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::spawn;
use tokio::sync::oneshot;

/// An event recorded by [`WireMockServer`].
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum RecordedEvent {
    DnsLookup(String),
    NewConnection,
    Response(ReplayedEvent),
}

type Matcher = (
    Box<dyn Fn(&RecordedEvent) -> Result<(), Box<dyn Error>>>,
    &'static str,
);

/// This method should only be used by the macro
pub fn check_matches(events: &[RecordedEvent], matchers: &[Matcher]) {
    let mut events_iter = events.iter();
    let mut matcher_iter = matchers.iter();
    let mut idx = -1;
    loop {
        idx += 1;
        let bail = |err: Box<dyn Error>| panic!("failed on event {}:\n  {}", idx, err);
        match (events_iter.next(), matcher_iter.next()) {
            (Some(event), Some((matcher, _msg))) => matcher(event).unwrap_or_else(bail),
            (None, None) => return,
            (Some(event), None) => {
                bail(format!("got {:?} but no more events were expected", event).into())
            }
            (None, Some((_expect, msg))) => {
                bail(format!("expected {:?} but no more events were expected", msg).into())
            }
        }
    }
}

#[macro_export]
macro_rules! matcher {
    ($expect:tt) => {
        (
            Box::new(
                |event: &$crate::client::http::test_util::wire::RecordedEvent| {
                    if !matches!(event, $expect) {
                        return Err(format!(
                            "expected `{}` but got {:?}",
                            stringify!($expect),
                            event
                        )
                        .into());
                    }
                    Ok(())
                },
            ),
            stringify!($expect),
        )
    };
}

/// Helper macro to generate a series of test expectations
#[macro_export]
macro_rules! match_events {
        ($( $expect:pat),*) => {
            |events| {
                $crate::client::http::test_util::wire::check_matches(events, &[$( $crate::matcher!($expect) ),*]);
            }
        };
    }

/// Helper to generate match expressions for events
#[macro_export]
macro_rules! ev {
    (http($status:expr)) => {
        $crate::client::http::test_util::wire::RecordedEvent::Response(
            $crate::client::http::test_util::wire::ReplayedEvent::HttpResponse {
                status: $status,
                ..
            },
        )
    };
    (dns) => {
        $crate::client::http::test_util::wire::RecordedEvent::DnsLookup(_)
    };
    (connect) => {
        $crate::client::http::test_util::wire::RecordedEvent::NewConnection
    };
    (timeout) => {
        $crate::client::http::test_util::wire::RecordedEvent::Response(
            $crate::client::http::test_util::wire::ReplayedEvent::Timeout,
        )
    };
}

pub use {ev, match_events, matcher};

#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayedEvent {
    Timeout,
    HttpResponse { status: u16, body: Bytes },
}

impl ReplayedEvent {
    pub fn ok() -> Self {
        Self::HttpResponse {
            status: 200,
            body: Bytes::new(),
        }
    }

    pub fn with_body(body: impl AsRef<[u8]>) -> Self {
        Self::HttpResponse {
            status: 200,
            body: Bytes::copy_from_slice(body.as_ref()),
        }
    }

    pub fn status(status: u16) -> Self {
        Self::HttpResponse {
            status,
            body: Bytes::new(),
        }
    }
}

/// Test server that binds to 127.0.0.1:0
///
/// See the [module docs](crate::client::http::test_util::wire) for a usage example.
///
/// Usage:
/// - Call [`WireMockServer::start`] to start the server
/// - Use [`WireMockServer::http_client`] or [`dns_resolver`](WireMockServer::dns_resolver) to configure your client.
/// - Make requests to [`endpoint_url`](WireMockServer::endpoint_url).
/// - Once the test is complete, retrieve a list of events from [`WireMockServer::events`]
#[derive(Debug)]
pub struct WireMockServer {
    event_log: Arc<Mutex<Vec<RecordedEvent>>>,
    bind_addr: SocketAddr,
    // when the sender is dropped, that stops the server
    shutdown_hook: oneshot::Sender<()>,
}

impl WireMockServer {
    /// Start a wire mock server with the given events to replay.
    pub async fn start(mut response_events: Vec<ReplayedEvent>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let (tx, rx) = oneshot::channel();
        let listener_addr = listener.local_addr().unwrap();
        response_events.reverse();
        let response_events = Arc::new(Mutex::new(response_events));
        let handler_events = response_events;
        let wire_events = Arc::new(Mutex::new(vec![]));
        let wire_log_for_service = wire_events.clone();
        let poisoned_conns: Arc<Mutex<HashSet<SocketAddr>>> = Default::default();
        let make_service = make_service_fn(move |connection: &AddrStream| {
            let poisoned_conns = poisoned_conns.clone();
            let events = handler_events.clone();
            let wire_log = wire_log_for_service.clone();
            let remote_addr = connection.remote_addr();
            tracing::info!("established connection: {:?}", connection);
            wire_log.lock().unwrap().push(RecordedEvent::NewConnection);
            async move {
                Ok::<_, Infallible>(service_fn(move |_: http_02x::Request<hyper_0_14::Body>| {
                    if poisoned_conns.lock().unwrap().contains(&remote_addr) {
                        tracing::error!("poisoned connection {:?} was reused!", &remote_addr);
                        panic!("poisoned connection was reused!");
                    }
                    let next_event = events.clone().lock().unwrap().pop();
                    let wire_log = wire_log.clone();
                    let poisoned_conns = poisoned_conns.clone();
                    async move {
                        let next_event = next_event
                            .unwrap_or_else(|| panic!("no more events! Log: {:?}", wire_log));
                        wire_log
                            .lock()
                            .unwrap()
                            .push(RecordedEvent::Response(next_event.clone()));
                        if next_event == ReplayedEvent::Timeout {
                            tracing::info!("{} is poisoned", remote_addr);
                            poisoned_conns.lock().unwrap().insert(remote_addr);
                        }
                        tracing::debug!("replying with {:?}", next_event);
                        let event = generate_response_event(next_event).await;
                        dbg!(event)
                    }
                }))
            }
        });
        let server = hyper_0_14::Server::from_tcp(listener)
            .unwrap()
            .serve(make_service)
            .with_graceful_shutdown(async {
                rx.await.ok();
                tracing::info!("server shutdown!");
            });
        spawn(server);
        Self {
            event_log: wire_events,
            bind_addr: listener_addr,
            shutdown_hook: tx,
        }
    }

    /// Retrieve the events recorded by this connection
    pub fn events(&self) -> Vec<RecordedEvent> {
        self.event_log.lock().unwrap().clone()
    }

    fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    pub fn dns_resolver(&self) -> LoggingDnsResolver {
        let event_log = self.event_log.clone();
        let bind_addr = self.bind_addr;
        LoggingDnsResolver {
            log: event_log,
            socket_addr: bind_addr,
        }
    }

    /// Prebuilt [`HttpClient`](aws_smithy_runtime_api::client::http::HttpClient) with correctly wired DNS resolver.
    ///
    /// **Note**: This must be used in tandem with [`Self::dns_resolver`]
    pub fn http_client(&self) -> SharedHttpClient {
        HyperClientBuilder::new()
            .build(hyper_0_14::client::HttpConnector::new_with_resolver(
                self.dns_resolver(),
            ))
            .into_shared()
    }

    /// Endpoint to use when connecting
    ///
    /// This works in tandem with the [`Self::dns_resolver`] to bind to the correct local IP Address
    pub fn endpoint_url(&self) -> String {
        format!(
            "http://this-url-is-converted-to-localhost.com:{}",
            self.bind_addr().port()
        )
    }

    /// Shuts down the mock server.
    pub fn shutdown(self) {
        let _ = self.shutdown_hook.send(());
    }
}

async fn generate_response_event(
    event: ReplayedEvent,
) -> Result<http_02x::Response<hyper_0_14::Body>, Infallible> {
    let resp = match event {
        ReplayedEvent::HttpResponse { status, body } => http_02x::Response::builder()
            .status(status)
            .body(hyper_0_14::Body::from(body))
            .unwrap(),
        ReplayedEvent::Timeout => {
            Never::new().await;
            unreachable!()
        }
    };
    Ok::<_, Infallible>(resp)
}

/// DNS resolver that keeps a log of all lookups
///
/// Regardless of what hostname is requested, it will always return the same socket address.
#[derive(Clone, Debug)]
pub struct LoggingDnsResolver {
    log: Arc<Mutex<Vec<RecordedEvent>>>,
    socket_addr: SocketAddr,
}

impl Service<Name> for LoggingDnsResolver {
    type Response = Once<SocketAddr>;
    type Error = Infallible;
    type Future = BoxFuture<'static, Self::Response, Self::Error>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Name) -> Self::Future {
        let socket_addr = self.socket_addr;
        let log = self.log.clone();
        Box::pin(async move {
            println!("looking up {:?}, replying with {:?}", req, socket_addr);
            log.lock()
                .unwrap()
                .push(RecordedEvent::DnsLookup(req.to_string()));
            Ok(std::iter::once(socket_addr))
        })
    }
}
