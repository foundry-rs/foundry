//! BAL provider requests, error recovery and timeout boundaries.

use alloy_eips::{BlockId, eip7928::BlockAccessList};
use alloy_json_rpc::{ErrorPayload, RequestPacket, ResponsePacket, SerializedRequest};
use alloy_network::Ethereum;
use alloy_primitives::B256;
use alloy_provider::{ProviderBuilder, RootProvider};
use alloy_rpc_client::RpcClient;
use alloy_transport::{
    TransportError, TransportErrorKind, TransportFut,
    mock::{Asserter, MockTransport},
};
use foundry_common::provider::block_access_list::fetch_block_access_list;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fmt::Debug,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::time::{Instant, sleep};
use tower::Service;

#[derive(Clone)]
struct RecordingTransport {
    inner: MockTransport,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    errors: Arc<Mutex<VecDeque<TransportError>>>,
    delays: Arc<Mutex<VecDeque<Duration>>>,
}

impl RecordingTransport {
    fn new(asserter: Asserter) -> Self {
        Self {
            inner: MockTransport::new(asserter),
            requests: Default::default(),
            errors: Default::default(),
            delays: Default::default(),
        }
    }

    fn record(&self, request: &SerializedRequest) {
        let params = serde_json::from_str(request.params().unwrap().get()).unwrap();
        self.requests.lock().unwrap().push((request.method().to_string(), params));
    }

    fn provider(&self) -> RootProvider<Ethereum> {
        RootProvider::new(RpcClient::new(self.clone(), true))
    }

    async fn assert_recovers(&self, next_block: u64, case: impl Debug) {
        self.inner.push_success(&json!([]));
        let provider = self.provider();

        assert_eq!(
            fetch_block_access_list(&provider, BlockId::number(20_000_000)).await,
            None,
            "case: {case:?}"
        );
        assert_eq!(
            fetch_block_access_list(&provider, BlockId::number(next_block)).await,
            Some(vec![]),
            "case: {case:?}"
        );
        assert_eq!(
            *self.requests.lock().unwrap(),
            [20_000_000, next_block].map(|block| {
                ("eth_getBlockAccessListByBlockNumber".to_string(), json!([format!("0x{block:x}")]))
            }),
            "case: {case:?}"
        );
        assert!(self.inner.read_q().is_empty(), "case: {case:?}");
    }
}

impl Service<RequestPacket> for RecordingTransport {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: RequestPacket) -> Self::Future {
        match &request {
            RequestPacket::Single(request) => self.record(request),
            RequestPacket::Batch(requests) => {
                for request in requests {
                    self.record(request);
                }
            }
        }
        if let Some(error) = self.errors.lock().unwrap().pop_front() {
            return Box::pin(async move { Err(error) });
        }
        let response = self.inner.call(request);
        let delay = self.delays.lock().unwrap().pop_front();
        Box::pin(async move {
            if let Some(delay) = delay {
                sleep(delay).await;
            }
            response.await
        })
    }
}

#[tokio::test]
async fn uses_alloy_block_access_list_methods_without_activation_requests() {
    let asserter = Asserter::new();
    let transport = RecordingTransport::new(asserter.clone());
    let provider = transport.provider();
    let hash = B256::repeat_byte(0x11);
    let cases = [
        (BlockId::number(20_000_000), "eth_getBlockAccessListByBlockNumber", json!(["0x1312d00"])),
        (BlockId::hash(hash), "eth_getBlockAccessListByBlockHash", json!([hash])),
    ];
    let mut expected = Vec::new();

    for (block, method, params) in cases {
        asserter.push_success(&json!([]));
        assert_eq!(fetch_block_access_list(&provider, block).await, Some(Vec::new()));
        expected.push((method.to_string(), params));
        assert_eq!(*transport.requests.lock().unwrap(), expected);
        assert!(asserter.read_q().is_empty());
    }
}

#[tokio::test]
async fn canonical_hash_requirement_does_not_get_silently_dropped() {
    let asserter = Asserter::new();
    asserter.push_success(&json!([]));
    let transport = RecordingTransport::new(asserter.clone());
    let provider = transport.provider();
    let hash = B256::repeat_byte(0x11);

    assert_eq!(fetch_block_access_list(&provider, BlockId::hash_canonical(hash)).await, None);
    assert!(transport.requests.lock().unwrap().is_empty());
    assert_eq!(fetch_block_access_list(&provider, BlockId::hash(hash)).await, Some(vec![]));
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    assert!(asserter.read_q().is_empty());
}

#[tokio::test]
async fn transport_failure_does_not_disable_later_requests() {
    let transport = RecordingTransport::new(Asserter::new());
    transport.errors.lock().unwrap().push_back(TransportErrorKind::backend_gone());
    transport.assert_recovers(20_000_000, "backend gone").await;
}

#[tokio::test(start_paused = true)]
async fn response_before_timeout_is_returned() {
    let asserter = Asserter::new();
    asserter.push_success(&json!([]));
    let transport = RecordingTransport::new(asserter);
    transport.delays.lock().unwrap().push_back(Duration::from_millis(499));
    let provider = transport.provider();
    let start = Instant::now();

    assert_eq!(fetch_block_access_list(&provider, BlockId::number(20_000_000)).await, Some(vec![]));
    assert_eq!(start.elapsed(), Duration::from_millis(499));
}

#[tokio::test(start_paused = true)]
async fn request_timeout_does_not_disable_later_requests() {
    let asserter = Asserter::new();
    asserter.push_success(&json!([]));
    let transport = RecordingTransport::new(asserter.clone());
    transport.delays.lock().unwrap().push_back(Duration::from_secs(60));
    let provider = transport.provider();
    let block = BlockId::number(20_000_000);
    let start = Instant::now();

    assert_eq!(fetch_block_access_list(&provider, block).await, None);
    assert_eq!(start.elapsed(), Duration::from_millis(500));
    assert_eq!(fetch_block_access_list(&provider, block).await, Some(vec![]));
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
    assert!(asserter.read_q().is_empty());
}

#[tokio::test]
async fn http_errors_do_not_disable_later_requests() {
    let cases = [
        (
            403,
            concat!(
                r#"{"jsonrpc":"2.0","error":{"code":-32601,"message":"method not found"}}"#,
                "\n\nHTTP diagnostics:\nstatus: 403 Forbidden"
            ),
        ),
        (404, r#"{"jsonrpc":"2.0","error":{"code":-32001,"message":"block not found"}}"#),
        (429, "Too many requests"),
    ];

    for (status, body) in cases {
        let transport = RecordingTransport::new(Asserter::new());
        transport
            .errors
            .lock()
            .unwrap()
            .push_back(TransportErrorKind::http_error(status, body.to_string()));
        transport.assert_recovers(20_000_000, (status, body)).await;
    }
}

#[tokio::test]
async fn retrieves_historical_block_access_list() {
    let response = json!([{
        "address": "0x0000000000000000000000000000000000000001",
        "storageChanges": [{"key": "0x1", "changes": [{"index": "0x1", "value": "0x2"}]}],
        "storageReads": ["0x3"],
        "balanceChanges": [{"index": "0x1", "value": "0x4"}],
        "nonceChanges": [{"index": "0x1", "value": "0x5"}],
        "codeChanges": [{"index": "0x1", "code": "0x6000"}]
    }]);
    let expected = serde_json::from_value::<BlockAccessList>(response.clone()).unwrap();
    let asserter = Asserter::new();
    asserter.push_success(&response);
    let provider = ProviderBuilder::new().connect_mocked_client(asserter);

    // Historical mainnet blocks must be probed even though Amsterdam was not active.
    let outcome = fetch_block_access_list(&provider, BlockId::number(20_000_000)).await;
    assert_eq!(outcome, Some(expected));
}

#[tokio::test]
async fn missing_block_access_list_does_not_disable_later_requests() {
    let asserter = Asserter::new();
    asserter.push_success(&serde_json::Value::Null);
    RecordingTransport::new(asserter).assert_recovers(20_000_001, Value::Null).await;
}

#[tokio::test]
async fn rpc_errors_do_not_disable_later_requests() {
    for code in [-32601, -32001, -32602, -32603, -32000, -32005] {
        let asserter = Asserter::new();
        asserter.push_failure(ErrorPayload {
            code,
            message: "block access list unavailable".into(),
            data: None,
        });
        RecordingTransport::new(asserter).assert_recovers(20_000_001, code).await;
    }
}

#[tokio::test]
async fn rejects_malformed_responses_without_disabling_later_requests() {
    let account = json!({
        "address": "0x0000000000000000000000000000000000000001",
        "storageChanges": [],
        "storageReads": [],
        "balanceChanges": [],
        "nonceChanges": [],
        "codeChanges": []
    });
    let mut invalid_address = account.clone();
    invalid_address["address"] = json!("0x01");
    let mut invalid_index = account.clone();
    invalid_index["balanceChanges"] = json!([{"index": "0x10000000000000000", "value": "0x1"}]);
    let mut invalid_balance = account.clone();
    invalid_balance["balanceChanges"] = json!([{"index": "0x1", "value": "0xinvalid"}]);
    let mut invalid_storage = account.clone();
    invalid_storage["storageChanges"] = json!([{"key": "0x1", "changes": [{}]}]);
    let mut invalid_code = account;
    invalid_code["codeChanges"] = json!([{"index": "0x1", "code": "0xgg"}]);

    for response in [
        json!({"blockAccessList": []}),
        json!("0x"),
        json!([{"address": "0x0000000000000000000000000000000000000001"}]),
        json!([invalid_address]),
        json!([invalid_index]),
        json!([invalid_balance]),
        json!([invalid_storage]),
        json!([invalid_code]),
    ] {
        let asserter = Asserter::new();
        asserter.push_success(&response);
        RecordingTransport::new(asserter).assert_recovers(20_000_001, response).await;
    }
}
