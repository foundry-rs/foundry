//! tests for custom anvil endpoints

use crate::next_port;
use anvil::{spawn, NodeConfig};
use ethers::prelude::Middleware;

#[tokio::test(flavor = "multi_thread")]
async fn can_set_gas_price() {
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

    let gas_price = 1337u64.into();
    api.anvil_set_min_gas_price(gas_price).await.unwrap();
    assert_eq!(gas_price, provider.get_gas_price().await.unwrap());
}
