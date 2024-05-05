//! IPC tests

use crate::{init_tracing, utils::connect_pubsub};
use alloy_primitives::U256;
use alloy_provider::Provider;
use anvil::{spawn, NodeConfig};
use futures::StreamExt;
use tempfile::TempDir;

fn ipc_config() -> (TempDir, NodeConfig) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("anvil.ipc").to_string_lossy().into_owned();
    let config = NodeConfig::test().with_ipc(Some(Some(path)));
    (dir, config)
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_block_number_ipc() {
    init_tracing();

    eprintln!("a");
    let (_dir, config) = ipc_config();
    let (api, handle) = spawn(config).await;

    eprintln!("b");
    let block_num = api.block_number().unwrap();
    assert_eq!(block_num, U256::ZERO);

    eprintln!("c");
    let provider = handle.ipc_provider().unwrap();

    eprintln!("d");
    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, block_num.to::<u64>());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_new_heads_ipc() {
    init_tracing();

    eprintln!("a");
    let (_dir, config) = ipc_config();
    let (api, handle) = spawn(config).await;

    eprintln!("b");
    let provider = connect_pubsub(handle.ipc_path().unwrap().as_str()).await;
    // mine a block every 1 seconds
    api.anvil_set_interval_mining(1).unwrap();

    eprintln!("c");
    let blocks = provider.subscribe_blocks().await.unwrap().into_stream();

    eprintln!("d");
    let blocks = blocks.take(3).collect::<Vec<_>>().await;
    let block_numbers = blocks.into_iter().map(|b| b.header.number.unwrap()).collect::<Vec<_>>();

    assert_eq!(block_numbers, vec![1, 2, 3]);
}
