use crate::utils::{TestNode, assert_with_tolerance, to_hex_string, unwrap_response};
use alloy_primitives::U256;
use anvil_core::eth::EthRequest;
use anvil_polkadot::config::{AnvilNodeConfig, SubstrateNodeConfig};

#[tokio::test(flavor = "multi_thread")]
async fn test_genesis() {
    let genesis_block_number: u32 = 1000;
    let anvil_genesis_timestamp: u64 = 42;
    let chain_id: u64 = 4242;
    let anvil_node_config = AnvilNodeConfig::test_config()
        .with_genesis_block_number(Some(genesis_block_number))
        .with_genesis_timestamp(Some(anvil_genesis_timestamp))
        .with_chain_id(Some(chain_id));
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config, substrate_node_config).await.unwrap();

    // Check that block number, timestamp, and chain id are set correctly at genesis
    assert_eq!(node.best_block_number().await, genesis_block_number);
    let genesis_hash = node.block_hash_by_number(genesis_block_number).await.unwrap();
    // Anvil genesis timestamp is in seconds, while Substrate timestamp is in milliseconds.
    let genesis_timestamp = anvil_genesis_timestamp.checked_mul(1000).unwrap();
    let actual_genesis_timestamp = node.get_decoded_timestamp(Some(genesis_hash)).await;
    assert_eq!(actual_genesis_timestamp, genesis_timestamp);
    let current_chain_id_hex =
        unwrap_response::<String>(node.eth_rpc(EthRequest::EthChainId(())).await.unwrap()).unwrap();
    assert_eq!(current_chain_id_hex, to_hex_string(chain_id));

    // Manually mine two blocks and force the timestamp to be increasing with 1 second each time.
    unwrap_response::<()>(
        node.eth_rpc(EthRequest::Mine(Some(U256::from(2)), Some(U256::from(1)))).await.unwrap(),
    )
    .unwrap();

    let latest_block_number = node.best_block_number().await;
    assert_eq!(latest_block_number, genesis_block_number + 2);
    let hash2 = node.block_hash_by_number(genesis_block_number + 2).await.unwrap();
    let timestamp2 = node.get_decoded_timestamp(Some(hash2)).await;
    assert_with_tolerance(
        timestamp2.saturating_sub(genesis_timestamp),
        2000,
        500,
        "Timestamp is not increasing as expected from genesis.",
    );
}
