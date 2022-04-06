use crate::{init_tracing, next_port};
use ethers::{self, prelude::Provider};
use foundry_node::{spawn, NodeConfig};

#[tokio::test(flavor = "multi_thread")]
async fn can_send_transaction() {
    init_tracing();

    let (api, handle) = spawn(NodeConfig::default().port(next_port()));
}
