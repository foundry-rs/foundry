//! IPC tests

use crate::{init_tracing, utils::connect_pubsub};
use alloy_primitives::{Address, U256};
use alloy_provider::Provider;
use anvil::{NodeConfig, spawn};
use futures::StreamExt;
use std::time::Duration;
use tempfile::TempDir;
#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(unix)]
use tokio::net::UnixStream;

fn ipc_config() -> (Option<TempDir>, NodeConfig) {
    let path;
    let dir;
    if cfg!(unix) {
        let tmp = tempfile::tempdir().unwrap();
        path = tmp.path().join("anvil.ipc").to_string_lossy().into_owned();
        dir = Some(tmp);
    } else {
        dir = None;
        path = format!(r"\\.\pipe\anvil_test_{}.ipc", rand::random::<u64>());
    }
    let config = NodeConfig::test().with_ipc(Some(Some(path)));
    (dir, config)
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_block_number_ipc() {
    init_tracing();

    let (_dir, config) = ipc_config();
    let (api, handle) = spawn(config).await;

    let block_num = api.block_number().unwrap();
    assert_eq!(block_num, U256::ZERO);

    let provider = handle.ipc_provider().unwrap();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, block_num.to::<u64>());
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_handle_closes_ipc_connections() {
    let (_dir, config) = ipc_config();
    let (_api, handle) = spawn(config).await;
    let provider = handle.ipc_provider().unwrap();

    provider.get_balance(Address::ZERO).await.unwrap();
    drop(handle);

    tokio::time::timeout(Duration::from_secs(5), async {
        while let Ok(Ok(_)) =
            tokio::time::timeout(Duration::from_millis(500), provider.get_balance(Address::ZERO))
                .await
        {}
    })
    .await
    .expect("IPC connection continued serving requests after node shutdown");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn malformed_delimiter_does_not_corrupt_ipc_connection() {
    let (_dir, config) = ipc_config();
    let (_api, handle) = spawn(config).await;
    let connection = UnixStream::connect(handle.ipc_path().unwrap()).await.unwrap();
    let (reader, mut writer) = connection.into_split();
    let mut reader = BufReader::new(reader);
    let mut response = String::new();

    writer.write_all(b"}").await.unwrap();
    reader.read_line(&mut response).await.unwrap();
    let malformed = serde_json::from_str::<serde_json::Value>(&response).unwrap();
    assert_eq!(malformed["error"]["code"], -32700);

    response.clear();
    writer
        .write_all(br#"{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}"#)
        .await
        .unwrap();
    reader.read_line(&mut response).await.unwrap();
    let valid = serde_json::from_str::<serde_json::Value>(&response).unwrap();
    assert_eq!(valid["result"], "0x7a69");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_new_heads_ipc() {
    init_tracing();

    let (_dir, config) = ipc_config();
    let (api, handle) = spawn(config).await;

    let provider = connect_pubsub(handle.ipc_path().unwrap().as_str()).await;
    // mine a block every 1 seconds
    api.anvil_set_interval_mining(1).unwrap();

    let blocks = provider.subscribe_blocks().await.unwrap().into_stream();

    let blocks = blocks.take(3).collect::<Vec<_>>().await;
    let block_numbers = blocks.into_iter().map(|b| b.number).collect::<Vec<_>>();

    assert_eq!(block_numbers, vec![1, 2, 3]);
}
