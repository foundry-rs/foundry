//! IPC tests

use crate::{init_tracing, utils::connect_pubsub};
use alloy_primitives::U256;
use alloy_provider::Provider;
use anvil::{spawn, NodeConfig};
use futures::StreamExt;
use tempfile::TempDir;

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
#[cfg_attr(windows, ignore = "TODO")]
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
#[cfg_attr(windows, ignore = "TODO")]
async fn test_sub_new_heads_ipc() {
    init_tracing();

    let (_dir, config) = ipc_config();
    let (api, handle) = spawn(config).await;

    let provider = connect_pubsub(handle.ipc_path().unwrap().as_str()).await;
    // mine a block every 1 seconds
    api.anvil_set_interval_mining(1).unwrap();

    let blocks = provider.subscribe_blocks().await.unwrap().into_stream();

    let blocks = blocks.take(3).collect::<Vec<_>>().await;
    let block_numbers = blocks.into_iter().map(|b| b.header.number).collect::<Vec<_>>();

    assert_eq!(block_numbers, vec![1, 2, 3]);
}
