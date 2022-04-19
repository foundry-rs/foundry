//! general eth api tests

use crate::{init_tracing, next_port};
use anvil::{spawn, NodeConfig};
use ethers::prelude::Middleware;
use futures::StreamExt;

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_new_heads() {
    init_tracing();
    let (api, handle) = spawn(NodeConfig::test().port(next_port())).await;

    let provider = handle.ws_provider().await;

    let blocks = provider.subscribe_blocks().await.unwrap();

    // mine a block every 1 seconds
    api.anvil_set_interval_mining(1).unwrap();

    let blocks = blocks.take(3).collect::<Vec<_>>().await;
    let block_numbers = blocks.into_iter().map(|b| b.number.unwrap().as_u64()).collect::<Vec<_>>();

    assert_eq!(block_numbers, vec![1, 2, 3]);
}
