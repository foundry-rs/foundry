//! Regression tests for failed RPC request dumps and credential redaction.

use super::*;
use ::anvil::{
    NodeConfig,
    logging::{LoggingManager, NodeLogLayer},
    spawn,
};
use alloy_primitives::{B256, Bytes, U256, keccak256};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use foundry_evm::hardfork::EthereumHardfork;
use foundry_test_utils::{assert_data_eq, snapbox::IntoData, str};
use futures::StreamExt;
use serde_json::json;
use std::{fs, sync::Arc, time::Duration};
use tracing_subscriber::prelude::*;

#[tokio::test]
async fn http_rpc_logs_preserve_failed_requests() {
    assert_rpc_logs_preserve_failed_requests("http", true).await;
}

#[tokio::test]
async fn ws_rpc_logs_preserve_failed_requests() {
    assert_rpc_logs_preserve_failed_requests("ws", true).await;
}

#[tokio::test]
async fn ipc_rpc_logs_preserve_failed_requests() {
    assert_rpc_logs_preserve_failed_requests("ipc", true).await;
}

#[tokio::test]
async fn http_default_logs_identify_failed_requests() {
    assert_rpc_logs_preserve_failed_requests("http", false).await;
}

#[tokio::test]
async fn ws_default_logs_identify_failed_requests() {
    assert_rpc_logs_preserve_failed_requests("ws", false).await;
}

#[tokio::test]
async fn ipc_default_logs_identify_failed_requests() {
    assert_rpc_logs_preserve_failed_requests("ipc", false).await;
}

async fn assert_rpc_logs_preserve_failed_requests(transport: &str, trace: bool) {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("rpc.log");
    let writer = Arc::new(fs::File::create(&log_path).unwrap());
    let subscriber = if trace {
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
    };
    let ipc_path = if cfg!(unix) {
        dir.path().join("anvil.ipc").to_string_lossy().into_owned()
    } else {
        format!(r"\\.\pipe\anvil_rpc_logging_{}.ipc", rand::random::<u64>())
    };
    let raw = tokio::time::timeout(Duration::from_secs(10), async {
        let (api, handle) = spawn(
            NodeConfig::test()
                .with_tracing(false)
                .with_no_mining(true)
                .with_ipc(Some(Some(ipc_path))),
        )
        .await;
        let from = handle.dev_accounts().next().unwrap();
        let transaction = TransactionRequest::default()
            .from(from)
            .to(from)
            .gas_limit(30_000)
            .gas_price(2_000_000_000)
            .nonce(0);
        let raw = api.sign_transaction(WithOtherFields::new(transaction)).await.unwrap();
        let expected_hash = keccak256(raw.parse::<Bytes>().unwrap());
        let endpoint = match transport {
            "http" => handle.http_endpoint(),
            "ws" => handle.ws_endpoint(),
            "ipc" => handle.ipc_path().unwrap(),
            _ => unreachable!(),
        };

        // The current-thread runtime keeps spawned server tasks under this subscriber.
        let _subscriber = tracing::dispatcher::set_default(&subscriber);
        let provider = utils::connect_pubsub(&endpoint).await;
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
ERROR rpc: failed to deserialize method method="anvil_setAutomine" err=[..]
TRACE rpc: received method call id=Number(4) method="eth_call" params=Array [Object {"to": String("0x1111111111111111111111111111111111111111"), "data": String("0x12345678")}, String("0x100000")]
TRACE rpc::api: executing eth request
 INFO node::user: eth_call
 INFO node::user: RPC request failed method="eth_call" id=Number(4) params=[{"to":"0x1111111111111111111111111111111111111111","data":"0x12345678"},"0x100000"] error=Invalid params: BlockOutOfRangeError: block height is 0 but requested was 1048576

"#]]
    } else {
        str![[r#"
anvil_setRpcUrl
eth_sendRawTransaction
eth_sendRawTransaction
RPC request failed method="eth_sendRawTransaction" id=Number(2) params=["[RAW_TX]"] error=Transaction rejected: transaction already imported
eth_call
RPC request failed method="eth_call" id=Number(4) params=[{"to":"0x1111111111111111111111111111111111111111","data":"0x12345678"},"0x100000"] error=Invalid params: BlockOutOfRangeError: block height is 0 but requested was 1048576

"#]]
    };
    assert_data_eq!(fs::read_to_string(log_path).unwrap().replace(&raw, "[RAW_TX]"), expected);
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

#[tokio::test]
async fn concurrent_default_logs_identify_failed_requests() {
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_tracing(false)
            .with_no_mining(true)
            .with_hardfork(Some(EthereumHardfork::Prague.into())),
    )
    .await;
    let from = handle.dev_accounts().next().unwrap();
    let transaction = TransactionRequest::default()
        .from(from)
        .to(from)
        .gas_limit(30_000)
        .gas_price(2_000_000_000)
        .nonce(0);
    let raw = api.sign_transaction(WithOtherFields::new(transaction)).await.unwrap();
    let hash = keccak256(raw.parse::<Bytes>().unwrap());
    let mut ready_transactions = api.new_ready_transactions();
    let file = tempfile::NamedTempFile::new().unwrap();
    let subscriber =
        tracing_subscriber::registry().with(NodeLogLayer::new(LoggingManager::default())).with(
            tracing_subscriber::fmt::layer()
                .without_time()
                .with_ansi(false)
                .with_target(false)
                .with_level(false)
                .with_writer(Arc::new(file.reopen().unwrap())),
        );
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let client = reqwest::Client::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        let slow = client
            .post(handle.http_endpoint())
            .json(&json!({
                "jsonrpc": "2.0",
                "id": "waiting-for-confirmation",
                "method": "eth_sendRawTransactionSync",
                "params": [&raw, 5000],
            }))
            .send();
        tokio::pin!(slow);

        // Start the other requests only after the first is waiting for a block.
        tokio::select! {
            response = &mut slow => panic!("request completed before submission: {response:?}"),
            transaction = ready_transactions.next() => assert_eq!(transaction, Some(hash)),
        }
        let fast = client
            .post(handle.http_endpoint())
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 42,
                "method": "eth_sendRawTransaction",
                "params": [&raw],
            }))
            .send();
        let alias = client
            .post(handle.http_endpoint())
            .json(&json!({
                "jsonrpc": "2.0",
                "id": "minimum-gas-price",
                "method": "hardhat_setMinGasPrice",
                "params": ["0x1"],
            }))
            .send();
        let (response, alias_response) = tokio::select! {
            response = &mut slow => panic!("request completed before the concurrent failure: {response:?}"),
            responses = async { tokio::join!(fast, alias) } => responses,
        };
        assert_data_eq!(
            response.unwrap().text().await.unwrap(),
            str![[r#"
{
  "jsonrpc": "2.0",
  "id": 42,
  "error": {
    "code": -32003,
    "message": "transaction already imported"
  }
}
"#]].is_json()
        );
        assert_data_eq!(
            alias_response.unwrap().text().await.unwrap(),
            str![[r#"
{
  "jsonrpc": "2.0",
  "id": "minimum-gas-price",
  "error": {
    "code": -32602,
    "message": "anvil_setMinGasPrice is not supported when EIP-1559 is active"
  }
}
"#]].is_json()
        );
        assert_data_eq!(
            slow.await.unwrap().text().await.unwrap().replace(&hash.to_string(), "[TX_HASH]"),
            str![[r#"
{
  "jsonrpc": "2.0",
  "id": "waiting-for-confirmation",
  "error": {
    "code": 4,
    "message": "Transaction confirmation timeout",
    "data": "[TX_HASH]"
  }
}
"#]].is_json()
        );
    })
    .await
    .expect("concurrent RPC requests timed out");

    // The immediate failures may finish in either order; retain every complete log line.
    let logs = fs::read_to_string(file.path()).unwrap().replace(&raw, "[RAW_TX]");
    let mut lines = logs.lines().collect::<Vec<_>>();
    lines.sort_unstable();
    assert_data_eq!(
        lines.join("\n") + "\n",
        str![[r#"
RPC request failed method="eth_sendRawTransaction" id=Number(42) params=["[RAW_TX]"] error=Transaction rejected: transaction already imported
RPC request failed method="eth_sendRawTransactionSync" id=String("waiting-for-confirmation") params=["[RAW_TX]",5000] error=Server error: Transaction confirmation timeout
RPC request failed method="hardhat_setMinGasPrice" id=String("minimum-gas-price") params=["0x1"] error=Invalid params: anvil_setMinGasPrice is not supported when EIP-1559 is active
anvil_setMinGasPrice
eth_sendRawTransaction
eth_sendRawTransaction
eth_sendRawTransactionSync

"#]]
    );
}

#[tokio::test]
async fn fork_default_logs_preserve_failed_resets() {
    assert_fork_logs_preserve_failed_resets(false).await;
}

#[tokio::test]
async fn fork_trace_logs_preserve_failed_resets() {
    assert_fork_logs_preserve_failed_resets(true).await;
}

async fn assert_fork_logs_preserve_failed_resets(trace: bool) {
    let (_api, handle) = spawn(NodeConfig::test().with_tracing(false)).await;
    let file = tempfile::NamedTempFile::new().unwrap();
    let writer = Arc::new(file.reopen().unwrap());
    let subscriber = if trace {
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
    };
    let _subscriber = tracing::dispatcher::set_default(&subscriber);
    tokio::time::timeout(Duration::from_secs(10), async {
        let provider = utils::connect_pubsub(&handle.http_endpoint()).await;
        // The invalid host fails URL parsing before any network request.
        let url = "https://synthetic-user:synthetic-password@invalid host/rpc/synthetic-api-key?key=short#synthetic-secret";
        for (method, params) in [
            (
                "anvil_reset",
                json!([{"forking": {"jsonRpcUrl": url, "blockNumber": 42}}]),
            ),
            (
                "hardhat_reset",
                json!({"params": {"forking": {"jsonRpcUrl": url, "blockNumber": 43}}}),
            ),
        ] {
            let err = provider.raw_request::<_, ()>(method.into(), params).await.unwrap_err();
            assert_eq!(err.as_error_resp().unwrap().code, -32603);
            assert_eq!(
                err.as_error_resp().unwrap().message,
                "failed to establish provider to fork url"
            );
        }
    })
    .await
    .expect("fork reset RPC requests timed out");

    let expected = if trace {
        str![[r#"
TRACE rpc: received method call id=Number(0) method="anvil_reset" params=Array [Object {"forking": Object {"jsonRpcUrl": String("[REDACTED_URL]"), "blockNumber": Number(42)}}]
TRACE rpc::api: executing eth request
 INFO node::user: anvil_reset
 INFO node::user: RPC request failed method="anvil_reset" id=Number(0) params=[{"forking":{"jsonRpcUrl":"[REDACTED_URL]","blockNumber":42}}] error=Internal error: failed to establish provider to fork url
TRACE rpc: received method call id=Number(1) method="hardhat_reset" params=Object {"params": Object {"forking": Object {"jsonRpcUrl": String("[REDACTED_URL]"), "blockNumber": Number(43)}}}
TRACE rpc::api: executing eth request
 INFO node::user: anvil_reset
 INFO node::user: RPC request failed method="hardhat_reset" id=Number(1) params={"params":{"forking":{"jsonRpcUrl":"[REDACTED_URL]","blockNumber":43}}} error=Internal error: failed to establish provider to fork url

"#]]
    } else {
        str![[r#"
anvil_reset
RPC request failed method="anvil_reset" id=Number(0) params=[{"forking":{"jsonRpcUrl":"[REDACTED_URL]","blockNumber":42}}] error=Internal error: failed to establish provider to fork url
anvil_reset
RPC request failed method="hardhat_reset" id=Number(1) params={"params":{"forking":{"jsonRpcUrl":"[REDACTED_URL]","blockNumber":43}}} error=Internal error: failed to establish provider to fork url

"#]]
    };
    assert_data_eq!(fs::read_to_string(file.path()).unwrap(), expected);
}
