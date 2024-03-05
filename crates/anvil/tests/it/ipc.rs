//! IPC tests

use crate::utils::ethers_ipc_provider;
use alloy_primitives::U256;
use anvil::{spawn, NodeConfig};
use ethers::prelude::Middleware;
use futures::StreamExt;

pub fn rand_ipc_endpoint() -> String {
    let num: u64 = rand::Rng::gen(&mut rand::thread_rng());
    if cfg!(windows) {
        format!(r"\\.\pipe\anvil-ipc-{num}")
    } else {
        format!(r"/tmp/anvil-ipc-{num}")
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

    let provider = ethers_ipc_provider(handle.ipc_path()).unwrap();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num.as_u64(), block_num.to::<u64>());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_new_heads_ipc() {
    let (api, handle) = spawn(ipc_config()).await;

    let provider = ethers_ipc_provider(handle.ipc_path()).unwrap();

    let blocks = provider.subscribe_blocks().await.unwrap();

    // mine a block every 1 seconds
    api.anvil_set_interval_mining(1).unwrap();

    let blocks = blocks.take(3).collect::<Vec<_>>().await;
    let block_numbers = blocks.into_iter().map(|b| b.number.unwrap().as_u64()).collect::<Vec<_>>();

    assert_eq!(block_numbers, vec![1, 2, 3]);
}
