//! Tests for the block-and-index transaction and uncle endpoints.
//!
//! Anvil never produces uncles, so the uncle endpoints exist for compatibility and forward to the
//! forked node for blocks that predate the fork.

use crate::utils::http_provider_with_signer;
use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, Index, TransactionRequest};
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, spawn};
use foundry_test_utils::rpc::spawn_rpc_proxy_canned_method;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

/// Mines a block holding a single transfer and returns its hash, number and transaction hash.
async fn mine_block_with_transfer(handle: &anvil::NodeHandle) -> (B256, u64, B256) {
    let wallet = handle.dev_wallets().next().unwrap();
    let sender = wallet.address();
    let signer: EthereumWallet = wallet.into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let receipt = provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .with_from(sender)
                .with_to(Address::repeat_byte(0x11))
                .with_value(U256::from(1)),
        ))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let block = provider.get_block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    (block.header.hash, block.header.number, receipt.transaction_hash)
}

#[tokio::test(flavor = "multi_thread")]
async fn transaction_by_block_hash_and_index_matches_the_mined_transaction() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let (block_hash, _number, tx_hash) = mine_block_with_transfer(&handle).await;
    let provider = handle.http_provider();

    let tx: Option<Value> = provider
        .client()
        .request("eth_getTransactionByBlockHashAndIndex", (block_hash, Index::from(0)))
        .await
        .unwrap();
    let tx = tx.expect("expected a transaction at index 0");
    assert_eq!(tx["hash"].as_str().unwrap().parse::<B256>().unwrap(), tx_hash);

    // Indexing past the end of the block is not an error.
    let missing: Option<Value> = provider
        .client()
        .request("eth_getTransactionByBlockHashAndIndex", (block_hash, Index::from(1)))
        .await
        .unwrap();
    assert_eq!(missing, None);

    let unknown_block: Option<Value> = provider
        .client()
        .request("eth_getTransactionByBlockHashAndIndex", (B256::random(), Index::from(0)))
        .await
        .unwrap();
    assert_eq!(unknown_block, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_transaction_by_block_and_index_matches_by_hash() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let (block_hash, number, tx_hash) = mine_block_with_transfer(&handle).await;
    let provider = handle.http_provider();

    let expected: Option<Bytes> =
        provider.client().request("eth_getRawTransactionByHash", (tx_hash,)).await.unwrap();
    let expected = expected.expect("expected a raw transaction");

    let by_hash: Option<Bytes> = provider
        .client()
        .request("eth_getRawTransactionByBlockHashAndIndex", (block_hash, Index::from(0)))
        .await
        .unwrap();
    assert_eq!(by_hash, Some(expected.clone()));

    let by_number: Option<Bytes> = provider
        .client()
        .request(
            "eth_getRawTransactionByBlockNumberAndIndex",
            (BlockNumberOrTag::Number(number), Index::from(0)),
        )
        .await
        .unwrap();
    assert_eq!(by_number, Some(expected));

    // Out-of-range indices report no transaction rather than failing.
    let missing: Option<Bytes> = provider
        .client()
        .request(
            "eth_getRawTransactionByBlockNumberAndIndex",
            (BlockNumberOrTag::Number(number), Index::from(7)),
        )
        .await
        .unwrap();
    assert_eq!(missing, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_transaction_by_block_number_and_index() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let sender = wallet.address();
    let signer: EthereumWallet = wallet.into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);
    api.anvil_set_auto_mine(false).await.unwrap();

    let pending = provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .with_from(sender)
                .with_to(Address::repeat_byte(0x11))
                .with_value(U256::from(1)),
        ))
        .await
        .unwrap()
        .register()
        .await
        .unwrap();
    let tx_hash = *pending.tx_hash();

    let tx: Option<Value> = provider
        .client()
        .request(
            "eth_getTransactionByBlockNumberAndIndex",
            (BlockNumberOrTag::Pending, Index::from(0)),
        )
        .await
        .unwrap();
    let tx = tx.expect("expected a pending transaction at index 0");
    assert_eq!(tx["hash"].as_str().unwrap().parse::<B256>().unwrap(), tx_hash);

    let expected: Option<Bytes> =
        provider.client().request("eth_getRawTransactionByHash", (tx_hash,)).await.unwrap();
    let expected = expected.expect("expected a raw pending transaction");
    let raw: Option<Bytes> = provider
        .client()
        .request(
            "eth_getRawTransactionByBlockNumberAndIndex",
            (BlockNumberOrTag::Pending, Index::from(0)),
        )
        .await
        .unwrap();
    assert_eq!(raw, Some(expected));
}

#[tokio::test(flavor = "multi_thread")]
async fn uncle_endpoints_report_no_uncles() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    api.mine_one().await.unwrap();
    let block = provider.get_block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    let (hash, number) = (block.header.hash, block.header.number);

    let count: U256 =
        provider.client().request("eth_getUncleCountByBlockHash", (hash,)).await.unwrap();
    assert_eq!(count, U256::ZERO);

    let count: U256 = provider
        .client()
        .request("eth_getUncleCountByBlockNumber", (BlockNumberOrTag::Number(number),))
        .await
        .unwrap();
    assert_eq!(count, U256::ZERO);

    let uncle: Option<Value> = provider
        .client()
        .request("eth_getUncleByBlockHashAndIndex", (hash, Index::from(0)))
        .await
        .unwrap();
    assert_eq!(uncle, None);

    let uncle: Option<Value> = provider
        .client()
        .request(
            "eth_getUncleByBlockNumberAndIndex",
            (BlockNumberOrTag::Number(number), Index::from(0)),
        )
        .await
        .unwrap();
    assert_eq!(uncle, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn uncle_count_rejects_unknown_blocks() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    // `BlockNotFound` surfaces as the JSON-RPC resource-not-found code.
    let err = provider
        .client()
        .request::<_, U256>("eth_getUncleCountByBlockHash", (B256::random(),))
        .await
        .unwrap_err();
    assert_eq!(err.as_error_resp().unwrap().code, -32001);

    let err = provider
        .client()
        .request::<_, U256>("eth_getUncleCountByBlockNumber", (BlockNumberOrTag::Number(9999),))
        .await
        .unwrap_err();
    assert_eq!(err.as_error_resp().unwrap().code, -32001);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_uncle_by_index_skips_locally_mined_blocks() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let (proxy, calls) = spawn_rpc_proxy_canned_method(
        origin_handle.http_endpoint(),
        "eth_getUncleByBlockNumberAndIndex",
        json!(null),
    )
    .await;

    let (api, handle) = spawn(NodeConfig::test().with_eth_rpc_url(Some(proxy))).await;
    let provider = handle.http_provider();

    api.mine_one().await.unwrap();
    let local = provider.get_block_number().await.unwrap();

    let uncle: Option<Value> = provider
        .client()
        .request(
            "eth_getUncleByBlockNumberAndIndex",
            (BlockNumberOrTag::Number(local), Index::from(0)),
        )
        .await
        .unwrap();
    assert_eq!(uncle, None);
    assert_eq!(calls.load(Ordering::Relaxed), 0, "locally mined block should not be forwarded");
}
