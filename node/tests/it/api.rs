//! general eth api tests

use crate::{init_tracing, next_port};
use ethers::types::U256;
use foundry_node::{spawn, NodeConfig};

#[tokio::test]
async fn can_get_block_number() {
    let (api, _handle) = spawn(NodeConfig::default().port(next_port()));

    dbg!(_handle.http_endpoint());
    let block_num = api.block_number().unwrap();
    assert_eq!(block_num, U256::zero());
}
