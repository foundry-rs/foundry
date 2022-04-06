use crate::{init_tracing, next_port};
use foundry_node::{spawn, NodeConfig};

#[tokio::test(flavor = "multi_thread")]
async fn can_send_transaction() {
    init_tracing();

    let (_api, _handle) = spawn(NodeConfig::default().port(next_port()));
}
