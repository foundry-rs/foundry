//! IPC tests

use anvil::{spawn, NodeConfig};
use ethers::{core::rand, prelude::Middleware, types::U256};
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
    assert_eq!(block_num, U256::zero());

    let provider = handle.ipc_provider().unwrap();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, block_num.as_u64().into());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_new_heads_ipc() {
    let (api, handle) = spawn(ipc_config()).await;

    let provider = handle.ipc_provider().unwrap();

    let blocks = provider.subscribe_blocks().await.unwrap();

    // mine a block every 1 seconds
    api.anvil_set_interval_mining(1).unwrap();

    let blocks = blocks.take(3).collect::<Vec<_>>().await;
    let block_numbers = blocks.into_iter().map(|b| b.number.unwrap().as_u64()).collect::<Vec<_>>();

    assert_eq!(block_numbers, vec![1, 2, 3]);
}
