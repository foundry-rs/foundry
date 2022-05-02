//! tests for anvil specifc logic

use crate::next_port;
use anvil::{spawn, NodeConfig};
use ethers::prelude::Middleware;

#[tokio::test(flavor = "multi_thread")]
async fn test_can_change_mining_mode() {
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

    assert!(api.anvil_get_auto_mine().unwrap());

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num.as_u64(), 0);

    api.anvil_set_interval_mining(1).unwrap();
    assert!(!api.anvil_get_auto_mine().unwrap());
    // changing the mining mode will instantly mine a new block
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num.as_u64(), 1);

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num.as_u64(), 2);
}
