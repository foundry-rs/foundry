//! Regression tests for failed RPC request dumps and credential redaction.

use super::*;
use ::anvil::{
    NodeConfig,
    logging::{LoggingManager, NodeLogLayer},
    spawn,
};
use alloy_primitives::{B256, Bytes, U256, keccak256};
use alloy_provider::{IpcConnect, Provider, builder};
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use anvil_core::eth::EthRequest;
use foundry_test_utils::{assert_data_eq, str};
use serde_json::json;
use std::{fs, sync::Arc, time::Duration};
use tracing_subscriber::prelude::*;

#[tokio::test]
async fn http_rpc_logs_preserve_failed_requests() {
    assert_rpc_logs_preserve_failed_requests(true).await;
}

#[tokio::test]
async fn http_default_logs_identify_failed_requests() {
    assert_rpc_logs_preserve_failed_requests(false).await;
}

fn rpc_subscriber(file: &tempfile::NamedTempFile, trace: bool) -> tracing::Dispatch {
    let writer = Arc::new(file.reopen().unwrap());
    if trace {
        tracing::Dispatch::new(
            tracing_subscriber::fmt()
                .with_env_filter("off,rpc=trace,node::user=info")
                .without_time()
                .with_ansi(false)
                .with_writer(writer)
                .finish(),
        )
    } else {
        tracing::Dispatch::new(
            tracing_subscriber::registry().with(NodeLogLayer::new(LoggingManager::default())).with(
                tracing_subscriber::fmt::layer()
                    .without_time()
                    .with_ansi(false)
                    .with_target(false)
                    .with_level(false)
                    .with_writer(writer),
            ),
        )
    }
}

async fn assert_rpc_logs_preserve_failed_requests(trace: bool) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let subscriber = rpc_subscriber(&file, trace);
    let raw = tokio::time::timeout(Duration::from_secs(10), async {
        let (api, handle) =
            spawn(NodeConfig::test().with_tracing(false).with_no_mining(true)).await;
        let from = handle.dev_accounts().next().unwrap();
        let transaction = TransactionRequest::default()
            .from(from)
            .to(from)
            .gas_limit(30_000)
            .gas_price(2_000_000_000)
            .nonce(0);
        let raw = api.sign_transaction(WithOtherFields::new(transaction)).await.unwrap();
        let expected_hash = keccak256(raw.parse::<Bytes>().unwrap());
        // The current-thread runtime keeps spawned server tasks under this subscriber.
        let _subscriber = tracing::dispatcher::set_default(&subscriber);
        let provider = utils::connect_pubsub(&handle.http_endpoint()).await;
        provider
            .raw_request::<_, ()>(
                "anvil_setRpcUrl".into(),
                ("https://synthetic-user:synthetic-password@example.invalid/rpc/synthetic-api-key?key=short#synthetic-secret",),
            )
            .await
            .unwrap();
        let hash = provider
            .raw_request::<_, B256>("eth_sendRawTransaction".into(), (&raw,))
            .await
            .unwrap();
        assert_eq!(hash, expected_hash);
        let err = provider
            .raw_request::<_, B256>("eth_sendRawTransaction".into(), (&raw,))
            .await
            .unwrap_err();
        assert_eq!(err.as_error_resp().unwrap().message, "transaction already imported");
        let err = provider
            .raw_request::<_, ()>(
                "anvil_setAutomine".into(),
                ("https://example.invalid/rpc/synthetic-api-key",),
            )
            .await
            .unwrap_err();
        assert_eq!(err.as_error_resp().unwrap().code, -32602);
        let err = provider
            .raw_request::<_, Bytes>(
                "eth_call".into(),
                json!([
                    {"to": "0x1111111111111111111111111111111111111111", "data": "0x12345678"},
                    "0x100000"
                ]),
            )
            .await
            .unwrap_err();
        assert_eq!(err.as_error_resp().unwrap().code, -32602);
        let err = provider
            .raw_request::<_, ()>("unsupported_method".into(), json!([]))
            .await
            .unwrap_err();
        assert_eq!(err.as_error_resp().unwrap().code, -32601);
        assert_eq!(err.as_error_resp().unwrap().message, "Method not found");
        // Cover canonical object and aliased positional resets through the real handler.
        for (method, params) in [
            ("anvil_reset", json!([{"forking": {"jsonRpcUrl": INVALID_FORK_URL, "blockNumber": 42}}])),
            ("hardhat_reset", json!([[[INVALID_FORK_URL, "0x2b"]]])),
        ] {
            let err = provider.raw_request::<_, ()>(method.into(), params).await.unwrap_err();
            assert_eq!(err.as_error_resp().unwrap().code, -32603);
            assert_eq!(err.as_error_resp().unwrap().message, "failed to establish provider to fork url");
        }
        raw
    })
    .await
    .expect("RPC requests timed out");

    let expected = if trace {
        str![[r#"
TRACE rpc: received method call id=Number(0) method="anvil_setRpcUrl" params=Array [String("https://example.invalid/")]
TRACE rpc::api: executing eth request
 INFO node::user: anvil_setRpcUrl
TRACE rpc: received method call id=Number(1) method="eth_sendRawTransaction" params=Array [String("[SIGNED_TX]")]
TRACE rpc::api: executing eth request
 INFO node::user: eth_sendRawTransaction
TRACE rpc: received method call id=Number(2) method="eth_sendRawTransaction" params=Array [String("[SIGNED_TX]")]
TRACE rpc::api: executing eth request
 INFO node::user: eth_sendRawTransaction
 INFO node::user: RPC request failed method="eth_sendRawTransaction" id=Number(2) params=["[RAW_TX]"] error=Transaction rejected: transaction already imported
TRACE rpc: received method call id=Number(3) method="anvil_setAutomine" params=Array [String("https://example.invalid/rpc/synthetic-api-key")]
ERROR rpc: failed to deserialize method id=Number(3) method="anvil_setAutomine" err=[..]
TRACE rpc: received method call id=Number(4) method="eth_call" params=Array [Object {"to": String("0x1111111111111111111111111111111111111111"), "data": String("0x12345678")}, String("0x100000")]
TRACE rpc::api: executing eth request
 INFO node::user: eth_call
 INFO node::user: RPC request failed method="eth_call" id=Number(4) params=[{"to":"0x1111111111111111111111111111111111111111","data":"0x12345678"},"0x100000"] error=Invalid params: BlockOutOfRangeError: block height is 0 but requested was 1048576
TRACE rpc: received method call id=Number(5) method="unsupported_method" params=Array []
ERROR rpc: failed to deserialize method due to unknown variant id=Number(5) method="unsupported_method"
TRACE rpc: received method call id=Number(6) method="anvil_reset" params=Array [Object {"forking": Object {"jsonRpcUrl": String("[REDACTED_URL]"), "blockNumber": Number(42)}}]
TRACE rpc::api: executing eth request
 INFO node::user: anvil_reset
 INFO node::user: RPC request failed method="anvil_reset" id=Number(6) params=[{"forking":{"jsonRpcUrl":"[REDACTED_URL]","blockNumber":42}}] error=Internal error: failed to establish provider to fork url
TRACE rpc: received method call id=Number(7) method="hardhat_reset" params=Array [Array [Array [String("[REDACTED_URL]"), String("0x2b")]]]
TRACE rpc::api: executing eth request
 INFO node::user: anvil_reset
 INFO node::user: RPC request failed method="hardhat_reset" id=Number(7) params=[[["[REDACTED_URL]","0x2b"]]] error=Internal error: failed to establish provider to fork url

"#]]
    } else {
        str![[r#"
anvil_setRpcUrl
eth_sendRawTransaction
eth_sendRawTransaction
RPC request failed method="eth_sendRawTransaction" id=Number(2) params=["[RAW_TX]"] error=Transaction rejected: transaction already imported
eth_call
RPC request failed method="eth_call" id=Number(4) params=[{"to":"0x1111111111111111111111111111111111111111","data":"0x12345678"},"0x100000"] error=Invalid params: BlockOutOfRangeError: block height is 0 but requested was 1048576
anvil_reset
RPC request failed method="anvil_reset" id=Number(6) params=[{"forking":{"jsonRpcUrl":"[REDACTED_URL]","blockNumber":42}}] error=Internal error: failed to establish provider to fork url
anvil_reset
RPC request failed method="hardhat_reset" id=Number(7) params=[[["[REDACTED_URL]","0x2b"]]] error=Internal error: failed to establish provider to fork url

"#]]
    };
    assert_data_eq!(fs::read_to_string(file.path()).unwrap().replace(&raw, "[RAW_TX]"), expected);
}

#[tokio::test]
async fn mining_logs_omit_signed_transactions() {
    let (api, handle) = spawn(NodeConfig::test().with_tracing(false).with_no_mining(true)).await;
    let from = handle.dev_accounts().next().unwrap();
    let transaction = TransactionRequest::default()
        .from(from)
        .to(from)
        .gas_limit(30_000)
        .gas_price(2_000_000_000)
        .nonce(0);
    let raw = api.sign_transaction(WithOtherFields::new(transaction)).await.unwrap();
    let hash = api.send_raw_transaction(raw.parse().unwrap()).await.unwrap();
    api.evm_set_block_gas_limit(U256::from(21_000)).unwrap();

    let file = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter("off,backend=trace,backend::fees=off")
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(file.reopen().unwrap()))
        .finish();
    let _subscriber = tracing::subscriber::set_default(subscriber);
    tokio::time::timeout(Duration::from_secs(10), api.mine_one())
        .await
        .expect("mining timed out")
        .unwrap();

    assert_data_eq!(
        fs::read_to_string(file.path()).unwrap().replace(&hash.to_string(), "[TX_HASH]"),
        str![[r#"
TRACE backend: creating new block with 1 transactions
TRACE backend: block gas limit exhausting, skipping transaction tx_gas_limit=30000 tx_hash=[TX_HASH]
TRACE backend: Mined block 1 with 0 tx []

"#]]
    );
}

// The invalid host fails URL parsing before any network request.
const INVALID_FORK_URL: &str = "https://synthetic-user:synthetic-password@invalid host/rpc/synthetic-api-key?key=short#synthetic-secret";

#[tokio::test]
async fn ws_rpc_logs_preserve_failed_requests() {
    assert_pubsub_logs_preserve_failed_reset(false).await;
}

#[tokio::test]
async fn ipc_rpc_logs_preserve_failed_requests() {
    assert_pubsub_logs_preserve_failed_reset(true).await;
}

async fn assert_pubsub_logs_preserve_failed_reset(ipc: bool) {
    let dir = tempfile::tempdir().unwrap();
    let ipc_path = if cfg!(unix) {
        dir.path().join("anvil.ipc").to_string_lossy().into_owned()
    } else {
        format!(r"\\.\pipe\anvil_rpc_logging_{}.ipc", rand::random::<u64>())
    };
    let (_api, handle) =
        spawn(NodeConfig::test().with_tracing(false).with_ipc(ipc.then_some(Some(ipc_path)))).await;
    let file = tempfile::NamedTempFile::new().unwrap();
    let subscriber = rpc_subscriber(&file, true);
    let _subscriber = tracing::dispatcher::set_default(&subscriber);
    tokio::time::timeout(Duration::from_secs(10), async {
        let endpoint = if ipc { handle.ipc_path().unwrap() } else { handle.ws_endpoint() };
        // Select IPC explicitly: detecting a Windows pipe opens a throwaway connection.
        let provider = if ipc {
            builder().connect_ipc(IpcConnect::new(endpoint)).await.unwrap()
        } else {
            utils::connect_pubsub(&endpoint).await
        };
        let err = provider
            .raw_request::<_, ()>("hardhat_reset".into(), json!([[INVALID_FORK_URL, 42]]))
            .await
            .unwrap_err();
        assert_eq!(err.as_error_resp().unwrap().code, -32603);
        assert_eq!(
            err.as_error_resp().unwrap().message,
            "failed to establish provider to fork url"
        );
    })
    .await
    .expect("fork reset RPC request timed out");
    assert_data_eq!(
        fs::read_to_string(file.path()).unwrap(),
        str![[r#"
TRACE rpc: received method call id=Number(0) method="hardhat_reset" params=Array [Array [String("[REDACTED_URL]"), Number(42)]]
TRACE rpc::api: executing eth request
 INFO node::user: anvil_reset
 INFO node::user: RPC request failed method="hardhat_reset" id=Number(0) params=[["[REDACTED_URL]",42]] error=Internal error: failed to establish provider to fork url

"#]]
    );
}

#[test]
fn reset_parameter_shapes_deserialize_without_changing_credentials() {
    for params in [
        json!([{"forking": {"jsonRpcUrl": INVALID_FORK_URL, "blockNumber": 42}}]),
        json!({"params": {"forking": {"jsonRpcUrl": INVALID_FORK_URL, "blockNumber": 42}}}),
        json!([[INVALID_FORK_URL, 42]]),
        json!([{"forking": [INVALID_FORK_URL, "0x2a"]}]),
        json!({"params": [INVALID_FORK_URL, 42]}),
        json!([[[INVALID_FORK_URL, "0x2a"]]]),
    ] {
        for method in ["anvil_reset", "hardhat_reset"] {
            let request =
                serde_json::from_value::<EthRequest>(json!({"method": method, "params": params}))
                    .unwrap();
            let EthRequest::Reset(Some(params)) = request else {
                panic!("expected reset parameters");
            };
            let fork = params.params.unwrap();
            assert_eq!(fork.json_rpc_url.as_deref(), Some(INVALID_FORK_URL));
            assert_eq!(fork.block_number, Some(42));
        }
    }
}
