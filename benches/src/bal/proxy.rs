//! Per-attempt HTTP RPC proxy with streaming, cancellation-aware accounting.
//!
//! All exchanges are metered, but only standalone BAL responses retain bodies for analysis.
//! Batches pass through unchanged; BAL injection requires a standalone request.

use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
    routing::post,
};
use eyre::{Result, bail, eyre};
use futures::{Stream, StreamExt};
use hyper_util::{rt::TokioIo, service::TowerToHyperService};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::{
    net::TcpListener,
    sync::watch,
    task::{JoinHandle, JoinSet},
};

const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;
const MAX_BAL_CAPTURE_BYTES: usize = 128 * 1024 * 1024;

/// Only standalone BAL methods are affected; other RPC calls use the same upstream.
#[derive(Clone, Debug)]
pub enum Policy {
    Passthrough,
    MethodNotFound,
    Null,
    Recorded(Value),
    /// A missing result forwards the BAL request after the delay.
    Delay {
        millis: u64,
        result: Option<Value>,
    },
}

/// Observed termination of a body, rather than successful handler return.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Completion {
    #[default]
    Pending,
    Eof,
    Dropped,
    TransportError,
    Cancelled,
}

/// Bytes are HTTP body bytes; headers and transfer framing are excluded.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BodyMetrics {
    pub body_bytes: u64,
    pub completion: Completion,
    pub elapsed_seconds: Option<f64>,
    pub content_encoding: String,
    pub completed_during_cleanup: bool,
    pub cleanup_body_bytes: u64,
}

impl Default for BodyMetrics {
    fn default() -> Self {
        Self {
            body_bytes: 0,
            completion: Completion::Pending,
            elapsed_seconds: None,
            content_encoding: "identity".into(),
            completed_during_cleanup: false,
            cleanup_body_bytes: 0,
        }
    }
}

/// Standalone BAL response classification is derived after the timed attempt.
/// Other RPC responses remain unobserved; HTTP and body metrics still cover every exchange.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RpcResponse {
    #[default]
    Unobserved,
    Result,
    Null,
    Error,
    Missing,
    Malformed,
    Ambiguous,
    Notification,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcCall {
    pub method: String,
    pub id: Option<Value>,
    pub params: Value,
    pub forwarded: bool,
    pub injected: bool,
    pub repeated: bool,
    pub response: RpcResponse,
    pub error_code: Option<i64>,
    pub result_json_bytes: Option<u64>,
}

/// One HTTP exchange. A batch owns its body byte counts exactly once.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcEvent {
    pub exchange_id: u64,
    pub batch: bool,
    pub calls: Vec<RpcCall>,
    pub client_request_body_bytes: u64,
    pub upstream_request_body_bytes: u64,
    pub upstream_http_exchanges: u64,
    pub http_status: Option<u16>,
    pub upstream_http_status: Option<u16>,
    pub response: BodyMetrics,
    pub upstream: Option<BodyMetrics>,
    pub issues: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub client_requests_by_method: BTreeMap<String, u64>,
    pub upstream_requests_by_method: BTreeMap<String, u64>,
    pub client_http_exchanges: u64,
    pub upstream_http_exchanges: u64,
    pub injected_responses: u64,
    pub client_response_body_bytes: u64,
    pub upstream_response_body_bytes: u64,
    pub repeated_requests: u64,
    /// HTTP/transport issues across all exchanges, plus analyzed BAL JSON-RPC errors.
    pub errors: u64,
    pub active_http_exchanges: u64,
    /// False for cheap process-exit snapshots, before deferred standalone BAL JSON analysis.
    pub response_analysis_complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub snapshot: Snapshot,
    pub events: Vec<RpcEvent>,
}

struct RecordedEvent {
    event: RpcEvent,
    started: Instant,
    upstream_started: Option<Instant>,
    chunks: Vec<Bytes>,
    capture_complete: bool,
}

#[derive(Default)]
struct Ledger {
    events: Vec<RecordedEvent>,
    requests: HashSet<String>,
    cleanup: bool,
}

struct ProxyState {
    client: reqwest::Client,
    upstream: reqwest::Url,
    policy: Policy,
    ledger: Mutex<Ledger>,
    shutdown: watch::Sender<bool>,
}

/// One proxy is owned by exactly one attempt. Never reset or reuse its counters.
pub struct Proxy {
    url: String,
    state: Arc<ProxyState>,
    server: Option<JoinHandle<()>>,
}

impl Proxy {
    pub async fn start(upstream: &str, policy: Policy) -> Result<Self> {
        let upstream = reqwest::Url::parse(upstream).map_err(|_| eyre!("invalid upstream URL"))?;
        if !matches!(upstream.scheme(), "http" | "https") {
            bail!("BAL metering supports HTTP and HTTPS upstreams only");
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .build()
            .map_err(|_| eyre!("failed to initialize metering HTTP client"))?;
        let (shutdown, receiver) = watch::channel(false);
        let state = Arc::new(ProxyState {
            client,
            upstream,
            policy,
            ledger: Mutex::new(Ledger::default()),
            shutdown,
        });
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let router = Router::new().route("/", post(handle)).with_state(Arc::clone(&state));
        let server = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            let shutdown = cancelled(receiver);
            tokio::pin!(shutdown);
            loop {
                tokio::select! {
                    biased;
                    () = &mut shutdown => break,
                    accepted = listener.accept() => {
                        let Ok((socket, _)) = accepted else { break };
                        let service = TowerToHyperService::new(router.clone());
                        connections.spawn(async move {
                            let _ = hyper::server::conn::http1::Builder::new()
                                .serve_connection(TokioIo::new(socket), service).await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
            // Joining each connection proves every local handler and response stream has
            // dropped before its sample ledger is detached.
            connections.abort_all();
            while connections.join_next().await.is_some() {}
        });
        Ok(Self { url, state, server: Some(server) })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// A cheap snapshot suitable for the exact child-exit boundary.
    pub fn snapshot(&self) -> Snapshot {
        snapshot(self.state.ledger.lock().unwrap().events.iter().map(|event| &event.event), false)
    }

    /// Atomically mark the timing boundary; late bytes during pipe draining belong to cleanup.
    /// This does not cancel in-flight work. `finish` closes and joins it afterward.
    pub fn end_measurement(&self) -> Snapshot {
        let mut ledger = self.state.ledger.lock().unwrap();
        let snapshot = snapshot(ledger.events.iter().map(|event| &event.event), false);
        ledger.cleanup = true;
        snapshot
    }

    /// Cancel outstanding local work and derive JSON metrics outside the measured interval.
    /// Disconnecting upstream does not prove that the remote node stopped its EVM work.
    pub async fn finish(mut self) -> Session {
        self.state.ledger.lock().unwrap().cleanup = true;
        let _ = self.state.shutdown.send(true);
        if let Some(server) = self.server.take() {
            let _ = server.await;
        }
        let mut ledger = self.state.ledger.lock().unwrap();
        let mut recorded = std::mem::take(&mut ledger.events);
        drop(ledger);
        for raw in &mut recorded {
            if raw.event.response.completion == Completion::Pending {
                complete(raw, Completion::Cancelled, true);
                raw.event.issues.push("cleanup_deadline".into());
            }
            analyze_response(raw);
        }
        let events = recorded.into_iter().map(|raw| raw.event).collect::<Vec<_>>();
        Session { snapshot: snapshot(events.iter(), true), events }
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = self.state.shutdown.send(true);
        if let Some(server) = &self.server {
            server.abort();
        }
    }
}

/// Includes the public aliases used by providers; every method is still counted by actual name.
pub fn is_bal_method(method: &str) -> bool {
    matches!(
        method,
        "eth_getBlockAccessListByBlockHash"
            | "eth_getBlockAccessListByBlockNumber"
            | "eth_getBlockAccessList"
            | "debug_getBlockAccessList"
    )
}

async fn cancelled(mut receiver: watch::Receiver<bool>) {
    if !*receiver.borrow() {
        let _ = receiver.changed().await;
    }
}

struct Exchange {
    state: Arc<ProxyState>,
    index: usize,
    completed: bool,
}

impl Exchange {
    fn new(state: Arc<ProxyState>) -> Self {
        let index = {
            let mut ledger = state.ledger.lock().unwrap();
            let index = ledger.events.len();
            ledger.events.push(RecordedEvent {
                event: RpcEvent {
                    exchange_id: index as u64,
                    batch: false,
                    calls: Vec::new(),
                    client_request_body_bytes: 0,
                    upstream_request_body_bytes: 0,
                    upstream_http_exchanges: 0,
                    http_status: None,
                    upstream_http_status: None,
                    response: BodyMetrics::default(),
                    upstream: None,
                    issues: Vec::new(),
                },
                started: Instant::now(),
                upstream_started: None,
                chunks: Vec::new(),
                capture_complete: false,
            });
            index
        };
        Self { state, index, completed: false }
    }

    fn update(&self, f: impl FnOnce(&mut RecordedEvent, bool)) {
        let mut ledger = self.state.ledger.lock().unwrap();
        let cleanup = ledger.cleanup;
        // A cleanup deadline can detach the ledger after cancelling all local work.
        if let Some(raw) = ledger.events.get_mut(self.index) {
            f(raw, cleanup);
        }
    }

    fn finish(&mut self, completion: Completion) {
        if !self.completed {
            self.update(|raw, cleanup| complete(raw, completion, cleanup));
            self.completed = true;
        }
    }
}

impl Drop for Exchange {
    fn drop(&mut self) {
        let completion =
            if *self.state.shutdown.borrow() { Completion::Cancelled } else { Completion::Dropped };
        self.finish(completion);
    }
}

fn complete(raw: &mut RecordedEvent, completion: Completion, cleanup: bool) {
    raw.event.response.completion = completion;
    raw.event.response.elapsed_seconds = Some(raw.started.elapsed().as_secs_f64());
    raw.event.response.completed_during_cleanup = cleanup;
    if let Some(upstream) = &mut raw.event.upstream
        && upstream.completion == Completion::Pending
    {
        upstream.completion = completion;
        upstream.elapsed_seconds = raw.upstream_started.map(|start| start.elapsed().as_secs_f64());
        upstream.completed_during_cleanup = cleanup;
    }
}

async fn handle(State(state): State<Arc<ProxyState>>, request: Request) -> Response {
    let exchange = Exchange::new(Arc::clone(&state));
    let receiver = state.shutdown.subscribe();
    tokio::select! {
        biased;
        () = cancelled(receiver) => Response::new(Body::empty()),
        response = handle_exchange(exchange, request) => response,
    }
}

async fn handle_exchange(exchange: Exchange, request: Request) -> Response {
    let Ok(body) = to_bytes(request.into_body(), MAX_REQUEST_BYTES).await else {
        exchange.update(|raw, _| raw.event.issues.push("invalid_request_body".into()));
        return measured_response(
            exchange,
            StatusCode::BAD_REQUEST,
            HeaderMap::new(),
            Bytes::new(),
        );
    };
    exchange.update(|raw, _| raw.event.client_request_body_bytes = body.len() as u64);
    let Ok(value) = serde_json::from_slice::<Value>(&body) else {
        exchange.update(|raw, _| raw.event.issues.push("invalid_request_json".into()));
        return measured_response(
            exchange,
            StatusCode::BAD_REQUEST,
            HeaderMap::new(),
            Bytes::new(),
        );
    };
    let batch = value.is_array();
    let requests = match &value {
        Value::Array(requests) if !requests.is_empty() => requests.as_slice(),
        Value::Object(_) => std::slice::from_ref(&value),
        _ => {
            exchange.update(|raw, _| raw.event.issues.push("invalid_request_shape".into()));
            return measured_response(
                exchange,
                StatusCode::BAD_REQUEST,
                HeaderMap::new(),
                Bytes::new(),
            );
        }
    };
    let has_bal =
        requests.iter().any(|request| request["method"].as_str().is_some_and(is_bal_method));
    {
        let mut ledger = exchange.state.ledger.lock().unwrap();
        let calls = requests
            .iter()
            .map(|request| {
                let method = request["method"].as_str().unwrap_or("<invalid>").to_owned();
                let params = request.get("params").cloned().unwrap_or(Value::Null);
                let repeated = !ledger.requests.insert(json!([method, params]).to_string());
                RpcCall {
                    method,
                    id: request.get("id").cloned(),
                    params,
                    forwarded: false,
                    injected: false,
                    repeated,
                    response: RpcResponse::Unobserved,
                    error_code: None,
                    result_json_bytes: None,
                }
            })
            .collect();
        let raw = &mut ledger.events[exchange.index];
        raw.event.batch = batch;
        raw.event.calls = calls;
        raw.capture_complete = !batch && has_bal;
    }
    if has_bal {
        if batch && !matches!(exchange.state.policy, Policy::Passthrough) {
            exchange.update(|raw, _| {
                raw.event.issues.push("unsupported_bal_batch_injection".into());
            });
            return measured_response(
                exchange,
                StatusCode::BAD_REQUEST,
                HeaderMap::new(),
                Bytes::new(),
            );
        }
        if !batch {
            let id = value.get("id");
            let replacement = match &exchange.state.policy {
                Policy::Passthrough => None,
                Policy::MethodNotFound => Some(
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}}),
                ),
                Policy::Null => Some(json!({"jsonrpc":"2.0","id":id,"result":null})),
                Policy::Recorded(result) => Some(json!({"jsonrpc":"2.0","id":id,"result":result})),
                Policy::Delay { millis, result } => {
                    tokio::time::sleep(Duration::from_millis(*millis)).await;
                    result.as_ref().map(|result| json!({"jsonrpc":"2.0","id":id,"result":result}))
                }
            };
            if let Some(replacement) = replacement {
                let response = if id.is_some() {
                    exchange.update(|raw, _| raw.event.calls[0].injected = true);
                    Bytes::from(serde_json::to_vec(&replacement).unwrap())
                } else {
                    Bytes::new()
                };
                return measured_response(exchange, StatusCode::OK, HeaderMap::new(), response);
            }
        }
    }
    exchange.update(|raw, _| {
        raw.upstream_started = Some(Instant::now());
        raw.event.upstream = Some(BodyMetrics::default());
        raw.event.upstream_request_body_bytes = body.len() as u64;
        raw.event.upstream_http_exchanges = 1;
        for call in &mut raw.event.calls {
            call.forwarded = true;
        }
    });
    let response = exchange
        .state
        .client
        .post(exchange.state.upstream.clone())
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT_ENCODING, "identity")
        .body(body)
        .send()
        .await;
    let Ok(response) = response else {
        exchange.update(|raw, cleanup| {
            raw.event.issues.push("upstream_transport_error".into());
            finish_upstream(raw, Completion::TransportError, cleanup);
        });
        return measured_response(
            exchange,
            StatusCode::BAD_GATEWAY,
            HeaderMap::new(),
            Bytes::new(),
        );
    };
    let status = response.status();
    let headers = response.headers().clone();
    exchange.update(|raw, _| {
        raw.event.upstream_http_status = Some(status.as_u16());
        raw.event.upstream.as_mut().unwrap().content_encoding = encoding(&headers);
        if !status.is_success() {
            raw.event.issues.push("upstream_http_error".into());
        }
    });
    let stream = Box::pin(
        response
            .bytes_stream()
            .map(|chunk| chunk.map_err(|_| io::Error::other("upstream response body error"))),
    );
    stream_response(exchange, status, headers, stream, true)
}

fn encoding(headers: &HeaderMap) -> String {
    headers
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("identity")
        .to_owned()
}

fn finish_upstream(raw: &mut RecordedEvent, completion: Completion, cleanup: bool) {
    if let Some(upstream) = &mut raw.event.upstream {
        upstream.completion = completion;
        upstream.elapsed_seconds = raw.upstream_started.map(|start| start.elapsed().as_secs_f64());
        upstream.completed_during_cleanup = cleanup;
    }
}

const fn add_upstream_bytes(raw: &mut RecordedEvent, bytes: usize, cleanup: bool) {
    if let Some(upstream) = &mut raw.event.upstream {
        upstream.body_bytes += bytes as u64;
        if cleanup {
            upstream.cleanup_body_bytes += bytes as u64;
        }
    }
}

type ByteStream = Pin<Box<dyn Stream<Item = io::Result<Bytes>> + Send>>;

struct MeteredStream {
    inner: ByteStream,
    exchange: Exchange,
    upstream: bool,
    cancelled: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl Stream for MeteredStream {
    type Item = io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.exchange.completed {
            return Poll::Ready(None);
        }
        if this.cancelled.as_mut().poll(cx).is_ready() {
            this.exchange.finish(Completion::Cancelled);
            return Poll::Ready(None);
        }
        match this.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                this.exchange.update(|raw, cleanup| {
                    raw.event.response.body_bytes += bytes.len() as u64;
                    if cleanup {
                        raw.event.response.cleanup_body_bytes += bytes.len() as u64;
                    }
                    if this.upstream {
                        add_upstream_bytes(raw, bytes.len(), cleanup);
                    }
                    if raw.capture_complete {
                        if raw.event.response.body_bytes <= MAX_BAL_CAPTURE_BYTES as u64 {
                            raw.chunks.push(bytes.clone());
                        } else {
                            raw.capture_complete = false;
                            raw.chunks.clear();
                            raw.event.issues.push("response_analysis_body_limit".into());
                        }
                    }
                });
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(error))) => {
                this.exchange.finish(Completion::TransportError);
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                this.exchange.finish(Completion::Eof);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

fn stream_response(
    exchange: Exchange,
    status: StatusCode,
    mut headers: HeaderMap,
    inner: ByteStream,
    upstream: bool,
) -> Response {
    exchange.update(|raw, _| {
        raw.event.http_status = Some(status.as_u16());
        raw.event.response.content_encoding = encoding(&headers);
    });
    let cancelled = Box::pin(cancelled(exchange.state.shutdown.subscribe()));
    let body = Body::from_stream(MeteredStream { inner, exchange, upstream, cancelled });
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let connection_headers = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|name| name.trim().to_owned())
        .collect::<Vec<_>>();
    for name in connection_headers {
        headers.remove(name);
    }
    // Preserve end-to-end semantics such as Retry-After. Content-Length is deliberately
    // omitted so Hyper polls the stream to EOF instead of inferring completion from length.
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "content-length",
    ] {
        headers.remove(name);
    }
    *response.headers_mut() = headers;
    response
        .headers_mut()
        .entry(header::CONTENT_TYPE)
        .or_insert(axum::http::HeaderValue::from_static("application/json"));
    response
}

fn measured_response(
    exchange: Exchange,
    status: StatusCode,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    stream_response(exchange, status, headers, Box::pin(futures::stream::iter([Ok(bytes)])), false)
}

fn analyze_response(raw: &mut RecordedEvent) {
    for call in &mut raw.event.calls {
        if call.id.is_none() {
            call.response = RpcResponse::Notification;
        }
    }
    if raw.event.response.completion != Completion::Eof || !raw.capture_complete {
        return;
    }
    if raw.event.response.content_encoding != "identity" {
        raw.event.issues.push("encoded_response_not_analyzed".into());
        return;
    }
    let body = raw.chunks.iter().flat_map(|bytes| bytes.iter().copied()).collect::<Vec<_>>();
    if body.is_empty() && raw.event.calls.iter().all(|call| call.id.is_none()) {
        return;
    }
    let Ok(value) = serde_json::from_slice::<Value>(&body) else {
        raw.event.issues.push("invalid_response_json".into());
        for call in &mut raw.event.calls {
            if call.id.is_some() {
                call.response = RpcResponse::Malformed;
            }
        }
        return;
    };
    let call = &mut raw.event.calls[0];
    let Some(id) = &call.id else { return };
    if !value.is_object() {
        call.response = RpcResponse::Malformed;
        raw.event.issues.push("invalid_response_shape".into());
    } else if value.get("id") != Some(id) {
        call.response = RpcResponse::Missing;
        raw.event.issues.push("unknown_response_id".into());
    } else if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || value.get("result").is_some() == value.get("error").is_some()
    {
        call.response = RpcResponse::Malformed;
    } else if let Some(result) = value.get("result") {
        call.response = if result.is_null() { RpcResponse::Null } else { RpcResponse::Result };
        call.result_json_bytes = Some(serde_json::to_vec(result).unwrap().len() as u64);
    } else {
        call.error_code = value["error"]["code"].as_i64();
        call.response = if call.error_code.is_some() && value["error"]["message"].is_string() {
            RpcResponse::Error
        } else {
            RpcResponse::Malformed
        };
    }
}

fn snapshot<'a>(events: impl Iterator<Item = &'a RpcEvent>, analyzed: bool) -> Snapshot {
    let mut snapshot = Snapshot { response_analysis_complete: analyzed, ..Snapshot::default() };
    for event in events {
        snapshot.client_http_exchanges += 1;
        snapshot.upstream_http_exchanges += event.upstream_http_exchanges;
        snapshot.client_response_body_bytes += event.response.body_bytes;
        snapshot.upstream_response_body_bytes +=
            event.upstream.as_ref().map_or(0, |body| body.body_bytes);
        snapshot.active_http_exchanges +=
            u64::from(event.response.completion == Completion::Pending);
        snapshot.errors += u64::from(
            event.issues.iter().any(|issue| {
                !matches!(
                    issue.as_str(),
                    "response_analysis_body_limit" | "encoded_response_not_analyzed"
                )
            }) || matches!(event.response.completion, Completion::TransportError),
        );
        for call in &event.calls {
            *snapshot.client_requests_by_method.entry(call.method.clone()).or_default() += 1;
            if call.forwarded && event.upstream_http_exchanges != 0 {
                *snapshot.upstream_requests_by_method.entry(call.method.clone()).or_default() += 1;
            }
            snapshot.injected_responses += u64::from(call.injected && call.id.is_some());
            snapshot.repeated_requests += u64::from(call.repeated);
            snapshot.errors += u64::from(matches!(
                call.response,
                RpcResponse::Error
                    | RpcResponse::Malformed
                    | RpcResponse::Missing
                    | RpcResponse::Ambiguous
            ));
        }
    }
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use foundry_common::sh_println;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        sync::oneshot,
        time::timeout,
    };

    fn request(method: &str, id: Value) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":[]})
    }

    #[tokio::test]
    async fn ordinary_responses_are_metered_without_retaining_bodies() {
        let payload = Bytes::from(vec![b'x'; 1024 * 1024]);
        let upstream_payload = payload.clone();
        let upstream = Router::new().route(
            "/",
            post(move || {
                let payload = upstream_payload.clone();
                async move { payload }
            }),
        );
        let (url, task) = serve(upstream).await;
        let proxy = Proxy::start(&url, Policy::Passthrough).await.unwrap();
        let client = reqwest::Client::new();
        for id in 0..3 {
            let response = client
                .post(proxy.url())
                .json(&request("eth_getCode", json!(id)))
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
            assert_eq!(response, payload);
        }
        let retained = proxy
            .state
            .ledger
            .lock()
            .unwrap()
            .events
            .iter()
            .flat_map(|raw| &raw.chunks)
            .map(Bytes::len)
            .sum::<usize>();
        assert_eq!(retained, 0, "ordinary response bodies must not accumulate until finish");
        let session = proxy.finish().await;
        assert_eq!(session.snapshot.client_requests_by_method["eth_getCode"], 3);
        assert_eq!(session.snapshot.upstream_requests_by_method["eth_getCode"], 3);
        assert_eq!(session.snapshot.client_response_body_bytes, 3 * payload.len() as u64);
        assert_eq!(session.snapshot.upstream_response_body_bytes, 3 * payload.len() as u64);
        assert_eq!(session.snapshot.repeated_requests, 2);
        assert_eq!(session.snapshot.errors, 0);
        for event in &session.events {
            assert_eq!(event.response.completion, Completion::Eof);
            assert_eq!(event.upstream.as_ref().unwrap().completion, Completion::Eof);
            assert_eq!(event.calls[0].response, RpcResponse::Unobserved);
            assert_eq!(event.calls[0].result_json_bytes, None);
            assert!(event.issues.is_empty());
        }
        task.abort();
    }

    #[tokio::test]
    async fn batches_pass_through_unchanged_and_count_body_once() {
        let response_body = Bytes::from_static(
            br#"[ {"jsonrpc":"2.0", "id":"b", "error":{"code":-32000,"message":"fixture"}},
                 {"jsonrpc":"2.0", "id":1, "result":null} ]"#,
        );
        for (policy, method) in [
            (Policy::MethodNotFound, "eth_getCode"),
            (Policy::Passthrough, "eth_getBlockAccessListByBlockHash"),
        ] {
            let request_body = Bytes::from(format!(
                r#"[ {{"jsonrpc":"2.0","id":1,"method":"{method}","params":[]}},
                     {{"jsonrpc":"2.0","id":"b","method":"eth_chainId","params":[]}} ]"#,
            ));
            let expected_request = request_body.clone();
            let upstream_body = response_body.clone();
            let upstream = Router::new().route(
                "/",
                post(move |body: Bytes| {
                    assert_eq!(body, expected_request);
                    let body = upstream_body.clone();
                    async move { body }
                }),
            );
            let (url, task) = serve(upstream).await;
            let proxy = Proxy::start(&url, policy).await.unwrap();
            let response = reqwest::Client::new()
                .post(proxy.url())
                .body(request_body.clone())
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
            assert_eq!(response, response_body);
            assert!(proxy.state.ledger.lock().unwrap().events[0].chunks.is_empty());
            let session = proxy.finish().await;
            assert_eq!(session.snapshot.client_http_exchanges, 1);
            assert_eq!(session.snapshot.upstream_http_exchanges, 1);
            assert_eq!(session.snapshot.client_requests_by_method[method], 1);
            assert_eq!(session.snapshot.upstream_requests_by_method["eth_chainId"], 1);
            assert_eq!(session.snapshot.client_response_body_bytes, response.len() as u64);
            assert_eq!(session.snapshot.upstream_response_body_bytes, response.len() as u64);
            assert_eq!(session.snapshot.injected_responses, 0);
            assert_eq!(session.snapshot.errors, 0);
            let event = &session.events[0];
            assert!(event.batch);
            assert_eq!(event.client_request_body_bytes, request_body.len() as u64);
            assert_eq!(event.upstream_request_body_bytes, request_body.len() as u64);
            assert_eq!(event.response.completion, Completion::Eof);
            for call in &event.calls {
                assert_eq!(call.response, RpcResponse::Unobserved);
                assert_eq!(call.error_code, None);
                assert_eq!(call.result_json_bytes, None);
            }
            task.abort();
        }
    }

    #[tokio::test]
    async fn bal_batches_cannot_silently_bypass_injection() {
        for policy in [
            Policy::MethodNotFound,
            Policy::Null,
            Policy::Recorded(json!([])),
            Policy::Delay { millis: 60_000, result: None },
        ] {
            for body in [
                json!([request("eth_getBlockAccessListByBlockHash", json!(4))]),
                json!([
                    request("eth_getBlockAccessListByBlockHash", json!(4)),
                    request("eth_chainId", json!("other")),
                ]),
            ] {
                let proxy = Proxy::start("http://127.0.0.1:1", policy.clone()).await.unwrap();
                let response = reqwest::Client::new()
                    .post(proxy.url())
                    .timeout(Duration::from_secs(2))
                    .json(&body)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::BAD_REQUEST);
                response.bytes().await.unwrap();
                let session = proxy.finish().await;
                assert_eq!(session.snapshot.client_http_exchanges, 1);
                assert_eq!(session.snapshot.upstream_http_exchanges, 0);
                assert_eq!(session.snapshot.injected_responses, 0);
                assert_eq!(session.snapshot.errors, 1);
                assert_eq!(session.events[0].issues, ["unsupported_bal_batch_injection"]);
                assert!(
                    session.events[0].calls.iter().all(|call| !call.forwarded && !call.injected)
                );
            }
        }
    }

    #[tokio::test]
    async fn null_and_recorded_policies_never_contact_upstream() {
        for (policy, result) in [
            (Policy::Null, Value::Null),
            (Policy::Recorded(json!({"bad":"bal"})), json!({"bad":"bal"})),
        ] {
            let proxy = Proxy::start("http://127.0.0.1:1/private-token", policy).await.unwrap();
            let response = reqwest::Client::new()
                .post(proxy.url())
                .json(&request("eth_getBlockAccessListByBlockHash", json!(7)))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap();
            assert_eq!(response, json!({"jsonrpc":"2.0","id":7,"result":result}));
            let session = proxy.finish().await;
            assert_eq!(session.snapshot.client_http_exchanges, 1);
            assert_eq!(session.snapshot.upstream_http_exchanges, 0);
            assert_eq!(session.snapshot.injected_responses, 1);
            assert!(session.events[0].upstream.is_none());
            assert_eq!(
                session.events[0].calls[0].result_json_bytes,
                Some(result.to_string().len() as u64)
            );
            assert!(!serde_json::to_string(&session).unwrap().contains("private-token"));
        }
    }

    #[tokio::test]
    async fn standalone_bal_response_classification() {
        for (body, response, error_code, issue) in [
            (json!({"jsonrpc":"2.0","id":1,"result":[]}), RpcResponse::Result, None, None),
            (json!({"jsonrpc":"2.0","id":1,"result":null}), RpcResponse::Null, None, None),
            (
                json!({"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"unavailable"}}),
                RpcResponse::Error,
                Some(-32601),
                None,
            ),
            (json!({"jsonrpc":"2.0","id":1,"error":"invalid"}), RpcResponse::Malformed, None, None),
            (
                json!({"jsonrpc":"2.0","id":99,"result":0}),
                RpcResponse::Missing,
                None,
                Some("unknown_response_id"),
            ),
            (
                json!([{"jsonrpc":"2.0","id":1,"result":0}]),
                RpcResponse::Malformed,
                None,
                Some("invalid_response_shape"),
            ),
            (
                json!({"jsonrpc":"2.0","id":1,"result":0,"error":{"code":-1,"message":"bad"}}),
                RpcResponse::Malformed,
                None,
                None,
            ),
        ] {
            let upstream = Router::new().route(
                "/",
                post(move || {
                    let body = body.clone();
                    async move { Json(body) }
                }),
            );
            let (url, task) = serve(upstream).await;
            let proxy = Proxy::start(&url, Policy::Passthrough).await.unwrap();
            reqwest::Client::new()
                .post(proxy.url())
                .json(&request("eth_getBlockAccessListByBlockHash", json!(1)))
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
            let session = proxy.finish().await;
            let call = &session.events[0].calls[0];
            assert_eq!(call.response, response);
            assert_eq!(call.error_code, error_code);
            assert_eq!(session.events[0].issues, issue.into_iter().collect::<Vec<_>>());
            if matches!(response, RpcResponse::Result | RpcResponse::Null) {
                assert!(call.result_json_bytes.is_some());
            } else {
                assert!(call.result_json_bytes.is_none());
            }
            task.abort();
        }
    }

    #[tokio::test]
    async fn invalid_json_and_http_failure_are_distinct() {
        let upstream = Router::new().route(
            "/",
            post(|| async { (StatusCode::SERVICE_UNAVAILABLE, "temporarily unavailable") }),
        );
        let (url, task) = serve(upstream).await;
        let proxy = Proxy::start(&url, Policy::Passthrough).await.unwrap();
        let response = reqwest::Client::new()
            .post(proxy.url())
            .json(&request("eth_getBlockAccessListByBlockHash", json!(1)))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.text().await.unwrap(), "temporarily unavailable");
        let session = proxy.finish().await;
        assert_eq!(session.events[0].issues, ["upstream_http_error", "invalid_response_json"]);
        assert_eq!(session.events[0].response.completion, Completion::Eof);
        assert_eq!(session.events[0].calls[0].response, RpcResponse::Malformed);
        task.abort();
    }

    #[tokio::test]
    async fn passthrough_preserves_retry_headers_without_hop_headers() {
        let upstream = Router::new().route(
            "/",
            post(|| async {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    [
                        ("retry-after", "3"),
                        ("connection", "x-fixture-hop"),
                        ("x-fixture-hop", "local"),
                    ],
                    "slow down",
                )
            }),
        );
        let (url, task) = serve(upstream).await;
        let proxy = Proxy::start(&url, Policy::Passthrough).await.unwrap();
        let response = reqwest::Client::new()
            .post(proxy.url())
            .json(&request("eth_chainId", json!(1)))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::RETRY_AFTER], "3");
        assert!(!response.headers().contains_key("x-fixture-hop"));
        assert!(!response.headers().contains_key(header::CONTENT_LENGTH));
        assert_eq!(response.text().await.unwrap(), "slow down");
        let session = proxy.finish().await;
        assert_eq!(session.snapshot.upstream_http_exchanges, 1);
        task.abort();
    }

    #[tokio::test]
    async fn encoded_bytes_are_forwarded_without_claiming_decoded_sizes() {
        let upstream = Router::new().route(
            "/",
            post(|| async {
                ([(header::CONTENT_ENCODING, "gzip")], Bytes::from_static(b"encoded fixture"))
            }),
        );
        let (url, task) = serve(upstream).await;
        let proxy = Proxy::start(&url, Policy::Passthrough).await.unwrap();
        let response = reqwest::Client::builder()
            .no_gzip()
            .build()
            .unwrap()
            .post(proxy.url())
            .json(&request("eth_getBlockAccessListByBlockHash", json!(1)))
            .send()
            .await
            .unwrap();
        assert_eq!(response.headers()[header::CONTENT_ENCODING], "gzip");
        assert_eq!(response.bytes().await.unwrap(), Bytes::from_static(b"encoded fixture"));
        let session = proxy.finish().await;
        assert_eq!(session.events[0].response.content_encoding, "gzip");
        assert_eq!(session.events[0].response.body_bytes, 15);
        assert_eq!(session.events[0].calls[0].response, RpcResponse::Unobserved);
        assert!(session.events[0].calls[0].result_json_bytes.is_none());
        assert_eq!(session.events[0].issues, ["encoded_response_not_analyzed"]);
        assert_eq!(session.snapshot.errors, 0);
        task.abort();
    }

    #[tokio::test]
    async fn partial_upstream_body_keeps_observed_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream = format!("http://{}", listener.local_addr().unwrap());
        let (close, ready) = oneshot::channel();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nContent-Type: application/json\r\n\r\n{\"res").await.unwrap();
            socket.flush().await.unwrap();
            ready.await.unwrap();
            socket.shutdown().await.unwrap();
        });
        let proxy = Proxy::start(&upstream, Policy::Passthrough).await.unwrap();
        let mut response = reqwest::Client::new()
            .post(proxy.url())
            .json(&request("eth_getBlockAccessListByBlockHash", json!(1)))
            .send()
            .await
            .unwrap();
        assert_eq!(response.chunk().await.unwrap().unwrap(), Bytes::from_static(b"{\"res"));
        close.send(()).unwrap();
        assert!(response.chunk().await.is_err());
        let session = proxy.finish().await;
        assert_eq!(session.events[0].response.completion, Completion::TransportError);
        assert_eq!(session.events[0].response.body_bytes, 5);
        assert_eq!(session.events[0].upstream.as_ref().unwrap().body_bytes, 5);
        assert_eq!(
            session.events[0].upstream.as_ref().unwrap().completion,
            Completion::TransportError
        );
        assert_eq!(session.events[0].calls[0].response, RpcResponse::Unobserved);
        assert!(session.events[0].calls[0].result_json_bytes.is_none());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn client_disconnect_is_observed_before_cleanup() {
        let upstream = Router::new().route(
            "/",
            post(|| async {
                Body::from_stream(
                    futures::stream::iter([Ok::<_, io::Error>(Bytes::from_static(b"partial"))])
                        .chain(futures::stream::pending()),
                )
            }),
        );
        let (url, task) = serve(upstream).await;
        let proxy = Proxy::start(&url, Policy::Passthrough).await.unwrap();
        let mut response = reqwest::Client::new()
            .post(proxy.url())
            .json(&request("eth_getBlockAccessListByBlockHash", json!(1)))
            .send()
            .await
            .unwrap();
        assert_eq!(response.chunk().await.unwrap().unwrap(), Bytes::from_static(b"partial"));
        drop(response);
        wait_for(&proxy, |snapshot| snapshot.active_http_exchanges == 0).await;
        let session = proxy.finish().await;
        assert_eq!(session.events[0].response.completion, Completion::Dropped);
        assert!(!session.events[0].response.completed_during_cleanup);
        assert_eq!(session.events[0].response.body_bytes, 7);
        assert_eq!(session.events[0].upstream.as_ref().unwrap().completion, Completion::Dropped);
        task.abort();
    }

    #[tokio::test]
    async fn bytes_after_measurement_boundary_are_cleanup_even_before_finish() {
        for method in ["eth_getCode", "eth_getBlockAccessListByBlockHash"] {
            let (release, delayed) = oneshot::channel();
            let delayed = Arc::new(Mutex::new(Some(delayed)));
            let upstream = Router::new().route(
                "/",
                post(move || {
                    let delayed = delayed.lock().unwrap().take().unwrap();
                    async move {
                        let first = futures::stream::once(async {
                            Ok::<_, io::Error>(Bytes::from_static(b"first"))
                        });
                        let second = futures::stream::once(async move {
                            delayed.await.unwrap();
                            Ok::<_, io::Error>(Bytes::from_static(b"late"))
                        });
                        Body::from_stream(first.chain(second))
                    }
                }),
            );
            let (url, task) = serve(upstream).await;
            let proxy = Proxy::start(&url, Policy::Passthrough).await.unwrap();
            let mut response = reqwest::Client::new()
                .post(proxy.url())
                .json(&request(method, json!(1)))
                .send()
                .await
                .unwrap();
            assert_eq!(response.chunk().await.unwrap().unwrap(), Bytes::from_static(b"first"));
            let at_exit = proxy.end_measurement();
            assert_eq!(at_exit.client_response_body_bytes, 5);
            release.send(()).unwrap();
            assert_eq!(response.bytes().await.unwrap(), Bytes::from_static(b"late"));
            let session = proxy.finish().await;
            assert_eq!(session.events[0].response.body_bytes, 9);
            assert_eq!(session.events[0].response.cleanup_body_bytes, 4);
            assert_eq!(session.events[0].upstream.as_ref().unwrap().cleanup_body_bytes, 4);
            assert!(session.events[0].response.completed_during_cleanup);
            assert_eq!(session.events[0].response.completion, Completion::Eof);
            task.abort();
        }
    }

    #[tokio::test]
    async fn cleanup_cancels_delayed_handlers_and_sessions_do_not_mix() {
        let proxy = Proxy::start(
            "http://127.0.0.1:1",
            Policy::Delay { millis: 60_000, result: Some(Value::Null) },
        )
        .await
        .unwrap();
        let url = proxy.url().to_owned();
        let request_task = tokio::spawn(async move {
            reqwest::Client::new()
                .post(url)
                .json(&request("eth_getBlockAccessListByBlockHash", json!(1)))
                .send()
                .await
        });
        wait_for(&proxy, |snapshot| {
            snapshot.client_requests_by_method.contains_key("eth_getBlockAccessListByBlockHash")
        })
        .await;
        assert_eq!(proxy.snapshot().active_http_exchanges, 1);
        let session = timeout(Duration::from_secs(2), proxy.finish()).await.unwrap();
        assert_eq!(session.events[0].response.completion, Completion::Cancelled);
        assert!(session.events[0].response.completed_during_cleanup);
        assert_eq!(session.snapshot.upstream_http_exchanges, 0);
        assert_eq!(session.snapshot.injected_responses, 0);
        assert_eq!(session.snapshot.active_http_exchanges, 0);
        let next = Proxy::start("http://127.0.0.1:1", Policy::Null).await.unwrap();
        assert_eq!(next.snapshot().client_http_exchanges, 0);
        assert!(next.finish().await.events.is_empty());
        let _ = request_task.await;
    }

    /// Observe loopback transport overhead without subtracting it from application measurements.
    #[tokio::test]
    #[ignore = "manual loopback overhead measurement; no performance threshold"]
    async fn proxy_overhead() {
        let payload = Bytes::from(
            serde_json::to_vec(&json!({
                "jsonrpc":"2.0", "id":1, "result":"ab".repeat(4096),
            }))
            .unwrap(),
        );
        let fixture_payload = payload.clone();
        let upstream = Router::new().route(
            "/",
            post(move || {
                let payload = fixture_payload.clone();
                async move { ([(header::CONTENT_TYPE, "application/json")], payload) }
            }),
        );
        let (url, task) = serve(upstream).await;
        let proxy = Proxy::start(&url, Policy::Passthrough).await.unwrap();
        let client = reqwest::Client::new();
        let body =
            serde_json::to_vec(&request("eth_getBlockAccessListByBlockHash", json!(1))).unwrap();
        let mut direct = Vec::new();
        let mut proxied = Vec::new();
        for round in 0..32 {
            let order = if round % 2 == 0 { [false, true] } else { [true, false] };
            for use_proxy in order {
                let endpoint = if use_proxy { proxy.url() } else { &url };
                let start = Instant::now();
                let response = client
                    .post(endpoint)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(body.clone())
                    .send()
                    .await
                    .unwrap()
                    .bytes()
                    .await
                    .unwrap();
                let elapsed = start.elapsed().as_secs_f64();
                assert_eq!(response, payload);
                if round >= 2 {
                    if use_proxy {
                        proxied.push(elapsed);
                    } else {
                        direct.push(elapsed);
                    }
                }
            }
        }
        let session = proxy.finish().await;
        assert_eq!(session.snapshot.client_http_exchanges, 32);
        assert_eq!(session.snapshot.upstream_http_exchanges, 32);
        assert_eq!(session.snapshot.client_response_body_bytes, 32 * payload.len() as u64);
        let direct_summary = timing_summary(&direct);
        let proxied_summary = timing_summary(&proxied);
        sh_println!(
            "{}",
            json!({
                "schema_version":1, "kind":"loopback_proxy_overhead", "warmup_rounds":2,
                "measured_rounds":30, "order":"alternating_direct_proxy",
                "request_body_bytes":body.len(), "response_body_bytes":payload.len(),
                "direct_seconds":direct_summary, "proxied_seconds":proxied_summary,
                "median_increment_seconds":proxied_summary["median"].as_f64().unwrap()
                    - direct_summary["median"].as_f64().unwrap(),
                "application_time_subtraction":false,
            })
        )
        .unwrap();
        task.abort();
    }

    fn timing_summary(samples: &[f64]) -> Value {
        let mut sorted = samples.to_vec();
        sorted.sort_by(f64::total_cmp);
        let quantile = |fraction: f64| {
            let position = fraction * (sorted.len() - 1) as f64;
            let lower = position.floor() as usize;
            let upper = position.ceil() as usize;
            (sorted[upper] - sorted[lower]).mul_add(position.fract(), sorted[lower])
        };
        json!({"median":quantile(0.5), "iqr":quantile(0.75)-quantile(0.25),
            "min":sorted[0], "max":sorted[sorted.len()-1], "samples":samples})
    }

    async fn wait_for(proxy: &Proxy, predicate: impl Fn(&Snapshot) -> bool) {
        timeout(Duration::from_secs(2), async {
            loop {
                if predicate(&proxy.snapshot()) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn serve(router: Router) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (url, task)
    }
}
