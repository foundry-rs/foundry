//! IPC tests

use crate::utils::connect_pubsub;
use alloy_primitives::U256;
use alloy_provider::Provider;
use anvil::{spawn, NodeConfig};
use futures::StreamExt;
use tempfile::NamedTempFile;

pub fn rand_ipc_endpoint() -> String {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.into_temp_path().to_path_buf();

    // [Windows named pipes](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipes)
    // are located at `\\<machine_address>\pipe\<pipe_name>`.
    if cfg!(windows) {
        format!(r"\\.\pipe\{}", path.display())
    } else {
        path.display().to_string()
    }
}

fn ipc_config() -> NodeConfig {
    NodeConfig::test().with_ipc(Some(Some(rand_ipc_endpoint())))
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_block_number_ipc() {
    let (api, handle) = spawn(ipc_config()).await;

    let block_num = api.block_number().unwrap();
    assert_eq!(block_num, U256::ZERO);

    let provider = handle.ipc_provider().unwrap();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, block_num.to::<u64>());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_new_heads_ipc() {
    let (api, handle) = spawn(ipc_config()).await;

    let provider = connect_pubsub(handle.ipc_path().unwrap().as_str()).await;

    let blocks = provider.subscribe_blocks().await.unwrap().into_stream();

    // mine a block every 1 seconds
    api.anvil_set_interval_mining(1).unwrap();

    let blocks = blocks.take(3).collect::<Vec<_>>().await;
    let block_numbers = blocks.into_iter().map(|b| b.header.number.unwrap()).collect::<Vec<_>>();

    assert_eq!(block_numbers, vec![1, 2, 3]);
}
