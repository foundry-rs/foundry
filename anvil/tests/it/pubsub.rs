//! general eth api tests

use crate::next_port;
use anvil::{spawn, NodeConfig};
use ethers::{prelude::Middleware};

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_new_heads() {
    let (_api, handle) = spawn(NodeConfig::test().port(next_port())).await;

    let provider = handle.ws_provider().await;

    let _blocks = provider.subscribe_blocks().await.unwrap();
}
