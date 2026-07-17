//! Tests for activity simulation (`--activity` / `anvil_setActivity`).

use alloy_network::ReceiptResponse;
use alloy_rpc_types::{BlockNumberOrTag, Filter};
use anvil::{
    NodeConfig,
    eth::activity::{ACTIVITY_ADDRESS, ACTIVITY_TOKEN_ADDRESS},
    spawn,
};
use anvil_core::types::{ActivityOptions, ActivityRange};
use std::time::Duration;

fn activity_config() -> ActivityOptions {
    ActivityOptions {
        txs: ActivityRange { min: 4, max: 8 },
        reverted: 25,
        pending: 15,
        seed: Some(42),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn generates_activity() {
    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_blocktime(Some(Duration::from_millis(300)))
            .with_activity(Some(activity_config())),
    )
    .await;

    // Let a few blocks of activity accumulate.
    tokio::time::sleep(Duration::from_secs(5)).await;

    let height = api.block_number().unwrap().to::<u64>();
    assert!(height > 3, "expected blocks to be mined, got {height}");

    // Blocks contain generated transactions.
    let mut total_txs = 0;
    let mut failed = 0;
    for number in 1..=height {
        let receipts =
            api.block_receipts(BlockNumberOrTag::Number(number).into()).await.unwrap().unwrap();
        total_txs += receipts.len();
        failed += receipts.iter().filter(|receipt| !receipt.status()).count();
    }
    assert!(total_txs > 10, "expected generated transactions, got {total_txs}");
    assert!(failed > 0, "expected reverted transactions among {total_txs}");

    // Activity contract and mock ERC20 emit logs.
    let filter = Filter::new().address(ACTIVITY_ADDRESS).from_block(0);
    assert!(!api.logs(filter).await.unwrap().is_empty());
    let filter = Filter::new().address(ACTIVITY_TOKEN_ADDRESS).from_block(0);
    assert!(!api.logs(filter).await.unwrap().is_empty());

    // Gapped-nonce transactions stay queued and never mine.
    let content = api.txpool_content().await.unwrap();
    assert!(!content.queued.is_empty(), "expected queued (pending-forever) transactions");

    // Disabling stops injection.
    api.anvil_set_activity(None).unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let stop_height = api.block_number().unwrap().to::<u64>();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let final_height = api.block_number().unwrap().to::<u64>();
    let mut new_txs = 0;
    for number in (stop_height + 1)..=final_height {
        let receipts =
            api.block_receipts(BlockNumberOrTag::Number(number).into()).await.unwrap().unwrap();
        new_txs += receipts.len();
    }
    assert_eq!(new_txs, 0, "expected no new transactions after disabling activity");
}

#[tokio::test(flavor = "multi_thread")]
async fn generates_tempo_activity() {
    let (api, _handle) = spawn(
        NodeConfig::test_tempo()
            .with_blocktime(Some(Duration::from_millis(300)))
            .with_activity(Some(activity_config())),
    )
    .await;

    tokio::time::sleep(Duration::from_secs(5)).await;

    let height = api.block_number().unwrap().to::<u64>();
    let mut total_txs = 0;
    for number in 1..=height {
        let receipts =
            api.block_receipts(BlockNumberOrTag::Number(number).into()).await.unwrap().unwrap();
        total_txs += receipts.len();
    }
    assert!(total_txs > 10, "expected generated transactions, got {total_txs}");

    // TIP-20 fee tokens emit Transfer logs from generated traffic.
    let filter = Filter::new().address(foundry_common::tempo::PATH_USD_ADDRESS).from_block(0);
    assert!(!api.logs(filter).await.unwrap().is_empty(), "expected TIP-20 transfer logs");

    // Activity contract still emits logs on Tempo.
    let filter = Filter::new().address(ACTIVITY_ADDRESS).from_block(0);
    assert!(!api.logs(filter).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn activity_rpc_roundtrip() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    assert_eq!(api.anvil_get_activity().unwrap(), None);

    let config = activity_config();
    api.anvil_set_activity(Some(config.clone())).unwrap();
    assert_eq!(api.anvil_get_activity().unwrap(), Some(config));

    api.anvil_set_activity(None).unwrap();
    assert_eq!(api.anvil_get_activity().unwrap(), None);
}
