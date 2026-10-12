//! Regression tests for sensitive RPC payloads in server logs.

use super::*;
use ::anvil::{NodeConfig, spawn};
use alloy_primitives::{B256, Bytes, U256, keccak256};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use foundry_test_utils::{assert_data_eq, str};
use std::{fs, sync::Arc, time::Duration};

#[tokio::test]
async fn http_rpc_logs_omit_payloads() {
    assert_rpc_logs_omit_payloads("http").await;
}

#[tokio::test]
async fn ws_rpc_logs_omit_payloads() {
    assert_rpc_logs_omit_payloads("ws").await;
}

#[tokio::test]
async fn ipc_rpc_logs_omit_payloads() {
    assert_rpc_logs_omit_payloads("ipc").await;
}

async fn assert_rpc_logs_omit_payloads(transport: &str) {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("rpc.log");
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter("off,rpc=trace,node::user=info")
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(fs::File::create(&log_path).unwrap()))
        .finish();
    let ipc_path = if cfg!(unix) {
        dir.path().join("anvil.ipc").to_string_lossy().into_owned()
    } else {
        format!(r"\\.\pipe\anvil_rpc_logging_{}.ipc", rand::random::<u64>())
    };
    tokio::time::timeout(Duration::from_secs(10), async {
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
        let _subscriber = tracing::subscriber::set_default(subscriber);
        let provider = utils::connect_pubsub(&endpoint).await;
        provider
            .raw_request::<_, ()>(
                "anvil_setRpcUrl".into(),
                ("https://example.invalid/rpc/synthetic-api-key",),
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
    })
    .await
    .expect("RPC requests timed out");

    // Ignore the formatter's trailing space before the error header's newline.
    assert_data_eq!(
        fs::read_to_string(log_path).unwrap().replace(" \n", "\n"),
        str![[r#"
TRACE rpc: handling call id=Number(0) method="anvil_setRpcUrl"
TRACE rpc: received method call id=Number(0) method="anvil_setRpcUrl"
TRACE rpc::api: executing eth request
 INFO node::user: anvil_setRpcUrl
TRACE rpc: handling call id=Number(1) method="eth_sendRawTransaction"
TRACE rpc: received method call id=Number(1) method="eth_sendRawTransaction"
TRACE rpc::api: executing eth request
 INFO node::user: eth_sendRawTransaction
TRACE rpc: handling call id=Number(2) method="eth_sendRawTransaction"
TRACE rpc: received method call id=Number(2) method="eth_sendRawTransaction"
TRACE rpc::api: executing eth request
 INFO node::user: eth_sendRawTransaction
 INFO node::user:
RPC request failed:
 INFO node::user:     Error: Transaction rejected: transaction already imported

TRACE rpc: handling call id=Number(3) method="anvil_setAutomine"
TRACE rpc: received method call id=Number(3) method="anvil_setAutomine"
ERROR rpc: failed to deserialize method method="anvil_setAutomine"

"#]]
    );
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
