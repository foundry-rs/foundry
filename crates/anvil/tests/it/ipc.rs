//! IPC tests

use crate::utils::connect_pubsub;
use alloy_primitives::U256;
use alloy_provider::Provider;
use anvil::{spawn, NodeConfig};
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

// TODO: throws: `Transport(Custom(Os { code: 2, kind: NotFound, message: "The system cannot find
// the file specified." }))` on Windows
#[tokio::test(flavor = "multi_thread")]
// #[cfg_attr(target_os = "windows", ignore)]
async fn can_get_block_number_ipc() {
    let (api, handle) = spawn(ipc_config()).await;

    let block_num = api.block_number().unwrap();
    assert_eq!(block_num, U256::ZERO);

    let provider = handle.ipc_provider().unwrap();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, block_num.to::<u64>());
}

// TODO: throws: `Transport(Custom(Os { code: 2, kind: NotFound, message: "The system cannot find
// the file specified." }))` on Windows
#[tokio::test(flavor = "multi_thread")]
// #[cfg_attr(target_os = "windows", ignore)]
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
