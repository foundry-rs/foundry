use crate::utils::TestNode;
use anvil_core::eth::EthRequest;
use anvil_polkadot::config::{AnvilNodeConfig, SubstrateNodeConfig};
use anvil_rpc::{
    error::{ErrorCode, RpcError},
    response::ResponseResult,
};
use tokio::time::{sleep, Duration};

#[tokio::test(flavor = "multi_thread")]
async fn demo_test() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config, substrate_node_config).await.unwrap();

    let mine_req = EthRequest::Mine(None, None);

    let response = node.eth_rpc(mine_req).await.unwrap();
    assert!(matches!(
        response,
        ResponseResult::Error(RpcError { code: ErrorCode::InternalError, .. })
    ));

    while node.get_best_block_number().await.unwrap() < 3 {
        sleep(Duration::from_secs(8)).await;
    }
    let hash3 = node.block_hash_by_number(3).await.unwrap();
    let hash2 = node.block_hash_by_number(2).await.unwrap();
    let timestamp2 = node.get_decoded_timestamp(Some(hash2)).await;
    let timestamp3 = node.get_decoded_timestamp(Some(hash3)).await;
    let timestamp_diff = timestamp3.saturating_sub(timestamp2);
    let expected_block_time = 6000;
    let tolerance = 2000;
    assert!(
        timestamp_diff >= expected_block_time - tolerance,
        "❌ Block time too fast! Got {timestamp_diff}ms, expected ≥4000ms",
    );

    assert!(
        timestamp_diff <= expected_block_time + tolerance,
        "❌ Block time too slow! Got {timestamp_diff}ms, expected ≤8000ms",
    );
}
