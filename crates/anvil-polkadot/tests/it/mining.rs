use crate::utils::{assert_with_tolerance, unwrap_response, TestNode};
use alloy_primitives::U256;
use alloy_rpc_types::anvil::MineOptions;
use anvil::eth::backend::time::duration_since_unix_epoch;
use anvil_core::eth::{EthRequest, Params};
use anvil_polkadot::{
    cmd::NodeArgs,
    config::{AnvilNodeConfig, SubstrateNodeConfig},
};
use anvil_rpc::{
    error::{ErrorCode, RpcError},
    response::ResponseResult,
};
use polkadot_sdk::sc_cli::clap::Parser;
use std::time::{Duration, SystemTime};
use subxt_signer::sr25519::dev;

#[tokio::test(flavor = "multi_thread")]
async fn test_invalid_mining() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config, substrate_node_config).await.unwrap();

    // Assert that we are in the correct mining mode
    assert!(
        !unwrap_response::<bool>(node.eth_rpc(EthRequest::GetAutoMine(())).await.unwrap()).unwrap()
    );
    assert_eq!(
        unwrap_response::<Option<u64>>(
            node.eth_rpc(EthRequest::GetIntervalMining(())).await.unwrap()
        )
        .unwrap(),
        None
    );

    assert!(matches!(
        node.eth_rpc(EthRequest::Mine(Some(U256::from(u128::MAX)), None)).await.unwrap(),
        ResponseResult::Error(RpcError {
            code: ErrorCode::InvalidParams,
            message,
            data: None
        }) if message == "The number of blocks is too large"
    ));
    assert!(matches!(
        node.eth_rpc(EthRequest::Mine(None, Some(U256::from(u128::MAX)))).await.unwrap(),
        ResponseResult::Error(RpcError {
            code: ErrorCode::InvalidParams,
            message,
            data: None
        }) if message == "The interval between blocks is too large"
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_manual_mining_with_no_of_blocks() {
    let node_args = NodeArgs::parse_from(["anvil", "--no-mining", "--port", "0"]);
    let (mut anvil_node_config, substrate_node_config) = node_args.into_node_config().unwrap();
    anvil_node_config = anvil_node_config.set_silent(true);
    let mut node = TestNode::new(anvil_node_config, substrate_node_config).await.unwrap();
    assert!(
        !unwrap_response::<bool>(node.eth_rpc(EthRequest::GetAutoMine(())).await.unwrap()).unwrap()
    );
    assert_eq!(
        unwrap_response::<Option<u64>>(
            node.eth_rpc(EthRequest::GetIntervalMining(())).await.unwrap()
        )
        .unwrap(),
        None
    );

    assert_eq!(node.best_block_number().await, 0);

    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(Some(U256::from(2)), None)).await.unwrap())
        .unwrap();
    assert_eq!(node.best_block_number().await, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_manual_mining_with_interval() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config, substrate_node_config).await.unwrap();

    assert!(
        !unwrap_response::<bool>(node.eth_rpc(EthRequest::GetAutoMine(())).await.unwrap()).unwrap()
    );
    assert_eq!(
        unwrap_response::<Option<u64>>(
            node.eth_rpc(EthRequest::GetIntervalMining(())).await.unwrap()
        )
        .unwrap(),
        None
    );

    // Manually mine three blocks and force the timestamp to be increasing with 3 seconds.

    unwrap_response::<()>(
        node.eth_rpc(EthRequest::Mine(Some(U256::from(3)), Some(U256::from(3)))).await.unwrap(),
    )
    .unwrap();
    let hash3 = node.block_hash_by_number(3).await.unwrap();
    let hash2 = node.block_hash_by_number(2).await.unwrap();
    let hash1 = node.block_hash_by_number(1).await.unwrap();
    let timestamp1 = node.get_decoded_timestamp(Some(hash1)).await;
    let timestamp2 = node.get_decoded_timestamp(Some(hash2)).await;
    let timestamp3 = node.get_decoded_timestamp(Some(hash3)).await;
    assert_with_tolerance(
        timestamp2.saturating_sub(timestamp1),
        3000,
        100,
        "Interval between the blocks is outside of the desired range.",
    );
    assert_with_tolerance(
        timestamp3.saturating_sub(timestamp2),
        3000,
        100,
        "Interval between the blocks is outside of the desired range.",
    );
    assert_with_tolerance(
        timestamp3.saturating_sub(timestamp1),
        6000,
        100,
        "Interval between the blocks is outside of the desired range.",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_interval_mining() {
    let node_args = NodeArgs::parse_from(["anvil", "--block-time", "3", "--port", "0"]);
    let (mut anvil_node_config, substrate_node_config) = node_args.into_node_config().unwrap();
    anvil_node_config = anvil_node_config.set_silent(true);
    let mut node = TestNode::new(anvil_node_config, substrate_node_config).await.unwrap();
    // enable interval mining
    assert_eq!(
        unwrap_response::<u64>(node.eth_rpc(EthRequest::GetIntervalMining(())).await.unwrap())
            .unwrap(),
        3
    );
    unwrap_response::<()>(node.eth_rpc(EthRequest::SetIntervalMining(1)).await.unwrap()).unwrap();

    assert_eq!(
        unwrap_response::<u64>(node.eth_rpc(EthRequest::GetIntervalMining(())).await.unwrap())
            .unwrap(),
        1
    );

    let before_mining = SystemTime::now();
    node.wait_for_block_with_timeout(3, Duration::from_secs(4)).await.unwrap();
    let after_mining = SystemTime::now();
    assert_eq!(node.best_block_number().await, 3);
    assert_with_tolerance(
        after_mining.duration_since(before_mining).unwrap().as_millis(),
        3000,
        500,
        "Interval between the blocks is outside of the desired range.",
    );
    let hash3 = node.block_hash_by_number(3).await.unwrap();
    let hash2 = node.block_hash_by_number(2).await.unwrap();
    let hash1 = node.block_hash_by_number(1).await.unwrap();
    let timestamp1 = node.get_decoded_timestamp(Some(hash1)).await;
    let timestamp2 = node.get_decoded_timestamp(Some(hash2)).await;
    let timestamp3 = node.get_decoded_timestamp(Some(hash3)).await;
    assert_with_tolerance(
        timestamp2.saturating_sub(timestamp1),
        1000,
        100,
        "Interval between the blocks is outside of the desired range.",
    );
    assert_with_tolerance(
        timestamp3.saturating_sub(timestamp2),
        1000,
        100,
        "Interval between the blocks is outside of the desired range.",
    );
    assert_with_tolerance(
        timestamp3.saturating_sub(timestamp1),
        2000,
        100,
        "Interval between the blocks is outside of the desired range.",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_auto_mine() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config, substrate_node_config).await.unwrap();

    unwrap_response::<()>(node.eth_rpc(EthRequest::SetAutomine(true)).await.unwrap()).unwrap();

    assert_eq!(node.best_block_number().await, 0);
    node.submit_remark(dev::alice()).await;
    assert_eq!(node.best_block_number().await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mixed_mining() {
    let mut anvil_node_config = AnvilNodeConfig::test_config();
    anvil_node_config.mixed_mining = true;
    anvil_node_config.block_time = Some(Duration::from_secs(1));
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let node = TestNode::new(anvil_node_config, substrate_node_config).await.unwrap();
    node.submit_remark(dev::bob()).await;
    assert_eq!(node.best_block_number().await, 1);
    node.wait_for_block_with_timeout(2, Duration::from_secs(1)).await.unwrap();
    assert_eq!(node.best_block_number().await, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_evm_mine_empty_params() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config, substrate_node_config).await.unwrap();

    assert_eq!(
        unwrap_response::<String>(node.eth_rpc(EthRequest::EvmMine(None)).await.unwrap()).unwrap(),
        "0x0"
    );
    assert_eq!(node.best_block_number().await, 1);
    assert_eq!(
        unwrap_response::<String>(
            node.eth_rpc(EthRequest::EvmMine(Some(Params { params: None }))).await.unwrap()
        )
        .unwrap(),
        "0x0"
    );
    assert_eq!(node.best_block_number().await, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_evm_mine_timestamp_is_none() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config, substrate_node_config).await.unwrap();
    let _ = node
        .eth_rpc(EthRequest::EvmMine(Some(Params { params: Some(MineOptions::Timestamp(None)) })))
        .await
        .unwrap();
    assert_eq!(node.best_block_number().await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_evm_mine_timestamp_is_some() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config, substrate_node_config).await.unwrap();
    let now = duration_since_unix_epoch();
    let timestamp = now + Duration::from_secs(3600);
    let _ = node
        .eth_rpc(EthRequest::EvmMine(Some(Params {
            params: Some(MineOptions::Timestamp(Some(timestamp.as_secs()))),
        })))
        .await
        .unwrap();
    assert_eq!(node.best_block_number().await, 1);
    let hash1 = node.block_hash_by_number(1).await.unwrap();
    let timestamp1 = node.get_decoded_timestamp(Some(hash1)).await;
    assert_eq!(timestamp1.saturating_div(1000), timestamp.as_secs());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_evm_mine_option_none() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config, substrate_node_config).await.unwrap();

    let _ = node
        .eth_rpc(EthRequest::EvmMine(Some(Params {
            params: Some(MineOptions::Options { timestamp: None, blocks: None }),
        })))
        .await
        .unwrap();
    assert_eq!(node.best_block_number().await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_evm_mine_options_timestamp() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config, substrate_node_config).await.unwrap();

    let timestamp = duration_since_unix_epoch() + Duration::from_secs(3600);
    let _ = node
        .eth_rpc(EthRequest::EvmMine(Some(Params {
            params: Some(MineOptions::Options {
                timestamp: Some(timestamp.as_secs()),
                blocks: None,
            }),
        })))
        .await
        .unwrap();
    assert_eq!(node.best_block_number().await, 1);
    let hash1 = node.block_hash_by_number(1).await.unwrap();
    let timestamp1 = node.get_decoded_timestamp(Some(hash1)).await;
    assert_eq!(timestamp1.saturating_div(1000), timestamp.as_secs());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_evm_mine_options_blocks() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config, substrate_node_config).await.unwrap();
    let _ = node
        .eth_rpc(EthRequest::EvmMine(Some(Params {
            params: Some(MineOptions::Options { timestamp: None, blocks: Some(3) }),
        })))
        .await
        .unwrap();

    assert_eq!(node.best_block_number().await, 3);
    let _ = node
        .eth_rpc(EthRequest::EvmMine(Some(Params {
            params: Some(MineOptions::Options { timestamp: None, blocks: Some(0) }),
        })))
        .await
        .unwrap();
    assert_eq!(node.best_block_number().await, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_evm_mine_detailed() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config, substrate_node_config).await.unwrap();

    assert!(matches!(
        node.eth_rpc(EthRequest::EvmMineDetailed(None)).await.unwrap(),
        ResponseResult::Error(RpcError { code: ErrorCode::InternalError, .. })
    ));
}
