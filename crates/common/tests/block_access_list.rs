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
use foundry_common::provider::block_access_list::{
    BlockAccessListError, BlockAccessListOutcome, fetch_block_access_list,
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tower::Service;

#[derive(Clone)]
struct RecordingTransport {
    inner: MockTransport,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    errors: Arc<Mutex<VecDeque<TransportError>>>,
}

impl RecordingTransport {
    fn new(asserter: Asserter) -> Self {
        Self {
            inner: MockTransport::new(asserter),
            requests: Default::default(),
            errors: Default::default(),
        }
    }

    fn record(&self, request: &SerializedRequest) {
        let params = serde_json::from_str(request.params().unwrap().get()).unwrap();
        self.requests.lock().unwrap().push((request.method().to_string(), params));
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
        self.inner.call(request)
    }
}

#[tokio::test]
async fn probes_exact_method_and_block_without_activation_requests() {
    let asserter = Asserter::new();
    let transport = RecordingTransport::new(asserter.clone());
    let provider = RootProvider::<Ethereum>::new(RpcClient::new(transport.clone(), true));
    let hash = B256::repeat_byte(0x11);
    let cases = [
        (BlockId::number(20_000_000), json!(["0x1312d00"])),
        (BlockId::hash(hash), json!([hash])),
        (BlockId::hash_canonical(hash), json!([{"blockHash": hash, "requireCanonical": true}])),
    ];
    let mut expected = Vec::new();

    for (block, params) in cases {
        asserter.push_success(&json!([]));
        assert_eq!(
            fetch_block_access_list(&provider, block).await.unwrap(),
            BlockAccessListOutcome::Available(Vec::new())
        );
        expected.push(("eth_getBlockAccessList".to_string(), params));
        assert_eq!(*transport.requests.lock().unwrap(), expected);
        assert!(asserter.read_q().is_empty());
    }
}

#[tokio::test]
async fn transport_failure_does_not_disable_later_requests() {
    let asserter = Asserter::new();
    asserter.push_success(&json!([]));
    let transport = RecordingTransport::new(asserter);
    transport.errors.lock().unwrap().push_back(TransportErrorKind::backend_gone());
    let provider = RootProvider::<Ethereum>::new(RpcClient::new(transport.clone(), true));
    let block = BlockId::number(20_000_000);

    assert!(matches!(
        fetch_block_access_list(&provider, block).await,
        Err(BlockAccessListError::Request(TransportError::Transport(
            TransportErrorKind::BackendGone
        )))
    ));
    assert_eq!(
        fetch_block_access_list(&provider, block).await.unwrap(),
        BlockAccessListOutcome::Available(Vec::new())
    );
    assert_eq!(
        *transport.requests.lock().unwrap(),
        vec![("eth_getBlockAccessList".to_string(), json!(["0x1312d00"])); 2]
    );
}

#[tokio::test]
async fn classifies_http_rpc_errors_without_disabling_provider() {
    let cases = [
        (
            403,
            concat!(
                r#"{"jsonrpc":"2.0","error":{"code":-32601,"message":"method not found"}}"#,
                "\n\nHTTP diagnostics:\nstatus: 403 Forbidden"
            ),
            BlockAccessListOutcome::Unsupported,
        ),
        (
            404,
            r#"{"jsonrpc":"2.0","error":{"code":-32001,"message":"block not found"}}"#,
            BlockAccessListOutcome::Unavailable,
        ),
    ];

    for (status, body, expected) in cases {
        let asserter = Asserter::new();
        asserter.push_success(&json!([]));
        let transport = RecordingTransport::new(asserter);
        transport
            .errors
            .lock()
            .unwrap()
            .push_back(TransportErrorKind::http_error(status, body.to_string()));
        let provider = RootProvider::<Ethereum>::new(RpcClient::new(transport.clone(), true));
        let block = BlockId::number(20_000_000);

        assert_eq!(fetch_block_access_list(&provider, block).await.unwrap(), expected);
        assert_eq!(
            fetch_block_access_list(&provider, block).await.unwrap(),
            BlockAccessListOutcome::Available(Vec::new())
        );
        assert_eq!(transport.requests.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn http_rate_limit_remains_a_request_error() {
    let asserter = Asserter::new();
    asserter.push_success(&json!([]));
    let transport = RecordingTransport::new(asserter);
    transport
        .errors
        .lock()
        .unwrap()
        .push_back(TransportErrorKind::http_error(429, "Too many requests".to_string()));
    let provider = RootProvider::<Ethereum>::new(RpcClient::new(transport.clone(), true));
    let block = BlockId::number(20_000_000);

    let error = fetch_block_access_list(&provider, block).await.unwrap_err();
    let BlockAccessListError::Request(TransportError::Transport(error)) = error else {
        panic!("expected the HTTP request error, got {error:?}");
    };
    let http = error.as_http_error().unwrap();
    assert_eq!(http.status, 429);
    assert_eq!(http.body, "Too many requests");
    assert_eq!(
        fetch_block_access_list(&provider, block).await.unwrap(),
        BlockAccessListOutcome::Available(Vec::new())
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
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
    let outcome = fetch_block_access_list(&provider, BlockId::number(20_000_000)).await.unwrap();
    assert_eq!(outcome, BlockAccessListOutcome::Available(expected));
}

#[tokio::test]
async fn missing_block_access_list_does_not_disable_later_requests() {
    let asserter = Asserter::new();
    asserter.push_success(&serde_json::Value::Null);
    asserter.push_success(&json!([]));
    let provider = ProviderBuilder::new().connect_mocked_client(asserter);

    assert_eq!(
        fetch_block_access_list(&provider, BlockId::number(20_000_000)).await.unwrap(),
        BlockAccessListOutcome::Unavailable
    );
    assert_eq!(
        fetch_block_access_list(&provider, BlockId::number(20_000_001)).await.unwrap(),
        BlockAccessListOutcome::Available(vec![])
    );
}

#[tokio::test]
async fn distinguishes_rpc_errors_without_disabling_later_requests() {
    for code in [-32601, -32001, -32602, -32603, -32000, -32005] {
        let asserter = Asserter::new();
        // Classification must use the code, not a provider-specific message.
        asserter.push_failure(ErrorPayload {
            code,
            message: "block access list unavailable".into(),
            data: None,
        });
        asserter.push_success(&json!([]));
        let provider = ProviderBuilder::new().connect_mocked_client(asserter);

        let result = fetch_block_access_list(&provider, BlockId::number(20_000_000)).await;
        match code {
            -32601 => assert_eq!(result.unwrap(), BlockAccessListOutcome::Unsupported),
            -32001 => assert_eq!(result.unwrap(), BlockAccessListOutcome::Unavailable),
            _ => {
                let BlockAccessListError::Request(error) = result.unwrap_err() else {
                    panic!("expected a request error for code {code}");
                };
                assert_eq!(error.as_error_resp().unwrap().code, code);
            }
        }
        assert_eq!(
            fetch_block_access_list(&provider, BlockId::number(20_000_001)).await.unwrap(),
            BlockAccessListOutcome::Available(vec![])
        );
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
        asserter.push_success(&json!([]));
        let provider = ProviderBuilder::new().connect_mocked_client(asserter);

        let error =
            fetch_block_access_list(&provider, BlockId::number(20_000_000)).await.unwrap_err();
        assert!(matches!(error, BlockAccessListError::InvalidResponse(_)), "{error:?}");
        assert_eq!(
            fetch_block_access_list(&provider, BlockId::number(20_000_001)).await.unwrap(),
            BlockAccessListOutcome::Available(vec![])
        );
    }
}
