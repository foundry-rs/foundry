//! various fork related test

use crate::{
    abi::{ERC721, Greeter},
    utils::{http_provider, http_provider_with_signer},
};
use alloy_chains::NamedChain;
use alloy_eips::{
    calc_next_block_base_fee,
    eip1559::BaseFeeParams,
    eip2718::Decodable2718,
    eip7840::BlobParams,
    eip7910::{EthConfig, SystemContract},
};
use alloy_genesis::Genesis;
use alloy_network::{EthereumWallet, ReceiptResponse, TransactionBuilder, TransactionResponse};
use alloy_primitives::{
    Address, B256, Bytes, TxHash, TxKind, U64, U256, address, b256, bytes, hex, uint,
};
use alloy_provider::{
    Provider,
    ext::{DebugApi, TxPoolApi},
};
use alloy_rpc_types::{
    AccountInfo, BlockId, BlockNumberOrTag, Index,
    anvil::Forking,
    request::{TransactionInput, TransactionRequest},
    state::EvmOverrides,
    trace::{
        geth::{CallConfig, GethDebugTracingOptions, GethTrace, TraceResult},
        parity::{Action, TraceResultsWithTransactionHash, TraceType},
    },
};
use alloy_serde::WithOtherFields;
use alloy_signer_local::PrivateKeySigner;
use anvil::{
    EthereumHardfork, NodeConfig, NodeHandle, PrecompileFactory,
    eth::{EthApi, fees::INITIAL_BASE_FEE},
    spawn, try_spawn,
};
use axum::{Json, Router, routing::post};
use foundry_common::provider::get_http_provider;
use foundry_config::Config;
use foundry_evm::hardfork::OpHardfork;
use foundry_evm_networks::NetworkConfigs;
use foundry_primitives::{FoundryNetwork, FoundryReceiptEnvelope};
use foundry_test_utils::rpc::{
    self, next_http_rpc_endpoint, next_rpc_endpoint, spawn_rpc_proxy_internal_error_after,
    spawn_rpc_proxy_method_not_found_before, spawn_rpc_proxy_rejecting_method_after,
    spawn_rpc_proxy_rejecting_method_when_enabled,
    spawn_rpc_proxy_retyping_first_block_transaction,
};
use futures::StreamExt;
use revm::{
    context::BlockEnv, context_interface::block::BlobExcessGasAndPrice,
    precompile::PrecompileStatus, primitives::hardfork::SpecId,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

const BLOCK_NUMBER: u64 = 14_608_400u64;
const DEAD_BALANCE_AT_BLOCK_NUMBER: u128 = 12_556_069_338_441_120_059_867u128;

const BLOCK_TIMESTAMP: u64 = 1_650_274_250u64;

/// Represents an anvil fork of an anvil node
#[expect(unused)]
pub struct LocalFork {
    origin_api: EthApi<FoundryNetwork>,
    origin_handle: NodeHandle,
    fork_api: EthApi<FoundryNetwork>,
    fork_handle: NodeHandle,
}

#[expect(dead_code)]
impl LocalFork {
    /// Spawns two nodes with the test config
    pub async fn new() -> Self {
        Self::setup(NodeConfig::test(), NodeConfig::test()).await
    }

    /// Spawns two nodes where one is a fork of the other
    pub async fn setup(origin: NodeConfig, fork: NodeConfig) -> Self {
        let (origin_api, origin_handle) = spawn(origin).await;

        let (fork_api, fork_handle) =
            spawn(fork.with_eth_rpc_url(Some(origin_handle.http_endpoint()))).await;
        Self { origin_api, origin_handle, fork_api, fork_handle }
    }
}

pub fn fork_config() -> NodeConfig {
    NodeConfig::test()
        .with_eth_rpc_url(Some(rpc::next_http_archive_rpc_url()))
        .with_fork_block_number(Some(BLOCK_NUMBER))
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_ignores_initial_anvil_node_info_rpc_error() {
    let (_api, origin) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::Mainnet as u64))).await;
    let fork_url =
        spawn_rpc_proxy_internal_error_after(origin.http_endpoint(), "anvil_nodeInfo", 0).await;

    let (api, _handle) = spawn(NodeConfig::test().with_eth_rpc_url(Some(fork_url))).await;

    assert_eq!(api.chain_id(), NamedChain::Mainnet as u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_rejects_node_info_failure_after_anvil_identification() {
    let (_api, origin) = spawn(NodeConfig::test()).await;
    let fork_url =
        spawn_rpc_proxy_rejecting_method_after(origin.http_endpoint(), "anvil_nodeInfo", 1).await;

    let result = try_spawn(NodeConfig::test().with_eth_rpc_url(Some(fork_url))).await;
    let Err(error) = result else { panic!("expected fork startup to fail") };

    assert!(
        error.to_string().contains("failed to determine network family from fork endpoint"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_keeps_node_info_probe_strict_after_identification_during_staging() {
    let (_initial_api, initial_origin) = spawn(NodeConfig::test()).await;
    let (api, _handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(initial_origin.http_endpoint()))).await;
    let (_target_api, target_origin) = spawn(NodeConfig::test()).await;
    let fork_url =
        spawn_rpc_proxy_rejecting_method_after(target_origin.http_endpoint(), "anvil_nodeInfo", 1)
            .await;

    let error = api
        .anvil_reset(Some(Forking { json_rpc_url: Some(fork_url), block_number: None }))
        .await
        .unwrap_err();

    assert!(
        error.to_string().contains("failed to determine network family from fork endpoint"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_keeps_cached_anvil_node_info_probe_strict() {
    let (_origin_api, origin) = spawn(NodeConfig::test()).await;
    let (fork_url, reject_node_info) =
        spawn_rpc_proxy_rejecting_method_when_enabled(origin.http_endpoint(), "anvil_nodeInfo")
            .await;
    let (api, _handle) = spawn(NodeConfig::test().with_eth_rpc_url(Some(fork_url.clone()))).await;
    api.anvil_reset(None).await.unwrap();
    reject_node_info.store(true, Ordering::SeqCst);

    let error = api
        .anvil_reset(Some(Forking { json_rpc_url: Some(fork_url), block_number: None }))
        .await
        .unwrap_err();

    assert!(
        error.to_string().contains("failed to determine network family from fork endpoint"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_does_not_carry_anvil_identity_to_new_endpoint() {
    let (_initial_api, initial_origin) = spawn(NodeConfig::test()).await;
    let (api, handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(initial_origin.http_endpoint()))).await;
    let (_target_api, target_origin) = spawn(NodeConfig::test().with_chain_id(Some(56u64))).await;
    let target_url =
        spawn_rpc_proxy_rejecting_method_after(target_origin.http_endpoint(), "anvil_nodeInfo", 0)
            .await;

    api.anvil_reset(Some(Forking { json_rpc_url: Some(target_url), block_number: None }))
        .await
        .unwrap();

    assert_eq!(handle.http_provider().get_chain_id().await.unwrap(), 56);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_keeps_set_rpc_url_anvil_node_info_probe_strict() {
    let (_initial_api, initial_origin) = spawn(NodeConfig::test()).await;
    let (api, _handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(initial_origin.http_endpoint()))).await;
    let (replacement_url, reject_node_info) = spawn_rpc_proxy_rejecting_method_when_enabled(
        initial_origin.http_endpoint(),
        "anvil_nodeInfo",
    )
    .await;
    api.anvil_set_rpc_url(replacement_url).await.unwrap();
    reject_node_info.store(true, Ordering::SeqCst);

    let error = api
        .anvil_reset(Some(Forking { json_rpc_url: None, block_number: None }))
        .await
        .unwrap_err();

    assert!(
        error.to_string().contains("failed to determine network family from fork endpoint"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_keeps_validated_mirror_anvil_node_info_probe_strict() {
    let (_origin_api, origin) = spawn(NodeConfig::test()).await;
    let primary_url = origin.http_endpoint();
    let (mirror_url, reject_node_info) =
        spawn_rpc_proxy_rejecting_method_when_enabled(primary_url.clone(), "anvil_nodeInfo").await;
    let (api, _handle) =
        spawn(NodeConfig::test().with_fork_urls(vec![primary_url, mirror_url.clone()])).await;
    reject_node_info.store(true, Ordering::SeqCst);

    let error = api
        .anvil_reset(Some(Forking { json_rpc_url: Some(mirror_url), block_number: None }))
        .await
        .unwrap_err();

    assert!(
        error.to_string().contains("failed to determine network family from fork endpoint"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_retries_when_anvil_node_info_becomes_available() {
    let (_api, origin) = spawn(NodeConfig::test()).await;
    let fork_url =
        spawn_rpc_proxy_method_not_found_before(origin.http_endpoint(), "anvil_nodeInfo", 1).await;

    try_spawn(NodeConfig::test().with_eth_rpc_url(Some(fork_url))).await.unwrap();
}

// A replayed block holds only the transactions anvil executed, so a skipped one must not leave a
// gap in the indices storage keys receipts by. The proxy stands in for Arbitrum, which opens every
// block with a system transaction anvil cannot execute.
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_transaction_hash_replay_skips_unsupported_prefix() {
    let (origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    origin_api.anvil_set_auto_mine(false).await.unwrap();
    let origin_provider = origin_handle.http_provider();
    // Distinct senders, because Arbitrum's system transactions do not consume a user nonce.
    let senders =
        origin_handle.dev_wallets().take(2).map(|wallet| wallet.address()).collect::<Vec<_>>();

    let skipped = origin_provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .from(senders[0])
                .to(Address::random())
                .value(U256::from(1))
                .nonce(0),
        ))
        .await
        .unwrap();
    let target = origin_provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .from(senders[1])
                .to(Address::random())
                .value(U256::from(2))
                .nonce(0),
        ))
        .await
        .unwrap();
    let target_hash = *target.tx_hash();
    origin_api.mine_one().await.unwrap();

    let fork_url = spawn_rpc_proxy_retyping_first_block_transaction(
        origin_handle.http_endpoint(),
        // `ArbitrumInternalTx`.
        "0x6a",
    )
    .await;
    let (fork_api, fork_handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(fork_url))
            .with_fork_transaction_hash(Some(target_hash))
            .with_no_mining(true),
    )
    .await;
    let fork_provider = fork_handle.http_provider();

    // Only the target survives the prefix, so it takes index 0 in the replayed block.
    let replayed =
        fork_api.block_by_number_full(BlockNumberOrTag::Number(1)).await.unwrap().unwrap();
    assert_eq!(
        replayed
            .transactions
            .as_transactions()
            .unwrap()
            .iter()
            .map(|tx| tx.tx_hash())
            .collect::<Vec<_>>(),
        vec![target_hash]
    );

    // Receipt lookups index into that block, so a stale index panics instead of answering.
    let receipt = fork_provider.get_transaction_receipt(target_hash).await.unwrap().unwrap();
    assert_eq!(receipt.transaction_index(), Some(0));
    assert_eq!(
        fork_provider.get_block_receipts(BlockId::number(1)).await.unwrap().unwrap().len(),
        1
    );
    assert!(fork_api.backend.mined_transaction_by_hash(*skipped.tx_hash()).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_transaction_hash_replays_before_startup() {
    let (origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    origin_api.anvil_set_auto_mine(false).await.unwrap();
    let origin_provider = origin_handle.http_provider();
    let sender = origin_handle.dev_wallets().next().unwrap().address();

    let first = origin_provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .from(sender)
                .to(Address::random())
                .value(U256::from(1))
                .nonce(0),
        ))
        .await
        .unwrap();
    let second = origin_provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .from(sender)
                .to(Address::random())
                .value(U256::from(2))
                .nonce(1),
        ))
        .await
        .unwrap();
    let reverted = origin_provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .from(sender)
                .with_input(hex!("60006000fd").to_vec())
                .gas_limit(100_000)
                .nonce(2),
        ))
        .await
        .unwrap();
    let expected_hashes = [*first.tx_hash(), *second.tx_hash(), *reverted.tx_hash()];
    origin_api.mine_one().await.unwrap();
    assert!(
        !origin_provider
            .get_transaction_receipt(expected_hashes[2])
            .await
            .unwrap()
            .unwrap()
            .status()
    );

    let (fork_api, fork_handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
            .with_fork_transaction_hash(Some(expected_hashes[2]))
            .with_no_mining(true),
    )
    .await;
    let fork_provider = fork_handle.http_provider();

    assert_eq!(fork_api.block_number().unwrap(), U256::from(1));
    let replayed =
        fork_api.block_by_number_full(BlockNumberOrTag::Number(1)).await.unwrap().unwrap();
    let replayed_hashes = replayed
        .transactions
        .as_transactions()
        .unwrap()
        .iter()
        .map(|tx| tx.tx_hash())
        .collect::<Vec<_>>();
    assert_eq!(replayed_hashes, expected_hashes);
    assert_eq!(fork_provider.txpool_status().await.unwrap().pending, 0);
    assert_eq!(fork_provider.txpool_status().await.unwrap().queued, 0);
    let content = fork_provider.txpool_content().await.unwrap();
    assert!(content.pending.is_empty());
    assert!(content.queued.is_empty());
    assert_eq!(fork_provider.get_transaction_count(sender).await.unwrap(), 3);
    assert!(
        !fork_provider.get_transaction_receipt(expected_hashes[2]).await.unwrap().unwrap().status()
    );

    let (prefix_api, _) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
            .with_fork_transaction_hash(Some(expected_hashes[1]))
            .with_no_mining(true),
    )
    .await;
    let prefix =
        prefix_api.block_by_number_full(BlockNumberOrTag::Number(1)).await.unwrap().unwrap();
    assert_eq!(
        prefix
            .transactions
            .as_transactions()
            .unwrap()
            .iter()
            .map(|tx| tx.tx_hash())
            .collect::<Vec<_>>(),
        expected_hashes[..2]
    );
    assert!(prefix_api.backend.mined_transaction_by_hash(expected_hashes[2]).is_none());

    let (pruned_api, _) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
            .with_fork_transaction_hash(Some(expected_hashes[0]))
            .with_transaction_block_keeper(Some(0usize))
            .with_no_mining(true),
    )
    .await;
    assert!(pruned_api.backend.mined_transaction_by_hash(expected_hashes[0]).is_none());

    let _pending = fork_provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .from(sender)
                .to(Address::random())
                .value(U256::from(3))
                .nonce(3),
        ))
        .await
        .unwrap();
    let pool = fork_provider.txpool_status().await.unwrap();
    assert_eq!(pool.pending, 1);
    assert_eq!(pool.queued, 0);

    let (auto_mining_api, _) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
            .with_fork_transaction_hash(Some(expected_hashes[2])),
    )
    .await;
    let replayed =
        auto_mining_api.block_by_number_full(BlockNumberOrTag::Number(1)).await.unwrap().unwrap();
    assert_eq!(
        replayed
            .transactions
            .as_transactions()
            .unwrap()
            .iter()
            .map(|tx| tx.tx_hash())
            .collect::<Vec<_>>(),
        expected_hashes
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_transaction_hash_replay_preserves_cancun_header_inputs() {
    let (origin_api, origin_handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()))).await;
    origin_api.anvil_set_auto_mine(false).await.unwrap();
    let origin_provider = origin_handle.http_provider();
    let sender = origin_handle.dev_wallets().next().unwrap().address();
    let genesis =
        origin_api.block_by_number_full(BlockNumberOrTag::Number(0)).await.unwrap().unwrap();
    let source_timestamp = genesis.header.timestamp + 100;
    origin_api.evm_set_next_block_timestamp(source_timestamp).unwrap();

    let first = origin_provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .from(sender)
                .to(Address::random())
                .value(U256::from(1))
                .nonce(0),
        ))
        .await
        .unwrap();
    let second = origin_provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .from(sender)
                .to(Address::random())
                .value(U256::from(2))
                .nonce(1),
        ))
        .await
        .unwrap();
    origin_api.mine_one().await.unwrap();
    let source =
        origin_api.block_by_number_full(BlockNumberOrTag::Number(1)).await.unwrap().unwrap();
    assert_eq!(source.header.timestamp, source_timestamp);

    let (fork_api, _) = spawn(
        NodeConfig::test()
            .with_hardfork(Some(EthereumHardfork::Cancun.into()))
            .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
            .with_fork_transaction_hash(Some(*second.tx_hash()))
            .with_no_mining(true),
    )
    .await;
    let replayed =
        fork_api.block_by_number_full(BlockNumberOrTag::Number(1)).await.unwrap().unwrap();

    assert_eq!(
        replayed
            .transactions
            .as_transactions()
            .unwrap()
            .iter()
            .map(|tx| tx.tx_hash())
            .collect::<Vec<_>>(),
        [*first.tx_hash(), *second.tx_hash()]
    );
    assert_eq!(replayed.header.timestamp, source.header.timestamp);
    assert_eq!(replayed.header.parent_beacon_block_root, source.header.parent_beacon_block_root);

    fork_api.mine_one().await.unwrap();
    let next = fork_api.block_by_number_full(BlockNumberOrTag::Number(2)).await.unwrap().unwrap();
    assert!(next.header.timestamp > source.header.timestamp);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_transaction_hash_replay_resolves_source_hardfork() {
    // Real chain ID so the hardfork can be resolved from chain + timestamp.
    const MAINNET_CHAIN_ID: u64 = 1;
    // Well before Cancun's 2024-03-13 activation.
    const SHANGHAI_ERA_TIMESTAMP: u64 = 1_690_000_000u64;

    let (origin_api, origin_handle) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(MAINNET_CHAIN_ID))
            .with_hardfork(Some(EthereumHardfork::Shanghai.into()))
            .with_genesis_timestamp(Some(SHANGHAI_ERA_TIMESTAMP - 100)),
    )
    .await;
    origin_api.anvil_set_auto_mine(false).await.unwrap();
    origin_api.evm_set_next_block_timestamp(SHANGHAI_ERA_TIMESTAMP).unwrap();
    let origin_provider = origin_handle.http_provider();
    let sender = origin_handle.dev_wallets().next().unwrap().address();

    let target = origin_provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default().from(sender).to(Address::random()).value(U256::from(1)),
        ))
        .await
        .unwrap();
    let target_hash = *target.tx_hash();
    origin_api.mine_one().await.unwrap();
    let source =
        origin_api.block_by_number_full(BlockNumberOrTag::Number(1)).await.unwrap().unwrap();
    assert_eq!(source.header.timestamp, SHANGHAI_ERA_TIMESTAMP);
    assert!(source.header.parent_beacon_block_root.is_none());

    // A newer explicit `--hardfork` used to wrongly require a `parentBeaconBlockRoot` on this
    // pre-Cancun source block, failing startup.
    let (fork_api, _fork_handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
            .with_fork_transaction_hash(Some(target_hash))
            .with_hardfork(Some(EthereumHardfork::Prague.into()))
            .with_no_mining(true),
    )
    .await;

    let replayed =
        fork_api.block_by_number_full(BlockNumberOrTag::Number(1)).await.unwrap().unwrap();
    assert_eq!(
        replayed
            .transactions
            .as_transactions()
            .unwrap()
            .iter()
            .map(|tx| tx.tx_hash())
            .collect::<Vec<_>>(),
        [target_hash]
    );
    assert!(replayed.header.parent_beacon_block_root.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_transaction_hash_replay_applies_source_beacon_root() {
    // Real chain ID so the hardfork can be resolved from chain + timestamp.
    const MAINNET_CHAIN_ID: u64 = 1;
    // Past Cancun's 2024-03-13 activation.
    const CANCUN_ERA_TIMESTAMP: u64 = 1_720_000_000u64;
    const HISTORY_BUFFER_LENGTH: u64 = 8191;

    let (origin_api, origin_handle) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(MAINNET_CHAIN_ID))
            .with_hardfork(Some(EthereumHardfork::Cancun.into()))
            .with_genesis_timestamp(Some(CANCUN_ERA_TIMESTAMP - 100)),
    )
    .await;
    origin_api.anvil_set_auto_mine(false).await.unwrap();
    origin_api.evm_set_next_block_timestamp(CANCUN_ERA_TIMESTAMP).unwrap();
    let origin_provider = origin_handle.http_provider();
    let sender = origin_handle.dev_wallets().next().unwrap().address();

    let target = origin_provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default().from(sender).to(Address::random()).value(U256::from(1)),
        ))
        .await
        .unwrap();
    let target_hash = *target.tx_hash();
    origin_api.mine_one().await.unwrap();
    let source =
        origin_api.block_by_number_full(BlockNumberOrTag::Number(1)).await.unwrap().unwrap();
    assert_eq!(source.header.timestamp, CANCUN_ERA_TIMESTAMP);
    assert!(source.header.parent_beacon_block_root.is_some());

    // An older explicit `--hardfork` used to silently skip the EIP-4788 beacon-root call on this
    // Cancun-era source block.
    let (fork_api, _fork_handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
            .with_fork_transaction_hash(Some(target_hash))
            .with_hardfork(Some(EthereumHardfork::Shanghai.into()))
            .with_no_mining(true),
    )
    .await;

    let replayed =
        fork_api.block_by_number_full(BlockNumberOrTag::Number(1)).await.unwrap().unwrap();
    assert_eq!(replayed.header.parent_beacon_block_root, source.header.parent_beacon_block_root);

    let slot = U256::from(CANCUN_ERA_TIMESTAMP % HISTORY_BUFFER_LENGTH);
    let stored_timestamp = fork_api
        .storage_at(
            alloy_eips::eip4788::BEACON_ROOTS_ADDRESS,
            slot,
            Some(BlockId::Number(BlockNumberOrTag::Number(1))),
        )
        .await
        .unwrap();
    assert_eq!(stored_timestamp, B256::from(U256::from(CANCUN_ERA_TIMESTAMP)));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_transaction_hash_replay_error_fails_startup() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let origin_provider = origin_handle.http_provider();
    let sender = origin_handle.dev_wallets().next().unwrap().address();
    let pending = origin_provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default().from(sender).to(Address::random()).value(U256::from(1)),
        ))
        .await
        .unwrap();
    let transaction_hash = *pending.tx_hash();
    pending.get_receipt().await.unwrap();

    let config = NodeConfig::test()
        .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
        .with_fork_transaction_hash(Some(transaction_hash))
        .with_gas_limit(Some(20_000));
    let cache_path = config.block_cache_path(0).unwrap();
    match std::fs::remove_file(&cache_path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => panic!("failed to clear stale fork cache: {err}"),
    }

    let result = try_spawn(config).await;
    let Err(error) = result else { panic!("expected fork transaction replay to fail") };
    let message = error.to_string();
    assert!(message.contains("failed to replay fork transaction prefix"));
    assert!(format!("{error:?}").contains(&transaction_hash.to_string()));
    assert!(!cache_path.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_gas_limit_applied_from_config() {
    let (api, _handle) = spawn(fork_config().with_gas_limit(Some(10_000_000))).await;

    assert_eq!(api.gas_limit(), uint!(10_000_000_U256));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_gas_limit_disabled_from_config() {
    let (api, handle) = spawn(fork_config().disable_block_gas_limit(true)).await;

    // see https://github.com/foundry-rs/foundry/pull/8933
    assert_eq!(api.gas_limit(), U256::from(U64::MAX));

    // try to mine a couple blocks
    let provider = handle.http_provider();
    let tx = TransactionRequest::default()
        .to(Address::random())
        .value(U256::from(1337u64))
        .from(handle.dev_wallets().next().unwrap().address());
    let tx = WithOtherFields::new(tx);
    let _ = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let tx = TransactionRequest::default()
        .to(Address::random())
        .value(U256::from(1337u64))
        .from(handle.dev_wallets().next().unwrap().address());
    let tx = WithOtherFields::new(tx);
    let _ = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_refreshes_derived_gas_settings() {
    let chain_id = 7_654_321u64;
    let first_gas_limit = 21_000_001u64;
    let first_base_fee = 101u64;
    let first_gas_price = 111u128;
    let second_gas_limit = 22_000_002u64;
    let second_base_fee = 202u64;
    let second_gas_price = 222u128;
    let (first_api, first_handle) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_gas_limit(Some(first_gas_limit))
            .with_base_fee(Some(first_base_fee))
            .with_gas_price(Some(first_gas_price)),
    )
    .await;
    let (second_api, second_handle) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_gas_limit(Some(second_gas_limit))
            .with_base_fee(Some(second_base_fee))
            .with_gas_price(Some(second_gas_price)),
    )
    .await;
    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_no_storage_caching(true)
            .with_eth_rpc_url(Some(first_handle.http_endpoint())),
    )
    .await;

    let first_info = api.anvil_node_info().await.unwrap();
    let first_next_base_fee =
        calc_next_block_base_fee(0, first_gas_limit, first_base_fee, BaseFeeParams::ethereum());
    assert_eq!(first_info.environment.gas_limit, first_gas_limit);
    assert_eq!(first_info.environment.base_fee, first_next_base_fee.into());
    assert_eq!(api.backend.fees().raw_gas_price(), first_api.gas_price());

    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(second_handle.http_endpoint()),
        block_number: Some(0),
    }))
    .await
    .unwrap();

    let second_info = api.anvil_node_info().await.unwrap();
    let second_next_base_fee =
        calc_next_block_base_fee(0, second_gas_limit, second_base_fee, BaseFeeParams::ethereum());
    assert_eq!(second_info.environment.gas_limit, second_gas_limit);
    assert_eq!(second_info.environment.base_fee, second_next_base_fee.into());
    assert_eq!(api.backend.fees().raw_gas_price(), second_api.gas_price());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_preserves_explicit_gas_settings_and_restores_memory() {
    let chain_id = 7_654_322u64;
    let explicit_gas_limit = 31_000_003u64;
    let explicit_base_fee = 303u64;
    let explicit_gas_price = 333u128;
    let (_first_api, first_handle) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_gas_limit(Some(21_000_001))
            .with_base_fee(Some(101))
            .with_gas_price(Some(111)),
    )
    .await;
    let (_second_api, second_handle) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_gas_limit(Some(22_000_002))
            .with_base_fee(Some(202))
            .with_gas_price(Some(222)),
    )
    .await;
    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_no_storage_caching(true)
            .with_gas_limit(Some(explicit_gas_limit))
            .with_base_fee(Some(explicit_base_fee))
            .with_gas_price(Some(explicit_gas_price))
            .with_eth_rpc_url(Some(first_handle.http_endpoint())),
    )
    .await;

    for url in [first_handle.http_endpoint(), second_handle.http_endpoint()] {
        api.anvil_reset(Some(Forking { json_rpc_url: Some(url), block_number: Some(0) }))
            .await
            .unwrap();
        let info = api.anvil_node_info().await.unwrap();
        assert_eq!(info.environment.gas_limit, explicit_gas_limit);
        assert_eq!(info.environment.base_fee, explicit_base_fee.into());
        assert_eq!(api.backend.fees().raw_gas_price(), explicit_gas_price);
    }

    api.anvil_reset(None).await.unwrap();
    let local_info = api.anvil_node_info().await.unwrap();
    assert!(local_info.fork_config.fork_url.is_none());
    assert_eq!(local_info.environment.gas_limit, explicit_gas_limit);
    assert_eq!(local_info.environment.base_fee, explicit_base_fee.into());
    assert_eq!(api.backend.fees().raw_gas_price(), explicit_gas_price);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_restores_implicit_memory_base_fee() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test().with_base_fee(Some(123))).await;
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_no_storage_caching(true)
            .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
            .with_fork_block_number(Some(0u64)),
    )
    .await;

    api.anvil_set_next_block_base_fee_per_gas(U256::from(999)).await.unwrap();
    api.anvil_reset(None).await.unwrap();

    let local_info = api.anvil_node_info().await.unwrap();
    assert!(local_info.fork_config.fork_url.is_none());
    assert_eq!(local_info.environment.base_fee, INITIAL_BASE_FEE.into());
    let genesis = handle.http_provider().get_block(BlockId::number(0)).await.unwrap().unwrap();
    assert_eq!(genesis.header.base_fee_per_gas, Some(INITIAL_BASE_FEE));
    api.mine_one().await.unwrap();
    let first = handle.http_provider().get_block(BlockId::number(1)).await.unwrap().unwrap();
    assert_eq!(first.header.base_fee_per_gas, Some(INITIAL_BASE_FEE));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_restores_explicit_genesis_base_fee() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test().with_base_fee(Some(123))).await;
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_no_storage_caching(true)
            .with_hardfork(Some(EthereumHardfork::default().into()))
            .with_genesis(Some(Genesis { base_fee_per_gas: Some(0), ..Default::default() }))
            .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
            .with_fork_block_number(Some(0u64)),
    )
    .await;

    api.anvil_set_next_block_base_fee_per_gas(U256::from(999)).await.unwrap();
    assert_eq!(api.anvil_node_info().await.unwrap().environment.base_fee, 999);

    api.anvil_reset(None).await.unwrap();

    let local_info = api.anvil_node_info().await.unwrap();
    assert!(local_info.fork_config.fork_url.is_none());
    assert_eq!(local_info.environment.base_fee, 0);
    let genesis = handle.http_provider().get_block(BlockId::number(0)).await.unwrap().unwrap();
    assert_eq!(genesis.header.base_fee_per_gas, Some(0));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_to_memory_rebuilds_complete_block_environment() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let (api, _handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_handle.http_endpoint()))).await;
    {
        let mut env = api.backend.evm_env().write();
        env.block_env.beneficiary = Address::random();
        env.block_env.difficulty = U256::from(123u64);
        env.block_env.blob_excess_gas_and_price = Some(BlobExcessGasAndPrice::new(456, 3_338_477));
    }

    api.anvil_reset(None).await.unwrap();

    let env = api.backend.evm_env().read();
    let default_block_env = BlockEnv::default();
    assert_eq!(env.block_env.beneficiary, Address::ZERO);
    assert_eq!(env.block_env.difficulty, U256::ZERO);
    assert_eq!(
        env.block_env.blob_excess_gas_and_price,
        default_block_env.blob_excess_gas_and_price
    );
}

// `debug_getRawReceipts` must serve pre-fork blocks from the upstream provider.
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_debug_get_raw_receipts() {
    let (_api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    // A pre-fork block known to contain transactions.
    let block_number = BLOCK_NUMBER - 1;
    let rpc_receipts =
        provider.get_block_receipts(BlockId::number(block_number)).await.unwrap().unwrap();
    assert!(!rpc_receipts.is_empty());

    let block = provider.get_block(BlockId::number(block_number)).await.unwrap().unwrap();
    let raw_by_number: Vec<Bytes> = provider
        .client()
        .request("debug_getRawReceipts", (BlockId::number(block_number),))
        .await
        .unwrap();
    let raw_by_hash: Vec<Bytes> = provider
        .client()
        .request("debug_getRawReceipts", (BlockId::hash(block.header.hash),))
        .await
        .unwrap();

    assert_eq!(raw_by_number, raw_by_hash);
    assert_eq!(raw_by_number.len(), rpc_receipts.len());

    // Each entry decodes back into a receipt envelope matching the RPC receipt.
    for (raw, rpc) in raw_by_number.iter().zip(rpc_receipts.iter()) {
        let decoded = FoundryReceiptEnvelope::decode_2718(&mut raw.as_ref()).unwrap();
        assert_eq!(decoded.status(), rpc.status());
    }
}

// `debug_getRawTransactions` must serve pre-fork blocks from the upstream provider.
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_debug_get_raw_transactions() {
    let (_api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    // A pre-fork block known to contain transactions.
    let block_number = BLOCK_NUMBER - 1;
    let block = provider.get_block(BlockId::number(block_number)).full().await.unwrap().unwrap();
    assert!(!block.transactions.is_empty());

    let raw_by_number: Vec<Bytes> = provider
        .client()
        .request("debug_getRawTransactions", (BlockId::number(block_number),))
        .await
        .unwrap();
    let raw_by_hash: Vec<Bytes> = provider
        .client()
        .request("debug_getRawTransactions", (BlockId::hash(block.header.hash),))
        .await
        .unwrap();

    assert_eq!(raw_by_number, raw_by_hash);
    assert_eq!(raw_by_number.len(), block.transactions.len());

    // Each entry matches the single-transaction raw encoding path for the same hash.
    for (raw, hash) in raw_by_number.iter().zip(block.transactions.hashes()) {
        let single: Bytes =
            provider.client().request("debug_getRawTransaction", (hash,)).await.unwrap();
        assert_eq!(*raw, single);
    }
}

// `debug_accountInfoAt` must delegate pre-fork blocks to the upstream and resolve block tags
// against the fork's frozen head, not the upstream's advancing head.
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_debug_account_info_at() {
    // Use a local anvil node as the upstream so we can advance it deterministically.
    let (origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let origin_provider = origin_handle.http_provider();

    let account = origin_handle.dev_wallets().next().unwrap().address();
    let to = Address::random();
    let amount = U256::from(1_000u64);

    // Mine one block on the upstream containing a single transfer to `to`.
    let tx = TransactionRequest::default().from(account).to(to).value(amount);
    let tx = WithOtherFields::new(tx);
    let receipt = origin_provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    let fork_block = receipt.block_number.unwrap();

    // Fork from the upstream at its current head.
    let (_fork_api, fork_handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_handle.http_endpoint()))).await;
    let fork_provider = fork_handle.http_provider();

    // Pre-fork block delegated by number and by hash returns the fork-point balance.
    let by_number: Option<AccountInfo> = fork_provider
        .raw_request(
            "debug_accountInfoAt".into(),
            (BlockId::number(fork_block), Index::from(0), to),
        )
        .await
        .unwrap();
    assert_eq!(by_number.unwrap().balance, amount);

    // Query via the `latest` tag: on the frozen fork this must resolve to `fork_block`.
    let by_tag: Option<AccountInfo> = fork_provider
        .raw_request("debug_accountInfoAt".into(), (BlockId::latest(), Index::from(0), to))
        .await
        .unwrap();
    assert_eq!(by_tag.unwrap().balance, amount);

    // Advance the upstream with more transfers to `to` so its `latest` head drifts ahead.
    for _ in 0..3 {
        let tx = TransactionRequest::default().from(account).to(to).value(amount);
        let tx = WithOtherFields::new(tx);
        origin_provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    }
    assert!(origin_api.block_number().unwrap() > U256::from(fork_block));

    // The fork never advanced, so `latest` must still resolve to `fork_block` and return the
    // fork-point balance rather than drifting with the upstream head.
    let by_tag_after: Option<AccountInfo> = fork_provider
        .raw_request("debug_accountInfoAt".into(), (BlockId::latest(), Index::from(0), to))
        .await
        .unwrap();
    assert_eq!(by_tag_after.unwrap().balance, amount);
}

// Pre-fork `trace_replayBlockTransactions` must be forwarded to the upstream with the trace types
// serialized as their camelCase JSON names (`trace`, `stateDiff`), not their Rust `Debug`
// representation, otherwise the upstream rejects the request.
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_trace_replay_block_transactions_forwards_trace_types() {
    // Use a local anvil node as the upstream so the request path is fully exercised in-process.
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let origin_provider = origin_handle.http_provider();

    let account = origin_handle.dev_wallets().next().unwrap().address();
    let to = Address::random();
    let amount = U256::from(1_000u64);

    // Mine two blocks on the upstream; the first strictly predates the fork head.
    let mut first_block = None;
    for _ in 0..2 {
        let tx = TransactionRequest::default().from(account).to(to).value(amount);
        let tx = WithOtherFields::new(tx);
        let receipt =
            origin_provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
        first_block.get_or_insert(receipt.block_number.unwrap());
    }
    let pre_fork_block = first_block.unwrap();

    // Fork from the upstream head so `pre_fork_block` is delegated upstream.
    let (_fork_api, fork_handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_handle.http_endpoint()))).await;
    let fork_provider = fork_handle.http_provider();

    let results: Vec<TraceResultsWithTransactionHash> = fork_provider
        .client()
        .request(
            "trace_replayBlockTransactions",
            (pre_fork_block, vec![TraceType::Trace, TraceType::StateDiff]),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    let full_trace = &results[0].full_trace;
    match &full_trace.trace[0].action {
        Action::Call(call) => {
            assert_eq!(call.from, account);
            assert_eq!(call.to, to);
        }
        other => panic!("expected Call action, got {other:?}"),
    }
    // `StateDiff` was also requested, so it must be honored, not just `Trace`.
    assert!(full_trace.state_diff.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_debug_trace_cache_includes_options() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let origin_provider = origin_handle.http_provider();
    let from = origin_handle.dev_wallets().next().unwrap().address();
    let receipt = origin_provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default().from(from).to(Address::random()).value(U256::from(1)),
        ))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let fork_url = spawn_rpc_proxy_rejecting_method_after(
        origin_handle.http_endpoint(),
        "debug_traceTransaction",
        2,
    )
    .await;
    let fork_url =
        spawn_rpc_proxy_rejecting_method_after(fork_url, "debug_traceBlockByHash", 2).await;
    let (_fork_api, fork_handle) = spawn(NodeConfig::test().with_eth_rpc_url(Some(fork_url))).await;
    let fork_provider = fork_handle.http_provider();
    let call_tracer = GethDebugTracingOptions::call_tracer(CallConfig::default());

    let default_trace = fork_provider
        .debug_trace_transaction(receipt.transaction_hash, GethDebugTracingOptions::default())
        .await
        .unwrap();
    let call_trace = fork_provider
        .debug_trace_transaction(receipt.transaction_hash, call_tracer.clone())
        .await
        .unwrap();
    assert!(matches!(default_trace, GethTrace::Default(_)));
    assert!(matches!(call_trace, GethTrace::CallTracer(_)));
    assert_eq!(
        fork_provider
            .debug_trace_transaction(receipt.transaction_hash, GethDebugTracingOptions::default())
            .await
            .unwrap(),
        default_trace
    );
    assert_eq!(
        fork_provider
            .debug_trace_transaction(receipt.transaction_hash, call_tracer.clone())
            .await
            .unwrap(),
        call_trace
    );

    let block_hash = receipt.block_hash.unwrap();
    let default_traces = fork_provider
        .debug_trace_block_by_hash(block_hash, GethDebugTracingOptions::default())
        .await
        .unwrap();
    let call_traces =
        fork_provider.debug_trace_block_by_hash(block_hash, call_tracer.clone()).await.unwrap();
    assert!(matches!(
        &default_traces[0],
        TraceResult::Success { result: GethTrace::Default(_), .. }
    ));
    assert!(matches!(
        &call_traces[0],
        TraceResult::Success { result: GethTrace::CallTracer(_), .. }
    ));
    assert_eq!(
        fork_provider
            .debug_trace_block_by_hash(block_hash, GethDebugTracingOptions::default())
            .await
            .unwrap(),
        default_traces
    );
    assert_eq!(
        fork_provider.debug_trace_block_by_hash(block_hash, call_tracer).await.unwrap(),
        call_traces
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_spawn_fork() {
    let (api, _handle) = spawn(fork_config()).await;
    assert!(api.is_fork());

    let head = api.block_number().unwrap();
    assert_eq!(head, U256::from(BLOCK_NUMBER))
}

#[tokio::test(flavor = "multi_thread")]
async fn validates_fork_source_chain_instead_of_local_chain_id() {
    for chain_id in [NamedChain::ZkSyncTestnet, NamedChain::ZkSync] {
        let (_origin_api, origin_handle) =
            spawn(NodeConfig::test().with_chain_id(Some(chain_id as u64))).await;

        let result = try_spawn(
            NodeConfig::test()
                .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
                .with_chain_id(Some(31_337u64)),
        )
        .await;
        let Err(error) = result else { panic!("expected zkSync fork startup to fail") };
        let message = format!("{error:?}");
        assert!(message.contains("cannot execute native EraVM bytecode"), "{message}");
        assert!(message.contains("anvil-zksync"), "{message}");
    }

    let (_origin_api, origin_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::Mainnet as u64))).await;

    let (_api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
            .with_chain_id(Some(NamedChain::ZkSync as u64)),
    )
    .await;

    assert_eq!(handle.http_provider().get_chain_id().await.unwrap(), NamedChain::ZkSync as u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn validates_every_fork_url_uses_the_same_supported_network() {
    let (_mainnet_api, mainnet_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::Mainnet as u64))).await;
    let (_zksync_api, zksync_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::ZkSync as u64))).await;

    let result = try_spawn(
        NodeConfig::test()
            .with_fork_urls(vec![mainnet_handle.http_endpoint(), zksync_handle.http_endpoint()]),
    )
    .await;
    let Err(error) = result else { panic!("expected zkSync fallback URL to be rejected") };
    assert!(format!("{error:?}").contains("cannot execute native EraVM bytecode"));

    let (_sepolia_api, sepolia_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::Sepolia as u64))).await;
    let result = try_spawn(
        NodeConfig::test()
            .with_fork_urls(vec![mainnet_handle.http_endpoint(), sepolia_handle.http_endpoint()]),
    )
    .await;
    let Err(error) = result else { panic!("expected mixed fork networks to be rejected") };
    assert!(format!("{error:?}").contains("fork endpoints must use the same chain ID"));

    let result = try_spawn(
        NodeConfig::test()
            .with_fork_urls(vec![mainnet_handle.http_endpoint(), sepolia_handle.http_endpoint()])
            .with_fork_chain_id(Some(U256::from(NamedChain::Mainnet as u64))),
    )
    .await;
    let Err(error) = result else { panic!("expected unverifiable offline fallbacks to fail") };
    assert!(format!("{error:?}").contains("multiple fork URLs cannot be validated"));
}

// <https://github.com/foundry-rs/foundry/issues/9743>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_set_storage_visible_to_call() {
    let (origin_api, origin_handle) = spawn(NodeConfig::test()).await;

    let target = Address::random();
    let slot = uint!(0x9f19e10bccde41c24f53ff4dbf7bb5ee2063896e54351d7230ecd1f7e361cb74_U256);
    let value = b256!("0000000000000000000000000000000000000000000000000000000000000001");

    // Return the value at `slot`, matching the storage read performed by ENS.resolver(bytes32).
    origin_api
        .anvil_set_code(
            target,
            bytes!(
                "7f9f19e10bccde41c24f53ff4dbf7bb5ee2063896e54351d7230ecd1f7e361cb74545f5260205ff3"
            ),
        )
        .await
        .unwrap();

    let (_fork_api, fork_handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_handle.http_endpoint()))).await;
    let provider = fork_handle.http_provider();

    let updated: bool =
        provider.raw_request("anvil_setStorageAt".into(), (target, slot, value)).await.unwrap();
    assert!(updated);
    assert_eq!(provider.get_storage_at(target, slot - U256::ONE).await.unwrap(), U256::ZERO);

    let tx = TransactionRequest::default().to(target);
    for _ in 0..10 {
        assert_eq!(provider.get_storage_at(target, slot).await.unwrap(), U256::ONE);
        let output = provider.call(tx.clone().into()).await.unwrap();
        assert_eq!(output.as_ref(), value.as_slice());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_eth_get_balance() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();
    for _ in 0..10 {
        let addr = Address::random();
        let balance = api.balance(addr, None).await.unwrap();
        let provider_balance = provider.get_balance(addr).await.unwrap();
        assert_eq!(balance, provider_balance)
    }
}

// <https://github.com/foundry-rs/foundry/issues/4082>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_eth_get_balance_after_mine() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();
    let info = api.anvil_node_info().await.unwrap();
    let number = info.fork_config.fork_block_number.unwrap();
    assert_eq!(number, BLOCK_NUMBER);

    let address = Address::random();

    let _balance = provider.get_balance(address).await.unwrap();

    api.evm_mine(None).await.unwrap();

    let _balance = provider.get_balance(address).await.unwrap();
}

// <https://github.com/foundry-rs/foundry/issues/4082>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_eth_get_code_after_mine() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();
    let info = api.anvil_node_info().await.unwrap();
    let number = info.fork_config.fork_block_number.unwrap();
    assert_eq!(number, BLOCK_NUMBER);

    let address = Address::random();

    let _code = provider.get_code_at(address).block_id(BlockId::number(1)).await.unwrap();

    api.evm_mine(None).await.unwrap();

    let _code = provider.get_code_at(address).block_id(BlockId::number(1)).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_eth_get_code() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();
    for _ in 0..10 {
        let addr = Address::random();
        let code = api.get_code(addr, None).await.unwrap();
        let provider_code = provider.get_code_at(addr).await.unwrap();
        assert_eq!(code, provider_code)
    }

    let addresses: Vec<Address> = vec![
        "0x6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
        "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".parse().unwrap(),
        "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2".parse().unwrap(),
        "0x1F98431c8aD98523631AE4a59f267346ea31F984".parse().unwrap(),
        "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45".parse().unwrap(),
    ];
    for address in addresses {
        let prev_code = api
            .get_code(address, Some(BlockNumberOrTag::Number(BLOCK_NUMBER - 10).into()))
            .await
            .unwrap();
        let code = api.get_code(address, None).await.unwrap();
        let provider_code = provider.get_code_at(address).await.unwrap();
        assert_eq!(code, prev_code);
        assert_eq!(code, provider_code);
        assert!(!code.as_ref().is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_eth_get_nonce() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    for _ in 0..10 {
        let addr = Address::random();
        let api_nonce = api.transaction_count(addr, None).await.unwrap().to::<u64>();
        let provider_nonce = provider.get_transaction_count(addr).await.unwrap();
        assert_eq!(api_nonce, provider_nonce);
    }

    let addr = Config::DEFAULT_SENDER;
    let api_nonce = api.transaction_count(addr, None).await.unwrap().to::<u64>();
    let provider_nonce = provider.get_transaction_count(addr).await.unwrap();
    assert_eq!(api_nonce, provider_nonce);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_optimism_with_transaction_hash() {
    use std::str::FromStr;

    // Fork to a block with a specific transaction
    let fork_tx_hash =
        TxHash::from_str("fcb864b5a50f0f0b111dbbf9e9167b2cb6179dfd6270e1ad53aac6049c0ec038")
            .unwrap();
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(rpc::next_rpc_endpoint(NamedChain::Optimism)))
            .with_fork_transaction_hash(Some(fork_tx_hash)),
    )
    .await;

    // The prefix is replayed synchronously as the next local block.
    let block_number = api.block_number().unwrap().to::<u64>();
    assert_eq!(block_number, 125777954);
    assert!(api.backend.mined_transaction_by_hash(fork_tx_hash).is_some());
    assert!(
        handle
            .http_provider()
            .get_transaction_receipt(fork_tx_hash)
            .await
            .unwrap()
            .unwrap()
            .status()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_eth_fee_history() {
    let (api, _handle) = spawn(fork_config()).await;
    let fork = api.get_fork().unwrap();

    let count = 10u64;
    let history =
        api.fee_history(U256::from(count), BlockNumberOrTag::Latest, vec![]).await.unwrap();
    let upstream_history =
        fork.fee_history(count, BlockNumberOrTag::Number(BLOCK_NUMBER), &[]).await.unwrap();

    assert_eq!(history, upstream_history);
}

// Regression test for a fork-range bug in `eth_feeHistory`: when the requested range straddles
// the fork boundary (newest block post-fork, oldest block pre-fork), the cache fallback must not
// call `backend.get_block` on the pre-fork blocks — local storage has none, which made the call
// hard-error. The pre-fork portion is served by the fork provider and merged with the locally
// computed post-fork portion. The earlier guard only covers fully pre-fork ranges, so this needs
// local blocks mined above the fork block to be exercised.
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_fee_history_across_fork_boundary() {
    let (api, _handle) = spawn(fork_config()).await;

    // Mine local blocks so `latest` sits above the fork block.
    api.anvil_mine(Some(U256::from(3)), None).await.unwrap();

    // latest = fork + 3, count = 10 => oldest = fork - 6 (pre-fork), newest = fork + 3 (post-fork).
    let count = 10u64;
    let history =
        api.fee_history(U256::from(count), BlockNumberOrTag::Latest, vec![]).await.unwrap();

    // The oldest block must be the true range start (latest - count + 1); a split that mistook the
    // whole range for pre-fork would shift it and return more entries than requested.
    let latest = api.block_number().unwrap().to::<u64>();
    assert_eq!(history.oldest_block, latest - count + 1, "wrong oldest_block");

    // Full range covered: per-block arrays have `count` entries; base_fee_per_gas has one more.
    assert_eq!(history.gas_used_ratio.len(), count as usize, "incomplete gas_used_ratio");
    assert_eq!(
        history.base_fee_per_gas.len(),
        count as usize + 1,
        "incomplete base_fee_per_gas across the fork boundary"
    );

    // The pre-fork segment here is pre-Cancun, so the fork provider may return empty blob-fee
    // arrays. They must still be padded to stay aligned with the gas arrays, otherwise the merged
    // response is short and misaligned.
    assert_eq!(
        history.blob_gas_used_ratio.len(),
        count as usize,
        "incomplete blob_gas_used_ratio across the fork boundary"
    );
    assert_eq!(
        history.base_fee_per_blob_gas.len(),
        count as usize + 1,
        "incomplete base_fee_per_blob_gas across the fork boundary"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let block_number = provider.get_block_number().await.unwrap();
    let balance_before = provider.get_balance(to).await.unwrap();
    let amount = handle.genesis_balance().checked_div(U256::from(2u64)).unwrap();

    let initial_nonce = provider.get_transaction_count(from).await.unwrap();

    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);
    let tx = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    assert_eq!(tx.transaction_index, Some(0));

    let nonce = provider.get_transaction_count(from).await.unwrap();

    assert_eq!(nonce, initial_nonce + 1);
    let to_balance = provider.get_balance(to).await.unwrap();
    assert_eq!(balance_before.saturating_add(amount), to_balance);
    api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(block_number) }))
        .await
        .unwrap();

    // reset block number
    assert_eq!(block_number, provider.get_block_number().await.unwrap());

    let nonce = provider.get_transaction_count(from).await.unwrap();
    assert_eq!(nonce, initial_nonce);
    let balance = provider.get_balance(from).await.unwrap();
    assert_eq!(balance, handle.genesis_balance());
    let balance = provider.get_balance(to).await.unwrap();
    assert_eq!(balance, handle.genesis_balance());

    // reset to latest
    api.anvil_reset(Some(Forking::default())).await.unwrap();

    let new_block_num = provider.get_block_number().await.unwrap();
    assert!(new_block_num > block_number);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_setup() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let dead_addr: Address = "000000000000000000000000000000000000dEaD".parse().unwrap();

    let block_number = provider.get_block_number().await.unwrap();
    assert_eq!(block_number, 0);

    let local_balance = provider.get_balance(dead_addr).await.unwrap();
    assert_eq!(local_balance, U256::ZERO);

    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(rpc::next_http_archive_rpc_url()),
        block_number: Some(BLOCK_NUMBER),
    }))
    .await
    .unwrap();

    let block_number = provider.get_block_number().await.unwrap();
    assert_eq!(block_number, BLOCK_NUMBER);

    let remote_balance = provider.get_balance(dead_addr).await.unwrap();
    assert_eq!(remote_balance, U256::from(DEAD_BALANCE_AT_BLOCK_NUMBER));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_state_snapshotting() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();
    let state_snapshot = api.evm_snapshot().await.unwrap();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let block_number = provider.get_block_number().await.unwrap();

    let initial_nonce = provider.get_transaction_count(from).await.unwrap();
    let balance_before = provider.get_balance(to).await.unwrap();
    let amount = handle.genesis_balance().checked_div(U256::from(2u64)).unwrap();

    let provider = handle.http_provider();
    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);

    let _ = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let provider = handle.http_provider();

    let nonce = provider.get_transaction_count(from).await.unwrap();
    assert_eq!(nonce, initial_nonce + 1);
    let to_balance = provider.get_balance(to).await.unwrap();
    assert_eq!(balance_before.saturating_add(amount), to_balance);

    assert!(api.evm_revert(state_snapshot).await.unwrap());

    let nonce = provider.get_transaction_count(from).await.unwrap();
    assert_eq!(nonce, initial_nonce);
    let balance = provider.get_balance(from).await.unwrap();
    assert_eq!(balance, handle.genesis_balance());
    let balance = provider.get_balance(to).await.unwrap();
    assert_eq!(balance, handle.genesis_balance());
    assert_eq!(block_number, provider.get_block_number().await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_state_snapshotting_repeated() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    let state_snapshot = api.evm_snapshot().await.unwrap();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let block_number = provider.get_block_number().await.unwrap();

    let initial_nonce = provider.get_transaction_count(from).await.unwrap();
    let balance_before = provider.get_balance(to).await.unwrap();
    let amount = handle.genesis_balance().checked_div(U256::from(92u64)).unwrap();

    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);
    let tx_provider = handle.http_provider();
    let _ = tx_provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let nonce = provider.get_transaction_count(from).await.unwrap();
    assert_eq!(nonce, initial_nonce + 1);
    let to_balance = provider.get_balance(to).await.unwrap();
    assert_eq!(balance_before.saturating_add(amount), to_balance);

    let second_state_snapshot = api.evm_snapshot().await.unwrap();

    assert!(api.evm_revert(state_snapshot).await.unwrap());

    let nonce = provider.get_transaction_count(from).await.unwrap();
    assert_eq!(nonce, initial_nonce);
    let balance = provider.get_balance(from).await.unwrap();
    assert_eq!(balance, handle.genesis_balance());
    let balance = provider.get_balance(to).await.unwrap();
    assert_eq!(balance, handle.genesis_balance());
    assert_eq!(block_number, provider.get_block_number().await.unwrap());

    // The newer snapshot was invalidated by reverting to an older snapshot.
    assert!(!api.evm_revert(second_state_snapshot).await.unwrap());

    // nothing is reverted, snapshot gone
    assert!(!api.evm_revert(state_snapshot).await.unwrap());
}

// <https://github.com/foundry-rs/foundry/issues/6463>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_state_snapshotting_blocks() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    let state_snapshot = api.evm_snapshot().await.unwrap();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let block_number = provider.get_block_number().await.unwrap();

    let initial_nonce = provider.get_transaction_count(from).await.unwrap();
    let balance_before = provider.get_balance(to).await.unwrap();
    let amount = handle.genesis_balance().checked_div(U256::from(2u64)).unwrap();

    // send the transaction
    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);
    let _ = provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let block_number_after = provider.get_block_number().await.unwrap();
    assert_eq!(block_number_after, block_number + 1);

    let nonce = provider.get_transaction_count(from).await.unwrap();
    assert_eq!(nonce, initial_nonce + 1);
    let to_balance = provider.get_balance(to).await.unwrap();
    assert_eq!(balance_before.saturating_add(amount), to_balance);

    assert!(api.evm_revert(state_snapshot).await.unwrap());

    assert_eq!(initial_nonce, provider.get_transaction_count(from).await.unwrap());
    let block_number_after = provider.get_block_number().await.unwrap();
    assert_eq!(block_number_after, block_number);

    // repeat transaction
    let _ = provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();
    let nonce = provider.get_transaction_count(from).await.unwrap();
    assert_eq!(nonce, initial_nonce + 1);

    // revert again: nothing to revert since state snapshot gone
    assert!(!api.evm_revert(state_snapshot).await.unwrap());
    let nonce = provider.get_transaction_count(from).await.unwrap();
    assert_eq!(nonce, initial_nonce + 1);
    let block_number_after = provider.get_block_number().await.unwrap();
    assert_eq!(block_number_after, block_number + 1);
}

/// tests that the remote state and local state are kept separate.
/// changes don't make into the read only Database that holds the remote state, which is flushed to
/// a cache file.
#[tokio::test(flavor = "multi_thread")]
async fn test_separate_states() {
    let (api, handle) = spawn(fork_config().with_fork_block_number(Some(14723772u64))).await;
    let provider = handle.http_provider();

    let addr: Address = "000000000000000000000000000000000000dEaD".parse().unwrap();

    let remote_balance = provider.get_balance(addr).await.unwrap();
    assert_eq!(remote_balance, U256::from(12556104082473169733500u128));

    api.anvil_set_balance(addr, U256::from(1337u64)).await.unwrap();
    let balance = provider.get_balance(addr).await.unwrap();
    assert_eq!(balance, U256::from(1337u64));

    let fork = api.get_fork().unwrap();
    let fork_db = fork.database.read().await;
    let acc = fork_db
        .maybe_inner()
        .expect("could not get fork db inner")
        .db()
        .accounts
        .read()
        .get(&addr)
        .cloned()
        .unwrap();

    assert_eq!(acc.balance, remote_balance);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_deploy_greeter_on_fork() {
    let (_api, handle) = spawn(fork_config().with_fork_block_number(Some(14723772u64))).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumWallet = wallet.into();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let greeter_contract = Greeter::deploy(&provider, "Hello World!".to_string()).await.unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);

    let greeter_contract = Greeter::deploy(&provider, "Hello World!".to_string()).await.unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_reset_properly() {
    let (origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let account = origin_handle.dev_accounts().next().unwrap();
    let origin_provider = origin_handle.http_provider();
    let origin_nonce = 1u64;
    origin_api.anvil_set_nonce(account, U256::from(origin_nonce)).await.unwrap();

    assert_eq!(origin_nonce, origin_provider.get_transaction_count(account).await.unwrap());

    let (fork_api, fork_handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_handle.http_endpoint()))).await;

    let fork_provider = fork_handle.http_provider();
    let fork_tx_provider = http_provider(&fork_handle.http_endpoint());
    assert_eq!(origin_nonce, fork_provider.get_transaction_count(account).await.unwrap());

    let to = Address::random();
    let to_balance = fork_provider.get_balance(to).await.unwrap();
    let tx = TransactionRequest::default().from(account).to(to).value(U256::from(1337u64));
    let tx = WithOtherFields::new(tx);
    let tx = fork_tx_provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    // nonce incremented by 1
    assert_eq!(origin_nonce + 1, fork_provider.get_transaction_count(account).await.unwrap());

    // resetting to origin state
    fork_api.anvil_reset(Some(Forking::default())).await.unwrap();

    // nonce reset to origin
    assert_eq!(origin_nonce, fork_provider.get_transaction_count(account).await.unwrap());

    // balance is reset
    assert_eq!(to_balance, fork_provider.get_balance(to).await.unwrap());

    // tx does not exist anymore
    assert!(fork_tx_provider.get_transaction_by_hash(tx.transaction_hash).await.unwrap().is_none())
}

// Ref: <https://github.com/foundry-rs/foundry/issues/8684>
#[tokio::test(flavor = "multi_thread")]
async fn can_reset_fork_to_new_fork() {
    let eth_rpc_url = next_rpc_endpoint(NamedChain::Mainnet);
    let (api, handle) = spawn(NodeConfig::test().with_eth_rpc_url(Some(eth_rpc_url))).await;
    let provider = handle.http_provider();

    let op = address!("0xC0d3c0d3c0D3c0D3C0d3C0D3C0D3c0d3c0d30007"); // L2CrossDomainMessenger - Dead on mainnet.

    let tx = TransactionRequest::default().with_to(op).with_input("0x54fd4d50");

    let tx = WithOtherFields::new(tx);

    let mainnet_call_output = provider.call(tx).await.unwrap();

    assert_eq!(mainnet_call_output, Bytes::new()); // 0x

    let optimism = next_rpc_endpoint(NamedChain::Optimism);

    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(optimism.clone()),
        block_number: Some(124659890),
    }))
    .await
    .unwrap();

    let code = provider.get_code_at(op).await.unwrap();

    assert_ne!(code, Bytes::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_timestamp() {
    let start = std::time::Instant::now();

    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    let block = provider.get_block(BlockId::Number(BLOCK_NUMBER.into())).await.unwrap().unwrap();
    assert_eq!(block.header.timestamp, BLOCK_TIMESTAMP);

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();

    let tx =
        TransactionRequest::default().to(Address::random()).value(U256::from(1337u64)).from(from);
    let tx = WithOtherFields::new(tx);
    let tx = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    let status = tx.inner.inner.inner.receipt.status.coerce_status();
    assert!(status);

    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();

    let elapsed = start.elapsed().as_secs() + 1;

    // ensure the diff between the new mined block and the original block is within the elapsed time
    let diff = block.header.timestamp - BLOCK_TIMESTAMP;
    assert!(diff <= elapsed, "diff={diff}, elapsed={elapsed}");

    let start = std::time::Instant::now();
    // reset to check timestamp works after resetting
    api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(BLOCK_NUMBER) }))
        .await
        .unwrap();
    let block = provider.get_block(BlockId::Number(BLOCK_NUMBER.into())).await.unwrap().unwrap();
    assert_eq!(block.header.timestamp, BLOCK_TIMESTAMP);

    let tx =
        TransactionRequest::default().to(Address::random()).value(U256::from(1337u64)).from(from);
    let tx = WithOtherFields::new(tx);
    let _ = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap(); // FIXME: Awaits endlessly here.

    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    let elapsed = start.elapsed().as_secs() + 1;
    let diff = block.header.timestamp - BLOCK_TIMESTAMP;
    assert!(diff <= elapsed);

    // ensure that after setting a timestamp manually, then next block time is correct
    let start = std::time::Instant::now();
    api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(BLOCK_NUMBER) }))
        .await
        .unwrap();
    api.evm_set_next_block_timestamp(BLOCK_TIMESTAMP + 1).unwrap();
    let tx =
        TransactionRequest::default().to(Address::random()).value(U256::from(1337u64)).from(from);
    let tx = WithOtherFields::new(tx);
    let _tx = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(block.header.timestamp, BLOCK_TIMESTAMP + 1);

    let tx =
        TransactionRequest::default().to(Address::random()).value(U256::from(1337u64)).from(from);
    let tx = WithOtherFields::new(tx);
    let _ = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    let elapsed = start.elapsed().as_secs() + 1;
    let diff = block.header.timestamp - (BLOCK_TIMESTAMP + 1);
    assert!(diff <= elapsed);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_set_empty_code() {
    let (api, _handle) = spawn(fork_config()).await;
    let addr = "0x1f9840a85d5af5bf1d1762f925bdaddc4201f984".parse().unwrap();
    let code = api.get_code(addr, None).await.unwrap();
    assert!(!code.as_ref().is_empty());
    api.anvil_set_code(addr, Vec::new().into()).await.unwrap();
    let code = api.get_code(addr, None).await.unwrap();
    assert!(code.as_ref().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_can_send_tx() {
    let (api, handle) =
        spawn(fork_config().with_blocktime(Some(std::time::Duration::from_millis(800)))).await;

    let wallet = PrivateKeySigner::random();
    let signer = wallet.address();
    let provider = handle.http_provider();
    // let provider = SignerMiddleware::new(provider, wallet);

    api.anvil_set_balance(signer, U256::MAX).await.unwrap();
    api.anvil_impersonate_account(signer).await.unwrap(); // Added until WalletFiller for alloy-provider is fixed.
    let balance = provider.get_balance(signer).await.unwrap();
    assert_eq!(balance, U256::MAX);

    let addr = Address::random();
    let val = U256::from(1337u64);
    let tx = TransactionRequest::default().to(addr).value(val).from(signer);
    let tx = WithOtherFields::new(tx);
    // broadcast it via the eth_sendTransaction API
    let _ = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let balance = provider.get_balance(addr).await.unwrap();
    assert_eq!(balance, val);
}

// <https://github.com/foundry-rs/foundry/issues/1920>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_nft_set_approve_all() {
    let (api, handle) = spawn(
        fork_config()
            .with_fork_block_number(Some(14812197u64))
            .with_blocktime(Some(Duration::from_secs(5)))
            .with_chain_id(1u64.into()),
    )
    .await;

    // create and fund a random wallet
    let wallet = PrivateKeySigner::random();
    let signer = wallet.address();
    api.anvil_set_balance(signer, U256::from(1000e18)).await.unwrap();

    let provider = handle.http_provider();

    // pick a random nft <https://opensea.io/assets/ethereum/0x9c8ff314c9bc7f6e59a9d9225fb22946427edc03/154>
    let nouns_addr: Address = "0x9c8ff314c9bc7f6e59a9d9225fb22946427edc03".parse().unwrap();

    let owner: Address = "0x052564eb0fd8b340803df55def89c25c432f43f4".parse().unwrap();
    let token_id: U256 = U256::from(154u64);

    let nouns = ERC721::new(nouns_addr, provider.clone());

    let real_owner = nouns.ownerOf(token_id).call().await.unwrap();
    assert_eq!(real_owner, owner);
    let approval = nouns.setApprovalForAll(nouns_addr, true);
    let tx = TransactionRequest::default()
        .from(owner)
        .to(nouns_addr)
        .with_input(approval.calldata().to_owned());
    let tx = WithOtherFields::new(tx);
    api.anvil_impersonate_account(owner).await.unwrap();
    let tx = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    let status = tx.inner.inner.inner.receipt.status.coerce_status();
    assert!(status);

    // transfer: impersonate real owner and transfer nft
    api.anvil_impersonate_account(real_owner).await.unwrap();

    api.anvil_set_balance(real_owner, U256::from(10000e18 as u64)).await.unwrap();

    let call = nouns.transferFrom(real_owner, signer, token_id);
    let tx = TransactionRequest::default()
        .from(real_owner)
        .to(nouns_addr)
        .with_input(call.calldata().to_owned());
    let tx = WithOtherFields::new(tx);
    let tx = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    let status = tx.inner.inner.inner.receipt.status.coerce_status();
    assert!(status);

    let real_owner = nouns.ownerOf(token_id).call().await.unwrap();
    assert_eq!(real_owner, wallet.address());
}

// <https://github.com/foundry-rs/foundry/issues/2261>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_with_custom_chain_id() {
    // spawn a forked node with some random chainId
    let (api, handle) = spawn(
        fork_config()
            .with_fork_block_number(Some(14812197u64))
            .with_blocktime(Some(Duration::from_secs(5)))
            .with_chain_id(3145u64.into()),
    )
    .await;

    // get the eth chainId and the txn chainId
    let eth_chain_id = api.eth_chain_id();
    let txn_chain_id = api.chain_id();

    // get the chainId in the config
    let config_chain_id = handle.config().chain_id;

    // check that the chainIds are the same
    assert_eq!(eth_chain_id.unwrap().unwrap().to::<u64>(), 3145u64);
    assert_eq!(txn_chain_id, 3145u64);
    assert_eq!(config_chain_id, Some(3145u64));
}

// <https://github.com/foundry-rs/foundry/issues/1920>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_can_send_opensea_tx() {
    let (api, handle) = spawn(
        fork_config()
            .with_fork_block_number(Some(14983338u64))
            .with_blocktime(Some(Duration::from_millis(5000))),
    )
    .await;

    let sender: Address = "0x8fdbae54b6d9f3fc2c649e3dd4602961967fd42f".parse().unwrap();

    // transfer: impersonate real sender
    api.anvil_impersonate_account(sender).await.unwrap();

    let provider = handle.http_provider();

    let input: Bytes = "0xfb0f3ee1000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000003ff2e795f5000000000000000000000000000023f28ae3e9756ba982a6290f9081b6a84900b758000000000000000000000000004c00500000ad104d7dbd00e3ae0a5c00560c0000000000000000000000000003235b597a78eabcb08ffcb4d97411073211dbcb0000000000000000000000000000000000000000000000000000000000000e72000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000062ad47c20000000000000000000000000000000000000000000000000000000062d43104000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000df44e65d2a2cf40000007b02230091a7ed01230072f7006a004d60a8d4e71d599b8104250f00000000007b02230091a7ed01230072f7006a004d60a8d4e71d599b8104250f00000000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000024000000000000000000000000000000000000000000000000000000000000002e000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000001c6bf526340000000000000000000000000008de9c5a032463c561423387a9648c5c7bcc5bc900000000000000000000000000000000000000000000000000005543df729c0000000000000000000000000006eb234847a9e3a546539aac57a071c01dc3f398600000000000000000000000000000000000000000000000000000000000000416d39b5352353a22cf2d44faa696c2089b03137a13b5acfee0366306f2678fede043bc8c7e422f6f13a3453295a4a063dac7ee6216ab7bade299690afc77397a51c00000000000000000000000000000000000000000000000000000000000000".parse().unwrap();
    let to: Address = "0x00000000006c3852cbef3e08e8df289169ede581".parse().unwrap();
    let tx = TransactionRequest::default()
        .from(sender)
        .to(to)
        .value(U256::from(20000000000000000u64))
        .with_input(input)
        .with_gas_price(22180711707u128)
        .with_gas_limit(150_000);
    let tx = WithOtherFields::new(tx);

    let tx = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    let status = tx.inner.inner.inner.receipt.status.coerce_status();
    assert!(status);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_base_fee() {
    let (api, handle) = spawn(fork_config()).await;

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();

    let provider = handle.http_provider();

    api.anvil_set_next_block_base_fee_per_gas(U256::ZERO).await.unwrap();

    let addr = Address::random();
    let val = U256::from(1337u64);
    let tx = TransactionRequest::default().from(from).to(addr).value(val);
    let tx = WithOtherFields::new(tx);
    let _res = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_pre_london_base_fee_is_null() {
    let (_api, handle) = spawn(fork_config().with_fork_block_number(Some(12_000_000u64))).await;

    let provider = handle.http_provider();

    let base_fee: Option<U256> = provider.client().request("eth_baseFee", ()).await.unwrap();
    assert_eq!(base_fee, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_init_base_fee() {
    let (api, handle) = spawn(fork_config().with_fork_block_number(Some(13184859u64))).await;

    let provider = handle.http_provider();

    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    // <https://etherscan.io/block/13184859>
    assert_eq!(block.header.number, 13184859u64);
    let init_base_fee = block.header.base_fee_per_gas.unwrap();
    assert_eq!(init_base_fee, 63739886069);

    api.mine_one().await.unwrap();

    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();

    let next_base_fee = block.header.base_fee_per_gas.unwrap();
    assert!(next_base_fee < init_base_fee);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_init_blob_base_fee_with_explicit_base_fee() {
    let fork_rpc_url = rpc::next_http_archive_rpc_url();
    let fork_block_number = 24_127_158u64;
    let (default_api, _) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(fork_rpc_url.clone()))
            .with_fork_block_number(Some(fork_block_number)),
    )
    .await;
    let explicit_base_fee = default_api
        .block_by_number(BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();
    let (explicit_api, _) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(fork_rpc_url))
            .with_fork_block_number(Some(fork_block_number))
            .with_base_fee(Some(explicit_base_fee)),
    )
    .await;

    let default_blob_base_fee = default_api.blob_base_fee().unwrap();
    let explicit_blob_base_fee = explicit_api.blob_base_fee().unwrap();

    assert!(default_blob_base_fee > U256::from(1));
    assert_eq!(explicit_blob_base_fee, default_blob_base_fee);
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_reset_fork_on_new_blocks() {
    let (api, handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(rpc::next_http_archive_rpc_url()))).await;

    let anvil_provider = handle.http_provider();
    let endpoint = next_http_rpc_endpoint();
    let provider = Arc::new(get_http_provider(&endpoint));

    let current_block = anvil_provider.get_block_number().await.unwrap();

    handle
        .task_manager()
        .spawn_reset_on_new_polled_blocks::<alloy_network::AnyNetwork, _>(provider.clone(), api);

    let mut stream = provider
        .watch_blocks()
        .await
        .unwrap()
        .with_poll_interval(Duration::from_secs(2))
        .into_stream()
        .flat_map(futures::stream::iter);
    // the http watcher may fetch multiple blocks at once, so we set a timeout here to offset edge
    // cases where the stream immediately returns a block
    tokio::time::sleep(Duration::from_secs(12)).await;
    stream.next().await.unwrap();
    stream.next().await.unwrap();

    let next_block = anvil_provider.get_block_number().await.unwrap();

    assert!(next_block > current_block, "nextblock={next_block} currentblock={current_block}")
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_call() {
    let input: Bytes = "0x77c7b8fc".parse().unwrap();
    let to: Address = "0x99d1Fa417f94dcD62BfE781a1213c092a47041Bc".parse().unwrap();
    let block_number = 14746300u64;

    let provider = http_provider(rpc::next_http_archive_rpc_url().as_str());
    let tx = TransactionRequest::default().to(to).with_input(input.clone());
    let tx = WithOtherFields::new(tx);
    let res0 = provider.call(tx).block(BlockId::Number(block_number.into())).await.unwrap();

    let (api, _) = spawn(fork_config().with_fork_block_number(Some(block_number))).await;

    let res1 = api
        .call(
            WithOtherFields::new(TransactionRequest {
                to: Some(TxKind::from(to)),
                input: input.into(),
                ..Default::default()
            }),
            None,
            EvmOverrides::default(),
        )
        .await
        .unwrap();

    assert_eq!(res0, res1);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_block_timestamp() {
    let (api, _) = spawn(fork_config()).await;

    let initial_block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    api.anvil_mine(Some(U256::from(1)), None).await.unwrap();
    let latest_block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();

    assert!(initial_block.header.timestamp <= latest_block.header.timestamp);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_snapshot_block_timestamp() {
    let (api, _) = spawn(fork_config()).await;

    let snapshot_id = api.evm_snapshot().await.unwrap();
    api.anvil_mine(Some(U256::from(1)), None).await.unwrap();
    let initial_block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    api.evm_revert(snapshot_id).await.unwrap();
    api.evm_set_next_block_timestamp(initial_block.header.timestamp).unwrap();
    api.anvil_mine(Some(U256::from(1)), None).await.unwrap();
    let latest_block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();

    assert_eq!(initial_block.header.timestamp, latest_block.header.timestamp);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_uncles_fetch() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    // Block on ETH mainnet with 2 uncles
    let block_with_uncles = 190u64;

    let block =
        api.block_by_number(BlockNumberOrTag::Number(block_with_uncles)).await.unwrap().unwrap();

    assert_eq!(block.uncles.len(), 2);

    let count = provider.get_uncle_count(block_with_uncles.into()).await.unwrap();
    assert_eq!(count as usize, block.uncles.len());

    let hash = BlockId::hash(block.header.hash);
    let count = provider.get_uncle_count(hash).await.unwrap();
    assert_eq!(count as usize, block.uncles.len());

    for (uncle_idx, uncle_hash) in block.uncles.iter().enumerate() {
        // Try with block number
        let uncle = provider
            .get_uncle(BlockId::number(block_with_uncles), uncle_idx as u64)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*uncle_hash, uncle.header.hash);

        // Try with block hash
        let uncle = provider
            .get_uncle(BlockId::hash(block.header.hash), uncle_idx as u64)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*uncle_hash, uncle.header.hash);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_block_transaction_count() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let sender = accounts[0].address();

    // disable automine (so there are pending transactions)
    api.anvil_set_auto_mine(false).await.unwrap();
    // transfer: impersonate real sender
    api.anvil_impersonate_account(sender).await.unwrap();

    let tx =
        TransactionRequest::default().from(sender).value(U256::from(42u64)).with_gas_limit(100_000);
    let tx = WithOtherFields::new(tx);
    let _ = provider.send_transaction(tx).await.unwrap();

    let pending_txs =
        api.block_transaction_count_by_number(BlockNumberOrTag::Pending).await.unwrap().unwrap();
    assert_eq!(pending_txs.to::<u64>(), 1);

    // mine a new block
    api.anvil_mine(None, None).await.unwrap();

    let pending_txs =
        api.block_transaction_count_by_number(BlockNumberOrTag::Pending).await.unwrap().unwrap();
    assert_eq!(pending_txs.to::<u64>(), 0);
    let latest_txs =
        api.block_transaction_count_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert_eq!(latest_txs.to::<u64>(), 1);
    let latest_block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    let latest_txs =
        api.block_transaction_count_by_hash(latest_block.header.hash).await.unwrap().unwrap();
    assert_eq!(latest_txs.to::<u64>(), 1);

    // check txs count on an older block: 420000 has 3 txs on mainnet
    let count_txs = api
        .block_transaction_count_by_number(BlockNumberOrTag::Number(420000))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(count_txs.to::<u64>(), 3);
    let count_txs = api
        .block_transaction_count_by_hash(
            "0xb3b0e3e0c64e23fb7f1ccfd29245ae423d2f6f1b269b63b70ff882a983ce317c".parse().unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(count_txs.to::<u64>(), 3);
}

// <https://github.com/foundry-rs/foundry/issues/2931>
#[tokio::test(flavor = "multi_thread")]
async fn can_impersonate_in_fork() {
    let (api, handle) = spawn(fork_config().with_fork_block_number(Some(15347924u64))).await;
    let provider = handle.http_provider();

    let token_holder: Address = "0x2f0b23f53734252bda2277357e97e1517d6b042a".parse().unwrap();
    let to = Address::random();
    let val = U256::from(1337u64);

    // fund the impersonated account
    api.anvil_set_balance(token_holder, U256::from(1e18)).await.unwrap();

    let tx = TransactionRequest::default().from(token_holder).to(to).value(val);
    let tx = WithOtherFields::new(tx);
    let res = provider.send_transaction(tx.clone()).await;
    res.unwrap_err();

    api.anvil_impersonate_account(token_holder).await.unwrap();

    let res = provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();
    assert_eq!(res.from, token_holder);
    let status = res.inner.inner.inner.receipt.status.coerce_status();
    assert!(status);

    let balance = provider.get_balance(to).await.unwrap();
    assert_eq!(balance, val);

    api.anvil_stop_impersonating_account(token_holder).await.unwrap();
    let res = provider.send_transaction(tx).await;
    res.unwrap_err();
}

// <https://etherscan.io/block/14608400>
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_total_difficulty_fork() {
    let (api, handle) = spawn(fork_config()).await;

    let total_difficulty = U256::from(46_673_965_560_973_856_260_636u128);
    let difficulty = U256::from(13_680_435_288_526_144u128);

    let provider = handle.http_provider();
    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(block.header.total_difficulty, Some(total_difficulty));
    assert_eq!(block.header.difficulty, difficulty);

    api.mine_one().await.unwrap();
    api.mine_one().await.unwrap();

    let next_total_difficulty = total_difficulty + difficulty;

    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(block.header.total_difficulty, Some(next_total_difficulty));
    assert_eq!(block.header.difficulty, U256::ZERO);
}

// <https://etherscan.io/block/14608400>
#[tokio::test(flavor = "multi_thread")]
async fn test_transaction_receipt() {
    let (api, _) = spawn(fork_config()).await;

    // A transaction from the forked block (14608400)
    let receipt = api
        .transaction_receipt(
            "0xce495d665e9091613fd962351a5cbca27a992b919d6a87d542af97e2723ec1e4".parse().unwrap(),
        )
        .await
        .unwrap();
    assert!(receipt.is_some());

    // A transaction from a block in the future (14608401)
    let receipt = api
        .transaction_receipt(
            "0x1a15472088a4a97f29f2f9159511dbf89954b58d9816e58a32b8dc17171dc0e8".parse().unwrap(),
        )
        .await
        .unwrap();
    assert!(receipt.is_none());
}

// <https://etherscan.io/block/14608400>
#[tokio::test(flavor = "multi_thread")]
async fn test_block_receipts() {
    let (api, _) = spawn(fork_config()).await;

    // Receipts from the forked block (14608400)
    let receipts = api.block_receipts(BlockNumberOrTag::Number(BLOCK_NUMBER).into()).await.unwrap();
    assert!(receipts.is_some());

    // Receipts from a block in the future (14608401)
    let receipts =
        api.block_receipts(BlockNumberOrTag::Number(BLOCK_NUMBER + 1).into()).await.unwrap();
    assert!(receipts.is_none());

    // Receipts from a block hash (14608400)
    let hash = b256!("0x4c1c76f89cfe4eb503b09a0993346dd82865cac9d76034efc37d878c66453f0a");
    let receipts = api.block_receipts(BlockId::Hash(hash.into())).await.unwrap();
    assert!(receipts.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_pending_block_receipts_do_not_return_fork_head_receipts() {
    let (origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let sender = origin_handle.dev_wallets().next().unwrap().address();
    origin_api
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default().from(sender).to(Address::random()).value(U256::from(1)),
        ))
        .await
        .unwrap();

    let (fork_api, _) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
            .with_fork_block_number(Some(1u64)),
    )
    .await;

    let latest_receipts = fork_api.block_receipts(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(latest_receipts.len(), 1);

    let pending_block =
        fork_api.block_by_number_full(BlockNumberOrTag::Pending).await.unwrap().unwrap();
    assert_eq!(pending_block.header.number, 2);
    assert!(pending_block.transactions.is_empty());

    let pending_receipts = fork_api.block_receipts(BlockId::pending()).await.unwrap().unwrap();
    assert!(pending_receipts.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_override_fork_chain_id() {
    let chain_id_override = 5u64;
    let (_api, handle) = spawn(
        fork_config()
            .with_fork_block_number(Some(16506610u64))
            .with_chain_id(Some(chain_id_override)),
    )
    .await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumWallet = wallet.into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let greeter_contract =
        Greeter::deploy(provider.clone(), "Hello World!".to_string()).await.unwrap();
    let greeting = greeter_contract.greet().call().await.unwrap();

    assert_eq!("Hello World!", greeting);
    let greeter_contract =
        Greeter::deploy(provider.clone(), "Hello World!".to_string()).await.unwrap();
    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);

    let provider = handle.http_provider();
    let chain_id = provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, chain_id_override);
}

// <https://github.com/foundry-rs/foundry/issues/6485>
#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_fork_reset_moonbeam() {
    crate::init_tracing();
    let (api, handle) = spawn(
        fork_config()
            .with_eth_rpc_url(Some("https://moonbeam.api.onfinality.io/public".to_string()))
            .with_fork_block_number(None::<u64>),
    )
    .await;
    let provider = handle.http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();

    let tx =
        TransactionRequest::default().to(Address::random()).value(U256::from(1337u64)).from(from);
    let tx = WithOtherFields::new(tx);
    api.anvil_impersonate_account(from).await.unwrap();
    let tx = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    let status = tx.inner.inner.inner.receipt.status.coerce_status();
    assert!(status);

    // reset to check timestamp works after resetting
    api.anvil_reset(Some(Forking {
        json_rpc_url: Some("https://moonbeam.api.onfinality.io/public".to_string()),
        block_number: None,
    }))
    .await
    .unwrap();

    let tx =
        TransactionRequest::default().to(Address::random()).value(U256::from(1337u64)).from(from);
    let tx = WithOtherFields::new(tx);
    let tx = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    let status = tx.inner.inner.inner.receipt.status.coerce_status();
    assert!(status);
}

// <https://github.com/foundry-rs/foundry/issues/6640
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_basefee() {
    // <https://etherscan.io/block/18835000>
    let (api, _handle) = spawn(fork_config().with_fork_block_number(Some(18835000u64))).await;

    api.mine_one().await.unwrap();
    let latest = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();

    // basefee of +1 block: <https://etherscan.io/block/18835001>
    assert_eq!(latest.header.base_fee_per_gas.unwrap(), 59455969592u64);

    // now reset to block 18835000 -1
    api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(18835000u64 - 1) }))
        .await
        .unwrap();

    api.mine_one().await.unwrap();
    let latest = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();

    // basefee of the forked block: <https://etherscan.io/block/18835000>
    assert_eq!(latest.header.base_fee_per_gas.unwrap(), 59017001138);
}

// <https://github.com/foundry-rs/foundry/issues/6795>
#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_arbitrum_fork_dev_balance() {
    let (api, handle) = spawn(
        fork_config()
            .with_fork_block_number(None::<u64>)
            .with_eth_rpc_url(Some(next_rpc_endpoint(NamedChain::Arbitrum))),
    )
    .await;

    let accounts: Vec<_> = handle.dev_wallets().collect();
    for acc in accounts {
        let balance = api.balance(acc.address(), Some(Default::default())).await.unwrap();
        assert_eq!(balance, U256::from(100000000000000000000u128));
    }
}

// <https://github.com/foundry-rs/foundry/issues/9152>
#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_arb_fork_mining() {
    let fork_block_number = 394274860u64;
    let fork_rpc = next_rpc_endpoint(NamedChain::Arbitrum);
    let (api, _handle) = spawn(
        fork_config()
            .with_fork_block_number(Some(fork_block_number))
            .with_eth_rpc_url(Some(fork_rpc)),
    )
    .await;

    let init_blk_num = api.block_number().unwrap().to::<u64>();

    // Mine one
    api.mine_one().await.unwrap();
    let mined_blk_num = api.block_number().unwrap().to::<u64>();

    assert_eq!(mined_blk_num, init_blk_num + 1);
}

// <https://github.com/foundry-rs/foundry/issues/6749>
#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_arbitrum_fork_block_number() {
    // Every fork below must observe the same chain head, so reuse one endpoint: providers are a few
    // blocks apart and refetching would otherwise fork at a block the next provider has not seen.
    let fork_rpc = next_rpc_endpoint(NamedChain::Arbitrum);

    // fork to get initial block for test
    let (_, handle) = spawn(
        fork_config().with_fork_block_number(None::<u64>).with_eth_rpc_url(Some(fork_rpc.clone())),
    )
    .await;
    let provider = handle.http_provider();
    let initial_block_number = provider.get_block_number().await.unwrap();

    // fork again at block number returned by `eth_blockNumber`
    // if wrong block number returned (e.g. L1) then fork will fail with error code -32000: missing
    // trie node
    let (api, _) = spawn(
        fork_config()
            .with_fork_block_number(Some(initial_block_number))
            .with_eth_rpc_url(Some(fork_rpc.clone())),
    )
    .await;
    let block_number = api.block_number().unwrap().to::<u64>();
    assert_eq!(block_number, initial_block_number);

    // take snapshot at initial block number
    let snapshot_state = api.evm_snapshot().await.unwrap();

    // mine new block and check block number returned by `eth_blockNumber`
    api.mine_one().await.unwrap();
    let block_number = api.block_number().unwrap().to::<u64>();
    assert_eq!(block_number, initial_block_number + 1);

    // test block by number API call returns proper block number and `l1BlockNumber` is set
    let block_by_number = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert_eq!(block_by_number.header.number, initial_block_number + 1);
    assert!(block_by_number.other.get("l1BlockNumber").is_some());

    // revert to recorded snapshot and check block number
    assert!(api.evm_revert(snapshot_state).await.unwrap());
    let block_number = api.block_number().unwrap().to::<u64>();
    assert_eq!(block_number, initial_block_number);

    // reset fork to different block number and compare with block returned by `eth_blockNumber`
    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(fork_rpc),
        block_number: Some(initial_block_number - 2),
    }))
    .await
    .unwrap();
    let block_number = api.block_number().unwrap().to::<u64>();
    assert_eq!(block_number, initial_block_number - 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_base_fork_gas_limit() {
    // fork to get initial block for test
    let (api, handle) = spawn(
        fork_config()
            .with_fork_block_number(None::<u64>)
            .with_eth_rpc_url(Some(next_rpc_endpoint(NamedChain::Base))),
    )
    .await;

    // The public Base RPC occasionally returns block zero when it is unhealthy.
    if api.block_number().unwrap().is_zero() {
        return;
    }

    let provider = handle.http_provider();
    let block =
        provider.get_block(BlockId::Number(BlockNumberOrTag::Latest)).await.unwrap().unwrap();

    assert!(api.gas_limit() >= uint!(96_000_000_U256));
    assert!(block.header.gas_limit >= 96_000_000_u64);
}

// <https://github.com/foundry-rs/foundry/issues/7023>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_execution_reverted() {
    let target = 16681681u64;
    let (api, _handle) = spawn(fork_config().with_fork_block_number(Some(target + 1))).await;

    let resp = api
        .call(
            WithOtherFields::new(TransactionRequest {
                to: Some(TxKind::from(address!("0xFd6CC4F251eaE6d02f9F7B41D1e80464D3d2F377"))),
                input: TransactionInput::new(bytes!("8f283b3c")),
                ..Default::default()
            }),
            Some(target.into()),
            EvmOverrides::default(),
        )
        .await;

    assert!(resp.is_err());
    let err = resp.unwrap_err();
    assert!(err.to_string().contains("execution reverted"));
}

// <https://github.com/foundry-rs/foundry/issues/8227>
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_immutable_fork_transaction_hash() {
    use std::str::FromStr;

    // Fork to a block with a specific transaction
    // <https://explorer.immutable.com/tx/0x39d64ebf9eb3f07ede37f8681bc3b61928817276c4c4680b6ef9eac9f88b6786>
    let fork_tx_hash =
        TxHash::from_str("2ac736ce725d628ef20569a1bb501726b42b33f9d171f60b92b69de3ce705845")
            .unwrap();
    let (api, _) = spawn(
        fork_config()
            .with_blocktime(Some(Duration::from_millis(500)))
            .with_fork_transaction_hash(Some(fork_tx_hash))
            .with_eth_rpc_url(Some("https://immutable-zkevm.drpc.org".to_string())),
    )
    .await;

    let fork_block_number = 21824325;

    // The prefix is installed before startup returns.
    let block_number = api.block_number().unwrap().to::<u64>();
    assert_eq!(block_number, fork_block_number);

    let block = api
        .block_by_number(BlockNumberOrTag::Number(fork_block_number - 1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(block.transactions.len(), 6);
    let block = api
        .block_by_number_full(BlockNumberOrTag::Number(fork_block_number))
        .await
        .unwrap()
        .unwrap();
    assert!(!block.transactions.is_empty());

    // Validate the transactions preceding the target transaction exist
    let expected_transactions = [
        TxHash::from_str("c900784c993221ba192c53a3ff9996f6af83a951100ceb93e750f7ef86bd43d5")
            .unwrap(),
        TxHash::from_str("f86f001bbdf69f8f64ff8a4a5fc3e684cf3a7706f204eba8439752f6f67cd2c4")
            .unwrap(),
        fork_tx_hash,
    ];
    for expected in [
        (expected_transactions[0], address!("0x0a02a416f87a13626dda0ad386859497565222aa")),
        (expected_transactions[1], address!("0x0a02a416f87a13626dda0ad386859497565222aa")),
        (expected_transactions[2], address!("0x4f07d669d76ed9a17799fc4c04c4005196240940")),
    ] {
        let tx = api.backend.mined_transaction_by_hash(expected.0).unwrap();
        assert_eq!(tx.inner.inner.signer(), expected.1);
    }

    // Validate the order of transactions in the new block
    for expected in [
        (expected_transactions[0], 0),
        (expected_transactions[1], 1),
        (expected_transactions[2], 2),
    ] {
        let tx = api
            .backend
            .mined_block_by_number(BlockNumberOrTag::Number(fork_block_number))
            .map(|b| b.header.hash)
            .and_then(|hash| {
                api.backend.mined_transaction_by_block_hash_and_index(hash, expected.1.into())
            })
            .unwrap();
        assert_eq!(tx.tx_hash().to_string(), expected.0.to_string());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_block_by_number_full_refetches_missing_cached_transactions() {
    let (api, _) = spawn(fork_config()).await;

    let block =
        api.block_by_number_full(BlockNumberOrTag::Number(BLOCK_NUMBER)).await.unwrap().unwrap();
    let block_txs = block.transactions.as_transactions().unwrap();
    let original_len = block_txs.len();
    let missing_hash = *block_txs[0].tx_hash();

    let fork = api.backend.get_fork().unwrap();
    {
        let mut storage = fork.storage.write();
        assert!(storage.transactions.remove(&missing_hash).is_some());
    }

    let refreshed =
        api.block_by_number_full(BlockNumberOrTag::Number(BLOCK_NUMBER)).await.unwrap().unwrap();
    let refreshed_txs = refreshed.transactions.as_transactions().unwrap();

    assert_eq!(refreshed_txs.len(), original_len);
    assert_eq!(refreshed_txs[0].tx_hash(), &missing_hash);
    assert!(fork.storage.read().transactions.contains_key(&missing_hash));
}

// <https://github.com/foundry-rs/foundry/issues/4700>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_query_at_fork_block() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();
    let info = api.anvil_node_info().await.unwrap();
    let number = info.fork_config.fork_block_number.unwrap();
    assert_eq!(number, BLOCK_NUMBER);

    let address = Address::random();

    let balance = provider.get_balance(address).await.unwrap();
    api.evm_mine(None).await.unwrap();
    api.anvil_set_balance(address, balance + U256::from(1)).await.unwrap();

    let balance_before =
        provider.get_balance(address).block_id(BlockId::number(number)).await.unwrap();

    assert_eq!(balance_before, balance);
}

// <https://github.com/foundry-rs/foundry/issues/4173>
#[tokio::test(flavor = "multi_thread")]
async fn test_reset_dev_account_nonce() {
    let config: NodeConfig = fork_config();
    let address = config.genesis_accounts[0].address();
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let info = api.anvil_node_info().await.unwrap();
    let number = info.fork_config.fork_block_number.unwrap();
    assert_eq!(number, BLOCK_NUMBER);

    let nonce_before = provider.get_transaction_count(address).await.unwrap();

    // Reset to older block with other nonce
    api.anvil_reset(Some(Forking {
        json_rpc_url: None,
        block_number: Some(BLOCK_NUMBER - 1_000_000),
    }))
    .await
    .unwrap();

    let nonce_after = provider.get_transaction_count(address).await.unwrap();

    assert!(nonce_before > nonce_after);

    let receipt = provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .from(address)
                .to(address)
                .nonce(nonce_after)
                .gas_limit(21000),
        ))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    assert!(receipt.status());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_set_erc20_balance() {
    let config: NodeConfig = fork_config();
    let address = config.genesis_accounts[0].address();
    let (api, handle) = spawn(config).await;

    let provider = handle.http_provider();

    alloy_sol_types::sol! {
       #[sol(rpc)]
       contract ERC20 {
            function balanceOf(address owner) public view returns (uint256);
       }
    }
    let dai = address!("0x6B175474E89094C44Da98b954EedeAC495271d0F");
    let erc20 = ERC20::new(dai, provider);
    let value = U256::from(500);

    api.anvil_deal_erc20(address, dai, value).await.unwrap();

    let new_balance = erc20.balanceOf(address).call().await.unwrap();

    assert_eq!(new_balance, value);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_set_erc20_allowance() {
    let config: NodeConfig = fork_config();
    let owner = config.genesis_accounts[0].address();
    let spender = config.genesis_accounts[1].address();
    let (api, handle) = spawn(config).await;

    let provider = handle.http_provider();

    alloy_sol_types::sol! {
       #[sol(rpc)]
       contract ERC20 {
            function allowance(address owner, address spender) external view returns (uint256);
       }
    }
    let dai = address!("0x6B175474E89094C44Da98b954EedeAC495271d0F");
    let erc20 = ERC20::new(dai, provider);
    let value = U256::from(500);

    api.anvil_set_erc20_allowance(owner, spender, dai, value).await.unwrap();

    let allowance = erc20.allowance(owner, spender).call().await.unwrap();
    assert_eq!(allowance, value);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_add_balance() {
    let config: NodeConfig = fork_config();
    let address = config.genesis_accounts[0].address();
    let (api, _handle) = spawn(config).await;

    let start_balance = U256::from(100_000_u64);
    api.anvil_set_balance(address, start_balance).await.unwrap();

    let balance_increase = U256::from(50_000_u64);
    api.anvil_add_balance(address, balance_increase).await.unwrap();

    let new_balance = api.balance(address, None).await.unwrap();
    assert_eq!(new_balance, start_balance + balance_increase);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reset_updates_cache_path_when_rpc_url_not_provided() {
    let config: NodeConfig = fork_config();

    let (mut api, _handle) = spawn(config).await;
    let info = api.anvil_node_info().await.unwrap();
    let number = info.fork_config.fork_block_number.unwrap();
    assert_eq!(number, BLOCK_NUMBER);

    async fn get_block_from_cache_path(api: &mut EthApi<FoundryNetwork>) -> u64 {
        let db = api.backend.get_db().read().await;
        let cache_path = db.maybe_inner().unwrap().cache().cache_path().unwrap();
        cache_path
            .parent()
            .expect("must have filename")
            .file_name()
            .expect("must have block number as dir name")
            .to_str()
            .expect("must be valid string")
            .parse::<u64>()
            .expect("must be valid number")
    }

    assert_eq!(BLOCK_NUMBER, get_block_from_cache_path(&mut api).await);

    // Reset to older block without specifying a new rpc url
    api.anvil_reset(Some(Forking {
        json_rpc_url: None,
        block_number: Some(BLOCK_NUMBER - 1_000_000),
    }))
    .await
    .unwrap();

    assert_eq!(BLOCK_NUMBER - 1_000_000, get_block_from_cache_path(&mut api).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_reuses_cached_remote_state() {
    let address = Address::random();
    let balance = U256::from(1337u64);
    let chain_id =
        u64::from_be_bytes(address.as_slice()[12..].try_into().unwrap()) % 1_000_000 + 1_000_000;
    let cache_dir = Config::foundry_chain_cache_dir(chain_id).unwrap();
    let _ = std::fs::remove_dir_all(&cache_dir);

    let origin_config = NodeConfig::test()
        .with_chain_id(Some(chain_id))
        .with_funded_accounts([(address, balance)].into_iter().collect());
    let (_origin_api, origin_handle) = spawn(origin_config).await;
    let fork_config = NodeConfig::test()
        .with_chain_id(Some(chain_id))
        .with_eth_rpc_url(Some(origin_handle.http_endpoint()));
    let (api, handle) = spawn(fork_config).await;
    let provider = handle.http_provider();
    let fork_block_number = api.anvil_node_info().await.unwrap().fork_config.fork_block_number;

    assert_eq!(provider.get_balance(address).await.unwrap(), balance);
    api.mine_one().await.unwrap();

    for _ in 0..2 {
        api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: fork_block_number }))
            .await
            .unwrap();

        let db = api.backend.get_db().read().await;
        assert!(db.maybe_inner().unwrap().accounts().read().contains_key(&address));
    }

    api.anvil_reset(None).await.unwrap();
    let (cached_api, _) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_eth_rpc_url(Some(origin_handle.http_endpoint())),
    )
    .await;
    let db = cached_api.backend.get_db().read().await;
    assert!(db.maybe_inner().unwrap().accounts().read().contains_key(&address));

    let _ = std::fs::remove_dir_all(cache_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_block_zero_does_not_reuse_cache_for_new_rpc_url() {
    let address = Address::random();
    let first_balance = U256::from(1337u64);
    let second_balance = U256::from(42u64);
    let timestamp = 1_000_000u64;
    let chain_id =
        u64::from_be_bytes(address.as_slice()[12..].try_into().unwrap()) % 1_000_000 + 1_000_000;
    let cache_dir = Config::foundry_chain_cache_dir(chain_id).unwrap();
    let _ = std::fs::remove_dir_all(&cache_dir);

    async {
        let first_origin = NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_genesis_timestamp(Some(timestamp))
            .with_funded_accounts([(address, first_balance)].into_iter().collect());
        let (_first_origin_api, first_origin_handle) = spawn(first_origin).await;
        let second_origin = NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_genesis_timestamp(Some(timestamp))
            .with_funded_accounts([(address, second_balance)].into_iter().collect());
        let (_second_origin_api, second_origin_handle) = spawn(second_origin).await;
        let fork_config = NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_eth_rpc_url(Some(first_origin_handle.http_endpoint()))
            .with_fork_block_number(Some(0u64));
        let (api, handle) = spawn(fork_config).await;
        let provider = handle.http_provider();

        assert_eq!(provider.get_balance(address).await.unwrap(), first_balance);

        api.anvil_reset(Some(Forking {
            json_rpc_url: Some(second_origin_handle.http_endpoint()),
            block_number: Some(0u64),
        }))
        .await
        .unwrap();

        assert_eq!(provider.get_balance(address).await.unwrap(), second_balance);

        let second_fork_config = NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_eth_rpc_url(Some(second_origin_handle.http_endpoint()))
            .with_fork_block_number(Some(0u64));
        let (_second_fork_api, second_fork_handle) = spawn(second_fork_config).await;
        assert_eq!(
            second_fork_handle.http_provider().get_balance(address).await.unwrap(),
            second_balance
        );
    }
    .await;
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_does_not_reuse_cache_for_new_rpc_url() {
    let address = Address::random();
    let first_balance = U256::from(1337u64);
    let second_balance = U256::from(42u64);
    let timestamp = 1_000_000u64;
    let chain_id =
        u64::from_be_bytes(address.as_slice()[12..].try_into().unwrap()) % 1_000_000 + 1_000_000;
    let cache_dir = Config::foundry_chain_cache_dir(chain_id).unwrap();
    let _ = std::fs::remove_dir_all(&cache_dir);

    async {
        let first_origin = NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_genesis_timestamp(Some(timestamp))
            .with_funded_accounts([(address, first_balance)].into_iter().collect());
        let (first_origin_api, first_origin_handle) = spawn(first_origin).await;
        let second_origin = NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_genesis_timestamp(Some(timestamp))
            .with_funded_accounts([(address, second_balance)].into_iter().collect());
        let (second_origin_api, second_origin_handle) = spawn(second_origin).await;
        first_origin_api.mine_one().await.unwrap();
        first_origin_api.mine_one().await.unwrap();
        second_origin_api.mine_one().await.unwrap();
        second_origin_api.mine_one().await.unwrap();
        let fork_config = NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_eth_rpc_url(Some(first_origin_handle.http_endpoint()))
            .with_fork_block_number(Some(1u64));
        let (api, handle) = spawn(fork_config).await;
        let provider = handle.http_provider();
        let fork_block_number = api.anvil_node_info().await.unwrap().fork_config.fork_block_number;

        assert_eq!(provider.get_balance(address).await.unwrap(), first_balance);
        api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(2) })).await.unwrap();
        assert_eq!(provider.get_balance(address).await.unwrap(), first_balance);

        let local_balance = U256::from(9001u64);
        api.anvil_set_balance(address, local_balance).await.unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let unavailable_url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        assert!(
            api.anvil_reset(Some(Forking {
                json_rpc_url: Some(unavailable_url),
                block_number: fork_block_number,
            }))
            .await
            .is_err()
        );
        assert_eq!(provider.get_balance(address).await.unwrap(), local_balance);

        api.anvil_reset(Some(Forking {
            json_rpc_url: Some(second_origin_handle.http_endpoint()),
            block_number: Some(2),
        }))
        .await
        .unwrap();

        assert_eq!(provider.get_balance(address).await.unwrap(), second_balance);

        api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: fork_block_number }))
            .await
            .unwrap();

        assert_eq!(provider.get_balance(address).await.unwrap(), second_balance);

        let second_fork_config = NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_eth_rpc_url(Some(second_origin_handle.http_endpoint()));
        let (_second_fork_api, second_fork_handle) = spawn(second_fork_config).await;
        let second_fork_provider = second_fork_handle.http_provider();
        assert_eq!(second_fork_provider.get_balance(address).await.unwrap(), second_balance);

        api.anvil_reset(None).await.unwrap();
        api.anvil_set_rpc_url(first_origin_handle.http_endpoint()).await.unwrap();
        api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(2) })).await.unwrap();
        assert_eq!(provider.get_balance(address).await.unwrap(), first_balance);
    }
    .await;
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_after_set_rpc_url_does_not_reuse_old_cache() {
    let address = Address::random();
    let first_balance = U256::from(1337u64);
    let second_balance = U256::from(42u64);
    let timestamp = 1_000_000u64;
    let chain_id =
        u64::from_be_bytes(address.as_slice()[12..].try_into().unwrap()) % 1_000_000 + 1_000_000;
    let cache_dir = Config::foundry_chain_cache_dir(chain_id).unwrap();
    let _ = std::fs::remove_dir_all(&cache_dir);

    async {
        let first_origin = NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_genesis_timestamp(Some(timestamp))
            .with_funded_accounts([(address, first_balance)].into_iter().collect());
        let (_first_origin_api, first_origin_handle) = spawn(first_origin).await;
        let second_origin = NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_genesis_timestamp(Some(timestamp))
            .with_funded_accounts([(address, second_balance)].into_iter().collect());
        let (_second_origin_api, second_origin_handle) = spawn(second_origin).await;
        let fork_config = NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_eth_rpc_url(Some(first_origin_handle.http_endpoint()));
        let (api, handle) = spawn(fork_config).await;
        let provider = handle.http_provider();

        assert_eq!(provider.get_balance(address).await.unwrap(), first_balance);

        api.anvil_set_rpc_url(second_origin_handle.http_endpoint()).await.unwrap();
        api.anvil_reset(Some(Forking::default())).await.unwrap();

        assert_eq!(provider.get_balance(address).await.unwrap(), second_balance);
    }
    .await;
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_invalidates_cache_for_same_url_anvil_identity_change() {
    let address = Address::random();
    let first_balance = U256::from(1337u64);
    let second_balance = U256::from(42u64);
    let third_balance = U256::from(9001u64);
    let chain_id =
        u64::from_be_bytes(address.as_slice()[12..].try_into().unwrap()) % 1_000_000 + 1_000_000;
    let cache_dir = Config::foundry_chain_cache_dir(chain_id).unwrap();
    let _ = std::fs::remove_dir_all(&cache_dir);

    async {
        let origin_config = NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_funded_accounts([(address, first_balance)].into_iter().collect());
        let (origin_api, origin_handle) = spawn(origin_config).await;
        let origin_url = origin_handle.http_endpoint();
        let fork_config = NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_eth_rpc_url(Some(origin_url.clone()));
        let (api, handle) = spawn(fork_config).await;
        let provider = handle.http_provider();

        assert_eq!(provider.get_balance(address).await.unwrap(), first_balance);
        api.backend.get_db().read().await.maybe_flush_cache().unwrap();

        let first_instance_id = origin_api.instance_id();
        origin_api.anvil_reset(None).await.unwrap();
        assert_ne!(origin_api.instance_id(), first_instance_id);
        origin_api.anvil_set_balance(address, second_balance).await.unwrap();

        api.anvil_reset(Some(Forking {
            json_rpc_url: Some(origin_url.clone()),
            block_number: Some(0),
        }))
        .await
        .unwrap();
        assert_eq!(provider.get_balance(address).await.unwrap(), second_balance);

        // The source identity must survive an intervening reset to memory as well.
        api.anvil_reset(None).await.unwrap();
        let second_instance_id = origin_api.instance_id();
        origin_api.anvil_reset(None).await.unwrap();
        assert_ne!(origin_api.instance_id(), second_instance_id);
        origin_api.anvil_set_balance(address, third_balance).await.unwrap();

        api.anvil_reset(Some(Forking { json_rpc_url: Some(origin_url), block_number: Some(0) }))
            .await
            .unwrap();
        assert_eq!(provider.get_balance(address).await.unwrap(), third_balance);
    }
    .await;

    let _ = std::fs::remove_dir_all(cache_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_failed_same_family_fork_reset_preserves_live_state() {
    let (origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    origin_api.mine_one().await.unwrap();
    let origin_url = origin_handle.http_endpoint();
    let (api, handle) = spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_url.clone()))).await;
    let provider = handle.http_provider();
    let marker = Address::random();
    let marker_balance = U256::from(987_654u64);
    let marker_nonce = 17u64;
    api.anvil_set_balance(marker, marker_balance).await.unwrap();
    api.anvil_set_nonce(marker, U256::from(marker_nonce)).await.unwrap();
    api.mine_one().await.unwrap();

    let info_before = api.anvil_node_info().await.unwrap();
    let metadata_before = api.anvil_metadata().await.unwrap();
    let block_before = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    let gas_price_before = api.gas_price();
    let base_fee_before = api.base_fee().unwrap();
    let gas_limit_before = api.gas_limit();

    let err = api
        .anvil_reset(Some(Forking {
            json_rpc_url: Some(origin_url),
            block_number: Some(1_000_000),
        }))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Failed to get block"), "{err}");

    let info_after = api.anvil_node_info().await.unwrap();
    let metadata_after = api.anvil_metadata().await.unwrap();
    let block_after = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(metadata_after.instance_id, metadata_before.instance_id);
    assert_eq!(metadata_after.forked_network, metadata_before.forked_network);
    assert_eq!(info_after.current_block_number, info_before.current_block_number);
    assert_eq!(info_after.current_block_hash, info_before.current_block_hash);
    assert_eq!(info_after.current_block_timestamp, info_before.current_block_timestamp);
    assert_eq!(info_after.hard_fork, info_before.hard_fork);
    assert_eq!(info_after.environment, info_before.environment);
    assert_eq!(info_after.fork_config, info_before.fork_config);
    assert_eq!(block_after.header.hash, block_before.header.hash);
    assert_eq!(api.gas_price(), gas_price_before);
    assert_eq!(api.base_fee().unwrap(), base_fee_before);
    assert_eq!(api.gas_limit(), gas_limit_before);
    assert_eq!(provider.get_balance(marker).await.unwrap(), marker_balance);
    assert_eq!(provider.get_transaction_count(marker).await.unwrap(), marker_nonce);

    api.mine_one().await.unwrap();
    assert_eq!(provider.get_block_number().await.unwrap(), block_before.header.number + 1);
    assert_eq!(provider.get_balance(marker).await.unwrap(), marker_balance);
    assert_eq!(provider.get_transaction_count(marker).await.unwrap(), marker_nonce);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_resets_invalidate_state_snapshots_without_mutating_new_context() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let (api, handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_handle.http_endpoint()))).await;
    let provider = handle.http_provider();
    let marker = Address::random();

    for reset in [Some(Forking::default()), None] {
        api.anvil_set_balance(marker, U256::from(123u64)).await.unwrap();
        api.mine_one().await.unwrap();
        let snapshot = api.evm_snapshot().await.unwrap();
        assert!(api.anvil_metadata().await.unwrap().snapshots.contains_key(&snapshot));

        api.anvil_reset(reset).await.unwrap();
        assert!(api.anvil_metadata().await.unwrap().snapshots.is_empty());
        let info = api.anvil_node_info().await.unwrap();
        let balance = provider.get_balance(marker).await.unwrap();

        assert!(!api.evm_revert(snapshot).await.unwrap());
        let info_after_revert = api.anvil_node_info().await.unwrap();
        assert_eq!(info_after_revert.current_block_number, info.current_block_number);
        assert_eq!(info_after_revert.current_block_hash, info.current_block_hash);
        assert_eq!(info_after_revert.current_block_timestamp, info.current_block_timestamp);
        assert_eq!(provider.get_balance(marker).await.unwrap(), balance);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_to_new_url_does_not_reuse_old_cache() {
    let address = Address::random();
    let first_balance = U256::from(1337u64);
    let second_balance = U256::from(42u64);
    let timestamp = 1_000_000u64;
    let chain_id =
        u64::from_be_bytes(address.as_slice()[12..].try_into().unwrap()) % 1_000_000 + 1_000_000;
    let cache_dir = Config::foundry_chain_cache_dir(chain_id).unwrap();
    let _ = std::fs::remove_dir_all(&cache_dir);

    async {
        let first_origin = NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_genesis_timestamp(Some(timestamp))
            .with_funded_accounts([(address, first_balance)].into_iter().collect());
        let (_first_origin_api, first_origin_handle) = spawn(first_origin).await;
        let second_origin = NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_genesis_timestamp(Some(timestamp))
            .with_funded_accounts([(address, second_balance)].into_iter().collect());
        let (_second_origin_api, second_origin_handle) = spawn(second_origin).await;
        let fork_config = NodeConfig::test()
            .with_chain_id(Some(chain_id))
            .with_eth_rpc_url(Some(first_origin_handle.http_endpoint()));
        let (api, handle) = spawn(fork_config).await;
        let provider = handle.http_provider();

        assert_eq!(provider.get_balance(address).await.unwrap(), first_balance);
        let instance_id = api.anvil_metadata().await.unwrap().instance_id;
        api.anvil_reset(Some(Forking {
            json_rpc_url: Some(second_origin_handle.http_endpoint()),
            block_number: None,
        }))
        .await
        .unwrap();

        assert_ne!(api.anvil_metadata().await.unwrap().instance_id, instance_id);
        assert_eq!(provider.get_balance(address).await.unwrap(), second_balance);
    }
    .await;
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_to_new_url_updates_source_chain_id() {
    let (_first_origin_api, first_origin_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(1u64))).await;
    let (_second_origin_api, second_origin_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(56u64))).await;
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_no_storage_caching(true)
            .with_eth_rpc_url(Some(first_origin_handle.http_endpoint())),
    )
    .await;
    let provider = handle.http_provider();

    assert_eq!(provider.get_chain_id().await.unwrap(), 1);
    assert_eq!(api.anvil_metadata().await.unwrap().forked_network.unwrap().chain_id, 1);
    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(second_origin_handle.http_endpoint()),
        block_number: None,
    }))
    .await
    .unwrap();

    assert_eq!(provider.get_chain_id().await.unwrap(), 56);
    assert_eq!(api.anvil_metadata().await.unwrap().forked_network.unwrap().chain_id, 56);

    let from = handle.dev_accounts().next().unwrap();
    let send = || {
        provider.send_transaction(WithOtherFields::new(
            TransactionRequest::default().from(from).to(Address::random()).value(U256::from(1)),
        ))
    };
    assert!(send().await.unwrap().get_receipt().await.unwrap().status());

    api.anvil_reset(None).await.unwrap();
    assert_eq!(provider.get_chain_id().await.unwrap(), 31337);
    assert!(send().await.unwrap().get_receipt().await.unwrap().status());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_pre_cancun_fork_with_post_cancun_hardfork() {
    let target = Address::random();
    let (origin_api, origin_handle) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(NamedChain::Mainnet as u64))
            .with_hardfork(Some(EthereumHardfork::Shanghai.into()))
            .with_genesis_timestamp(EthereumHardfork::Cancun.mainnet_activation_timestamp()),
    )
    .await;
    origin_api.anvil_set_code(target, bytes!("600060005260206000f3")).await.unwrap();
    origin_api.mine_one().await.unwrap();
    let origin_url = origin_handle.http_endpoint();

    for hardfork in [EthereumHardfork::Cancun, EthereumHardfork::Prague] {
        let (api, handle) = spawn(
            NodeConfig::test()
                .with_eth_rpc_url(Some(origin_url.clone()))
                .with_fork_block_number(Some(1u64))
                .with_hardfork(Some(hardfork.into())),
        )
        .await;
        let provider = handle.http_provider();
        let request =
            || TransactionRequest { to: Some(TxKind::Call(target)), ..Default::default() };

        assert_eq!(provider.call(request().into()).await.unwrap(), Bytes::from(vec![0; 32]));
        assert_eq!(
            api.backend
                .evm_env()
                .read()
                .block_env
                .blob_excess_gas_and_price
                .as_ref()
                .map(|blob| blob.excess_blob_gas),
            Some(0)
        );

        api.anvil_reset(Some(Forking {
            json_rpc_url: Some(origin_url.clone()),
            block_number: Some(1),
        }))
        .await
        .unwrap();
        assert_eq!(provider.call(request().into()).await.unwrap(), Bytes::from(vec![0; 32]));
    }

    let partial_header_url =
        spawn_rpc_proxy_with_blob_header_fields(origin_url, Some(0), None).await;
    let partial_header_url =
        spawn_rpc_proxy_rejecting_method_after(partial_header_url, "anvil_nodeInfo", 0).await;
    let (api, _) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(partial_header_url))
            .with_fork_block_number(Some(1u64))
            .with_hardfork(Some(EthereumHardfork::Prague.into())),
    )
    .await;
    assert!(api.backend.evm_env().read().block_env.blob_excess_gas_and_price.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_unknown_schedule_fork_with_post_cancun_hardfork() {
    let target = Address::random();
    let (origin_api, origin_handle) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(NamedChain::BinanceSmartChain as u64))
            .with_hardfork(Some(EthereumHardfork::Shanghai.into()))
            .with_genesis_timestamp(EthereumHardfork::Cancun.mainnet_activation_timestamp()),
    )
    .await;
    origin_api.anvil_set_code(target, bytes!("600060005260206000f3")).await.unwrap();
    origin_api.mine_one().await.unwrap();
    let origin_url =
        spawn_rpc_proxy_with_blob_header_fields(origin_handle.http_endpoint(), None, None).await;
    let origin_url = spawn_rpc_proxy_rejecting_method_after(origin_url, "anvil_nodeInfo", 0).await;
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_url.clone()))
            .with_fork_block_number(Some(1u64))
            .with_hardfork(Some(EthereumHardfork::Prague.into())),
    )
    .await;

    assert_eq!(
        api.backend
            .evm_env()
            .read()
            .block_env
            .blob_excess_gas_and_price
            .as_ref()
            .map(|blob| blob.excess_blob_gas),
        Some(0)
    );
    assert_eq!(
        handle
            .http_provider()
            .call(
                TransactionRequest { to: Some(TxKind::Call(target)), ..Default::default() }.into()
            )
            .await
            .unwrap(),
        Bytes::from(vec![0; 32])
    );

    let (api, _) = spawn(
        NodeConfig::test().with_eth_rpc_url(Some(origin_url)).with_fork_block_number(Some(1u64)),
    )
    .await;
    assert!(api.backend.evm_env().read().block_env.blob_excess_gas_and_price.is_none());
}

async fn spawn_rpc_proxy_with_blob_header_fields(
    endpoint: String,
    blob_gas_used: Option<u64>,
    excess_blob_gas: Option<u64>,
) -> String {
    let client = reqwest::Client::new();
    let router = Router::new().route(
        "/",
        post(move |Json(request): Json<Value>| {
            let client = client.clone();
            let endpoint = endpoint.clone();
            async move {
                let mut response = client
                    .post(endpoint)
                    .json(&request)
                    .send()
                    .await
                    .unwrap()
                    .json::<Value>()
                    .await
                    .unwrap();
                if matches!(
                    request.get("method").and_then(Value::as_str),
                    Some("eth_getBlockByHash" | "eth_getBlockByNumber")
                ) && let Some(block) = response.get_mut("result").and_then(Value::as_object_mut)
                {
                    for (field, value) in
                        [("blobGasUsed", blob_gas_used), ("excessBlobGas", excess_blob_gas)]
                    {
                        if let Some(value) = value {
                            block.insert(field.to_string(), Value::String(format!("0x{value:x}")));
                        } else {
                            block.remove(field);
                        }
                    }
                }
                Json(response)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{address}")
}

#[tokio::test(flavor = "multi_thread")]
async fn test_optimism_fork_keeps_excess_blob_gas_zero_after_mining() {
    // Base Jovian stores the DA footprint in `blobGasUsed`, but OP Stack clients keep
    // `excessBlobGas` at zero because the chain does not support EIP-4844 blobs. This captured
    // footprint is from Base mainnet block 50_729_760.
    let (_, origin) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(NamedChain::Base as u64))
            .with_networks(NetworkConfigs::with_optimism())
            .with_hardfork(Some(OpHardfork::Jovian.into())),
    )
    .await;
    let fork_url =
        spawn_rpc_proxy_with_blob_header_fields(origin.http_endpoint(), Some(0x2e_b434), Some(0))
            .await;
    let (api, _) = spawn(
        NodeConfig::test().with_eth_rpc_url(Some(fork_url)).with_fork_block_number(Some(0u64)),
    )
    .await;

    api.mine_one().await.unwrap();
    let block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();

    assert_eq!(block.header.excess_blob_gas, Some(0));
    let next_blob_fee = api.excess_blob_gas_and_price().unwrap().unwrap();
    assert_eq!(next_blob_fee.excess_blob_gas, 0);
    assert_eq!(next_blob_fee.blob_gasprice, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_arbitrum_forks_accept_nitro_headers_without_blob_fields() {
    // Nitro omits Ethereum's EIP-4844 header fields, including on post-Cancun chains. Hide Anvil
    // metadata so these deterministic local origins have the same observable shape as public RPCs.
    for chain in [NamedChain::Arbitrum, NamedChain::Robinhood] {
        let target = Address::random();
        let (origin_api, origin) = spawn(
            NodeConfig::test()
                .with_chain_id(Some(chain as u64))
                .with_hardfork(Some(EthereumHardfork::Prague.into()))
                .with_genesis_timestamp(EthereumHardfork::Prague.arbitrum_activation_timestamp()),
        )
        .await;
        origin_api.anvil_set_code(target, bytes!("600060005260206000f3")).await.unwrap();
        let fork_url =
            spawn_rpc_proxy_with_blob_header_fields(origin.http_endpoint(), None, None).await;
        let fork_url = spawn_rpc_proxy_rejecting_method_after(fork_url, "anvil_nodeInfo", 0).await;

        let (api, handle) = spawn(
            NodeConfig::test()
                .with_no_storage_caching(true)
                .with_eth_rpc_url(Some(fork_url))
                .with_fork_block_number(Some(0u64)),
        )
        .await;

        assert!(api.backend.spec_id() >= SpecId::CANCUN);
        assert_eq!(
            api.backend
                .evm_env()
                .read()
                .block_env
                .blob_excess_gas_and_price
                .as_ref()
                .map(|blob| blob.excess_blob_gas),
            Some(0),
            "{chain}"
        );
        let request = TransactionRequest { to: Some(TxKind::Call(target)), ..Default::default() };
        assert_eq!(
            handle.http_provider().call(request.into()).await.unwrap(),
            Bytes::from(vec![0; 32]),
            "{chain}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_polygon_fork_missing_blob_fields_is_chain_scoped_across_reset() {
    // Polygon's Bor headers omit the EIP-4844 fields even though Anvil executes the fork with a
    // post-Cancun spec. Local origins provide deterministic state while the proxies reproduce that
    // header shape and hide Anvil metadata, matching an external endpoint.
    let token = address!("0d500B1d8E8eF31E21C99d1Db9A6444d3ADf1270");
    let return_zero = bytes!("600060005260206000f3");
    let (polygon_api, polygon_origin) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(NamedChain::Polygon as u64))
            .with_hardfork(Some(EthereumHardfork::Shanghai.into()))
            .with_genesis_timestamp(Some(1_750_000_000u64)),
    )
    .await;
    polygon_api.anvil_set_code(token, return_zero.clone()).await.unwrap();
    let polygon_url =
        spawn_rpc_proxy_with_blob_header_fields(polygon_origin.http_endpoint(), None, None).await;
    let polygon_url =
        spawn_rpc_proxy_rejecting_method_after(polygon_url, "anvil_nodeInfo", 0).await;

    let (pre_cancun_api, pre_cancun_origin) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(NamedChain::Arbitrum as u64))
            .with_hardfork(Some(EthereumHardfork::Shanghai.into()))
            .with_genesis_timestamp(EthereumHardfork::Shanghai.arbitrum_activation_timestamp()),
    )
    .await;
    pre_cancun_api.anvil_set_code(token, return_zero.clone()).await.unwrap();
    let pre_cancun_url =
        spawn_rpc_proxy_with_blob_header_fields(pre_cancun_origin.http_endpoint(), None, None)
            .await;
    let pre_cancun_url =
        spawn_rpc_proxy_rejecting_method_after(pre_cancun_url, "anvil_nodeInfo", 0).await;

    // A canonical Ethereum endpoint that drops required post-Cancun fields must remain invalid;
    // the Polygon compatibility fallback must not hide that upstream error.
    let (ethereum_api, ethereum_origin) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(NamedChain::Mainnet as u64))
            .with_hardfork(Some(EthereumHardfork::Cancun.into()))
            .with_genesis_timestamp(EthereumHardfork::Cancun.mainnet_activation_timestamp()),
    )
    .await;
    ethereum_api.anvil_set_code(token, return_zero).await.unwrap();
    let ethereum_url =
        spawn_rpc_proxy_with_blob_header_fields(ethereum_origin.http_endpoint(), None, None).await;
    let ethereum_url =
        spawn_rpc_proxy_rejecting_method_after(ethereum_url, "anvil_nodeInfo", 0).await;

    let (api, handle) = spawn(
        NodeConfig::test()
            .with_no_storage_caching(true)
            .with_eth_rpc_url(Some(polygon_url.clone()))
            .with_fork_block_number(Some(0u64)),
    )
    .await;
    let provider = handle.http_provider();
    // This is the WMATIC `balanceOf` call from the Balancer failure report.
    let call = || {
        provider
            .call(
                TransactionRequest {
                    to: Some(TxKind::Call(token)),
                    input: TransactionInput::new(bytes!(
                        "70a08231000000000000000000000000625ac8caddc5dfb99c98176cb6e79d55c7c14e63"
                    )),
                    ..Default::default()
                }
                .into(),
            )
            .block(BlockId::latest())
    };

    assert!(api.backend.spec_id() >= SpecId::CANCUN);
    assert_eq!(call().await.unwrap(), Bytes::from(vec![0; 32]));
    assert_eq!(
        api.backend
            .evm_env()
            .read()
            .block_env
            .blob_excess_gas_and_price
            .as_ref()
            .map(|blob| blob.excess_blob_gas),
        Some(0)
    );

    api.anvil_reset(Some(Forking { json_rpc_url: Some(pre_cancun_url), block_number: Some(0) }))
        .await
        .unwrap();
    assert!(api.backend.spec_id() < SpecId::CANCUN);
    assert!(api.backend.evm_env().read().block_env.blob_excess_gas_and_price.is_none());
    assert_eq!(call().await.unwrap(), Bytes::from(vec![0; 32]));

    api.anvil_reset(Some(Forking { json_rpc_url: Some(ethereum_url), block_number: Some(0) }))
        .await
        .unwrap();
    assert!(api.backend.spec_id() >= SpecId::CANCUN);
    assert!(api.backend.evm_env().read().block_env.blob_excess_gas_and_price.is_none());
    let err = call().await.unwrap_err();
    assert!(err.to_string().contains("Excess blob gas not set"), "{err:?}");

    api.anvil_reset(Some(Forking { json_rpc_url: Some(polygon_url), block_number: Some(0) }))
        .await
        .unwrap();
    assert!(api.backend.spec_id() >= SpecId::CANCUN);
    assert_eq!(call().await.unwrap(), Bytes::from(vec![0; 32]));
    assert_eq!(
        api.backend
            .evm_env()
            .read()
            .block_env
            .blob_excess_gas_and_price
            .as_ref()
            .map(|blob| blob.excess_blob_gas),
        Some(0)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_reset_updates_bpo_blob_schedule_in_both_directions() {
    let excess_blob_gas = 20_000_000u64;
    let bpo1_params = BlobParams::bpo1();
    let bpo2_params = BlobParams::bpo2();
    let origin = |hardfork: EthereumHardfork, params: BlobParams| {
        let mut config = NodeConfig::test()
            .with_chain_id(Some(1u64))
            .with_hardfork(Some(hardfork.into()))
            .with_genesis_timestamp(hardfork.mainnet_activation_timestamp());
        config.blob_excess_gas_and_price =
            Some(BlobExcessGasAndPrice::new(excess_blob_gas, params.update_fraction as u64));
        config
    };
    let (bpo1_api, bpo1_handle) = spawn(origin(EthereumHardfork::Bpo1, bpo1_params)).await;
    let (bpo2_api, bpo2_handle) = spawn(origin(EthereumHardfork::Bpo2, bpo2_params)).await;
    bpo1_api.mine_one().await.unwrap();
    bpo2_api.mine_one().await.unwrap();
    let (api, _) = spawn(
        NodeConfig::test()
            .with_no_storage_caching(true)
            .with_eth_rpc_url(Some(bpo1_handle.http_endpoint()))
            .with_fork_block_number(Some(1u64)),
    )
    .await;

    let assert_blob_schedule = |expected: BlobParams| {
        assert_eq!(api.backend.blob_params(), expected);
        assert_eq!(api.config().unwrap().current.blob_schedule, expected);
        let fees = api.excess_blob_gas_and_price().unwrap().unwrap();
        assert!(fees.excess_blob_gas > 0);
        assert_eq!(fees.blob_gasprice, expected.calc_blob_fee(fees.excess_blob_gas));
    };

    assert_blob_schedule(bpo1_params);
    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(bpo2_handle.http_endpoint()),
        block_number: Some(1),
    }))
    .await
    .unwrap();
    assert_blob_schedule(bpo2_params);
    api.mine_one().await.unwrap();
    assert_blob_schedule(bpo2_params);

    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(bpo1_handle.http_endpoint()),
        block_number: Some(1),
    }))
    .await
    .unwrap();
    assert_blob_schedule(bpo1_params);
    api.mine_one().await.unwrap();
    assert_blob_schedule(bpo1_params);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_reset_restores_selected_blob_schedule_in_both_directions() {
    let excess_blob_gas = 20_000_000u64;
    let configured = |hardfork: EthereumHardfork| {
        NodeConfig::test()
            .with_chain_id(Some(1u64))
            .with_hardfork(Some(hardfork.into()))
            .with_genesis(Some(Genesis {
                timestamp: hardfork.mainnet_activation_timestamp().unwrap(),
                excess_blob_gas: Some(excess_blob_gas),
                ..Default::default()
            }))
    };

    let (cancun_origin_api, cancun_origin) = spawn(configured(EthereumHardfork::Cancun)).await;
    let (bpo2_origin_api, bpo2_origin) = spawn(configured(EthereumHardfork::Bpo2)).await;
    cancun_origin_api.mine_one().await.unwrap();
    bpo2_origin_api.mine_one().await.unwrap();

    let assert_blob_schedule = |api: &EthApi<FoundryNetwork>, expected: BlobParams| {
        assert_eq!(api.backend.blob_params(), expected);
        let fees = api.excess_blob_gas_and_price().unwrap().unwrap();
        assert!(fees.excess_blob_gas > 0);
        assert_eq!(fees.blob_gasprice, expected.calc_blob_fee(fees.excess_blob_gas));
    };

    let (cancun_api, _) = spawn(configured(EthereumHardfork::Cancun)).await;
    assert_blob_schedule(&cancun_api, BlobParams::cancun());
    cancun_api
        .anvil_reset(Some(Forking {
            json_rpc_url: Some(bpo2_origin.http_endpoint()),
            block_number: Some(1),
        }))
        .await
        .unwrap();
    assert_blob_schedule(&cancun_api, BlobParams::bpo2());
    cancun_api.anvil_reset(None).await.unwrap();
    assert_blob_schedule(&cancun_api, BlobParams::cancun());

    let (bpo2_api, _) = spawn(configured(EthereumHardfork::Bpo2)).await;
    assert_blob_schedule(&bpo2_api, BlobParams::bpo2());
    bpo2_api
        .anvil_reset(Some(Forking {
            json_rpc_url: Some(cancun_origin.http_endpoint()),
            block_number: Some(1),
        }))
        .await
        .unwrap();
    assert_blob_schedule(&bpo2_api, BlobParams::cancun());
    bpo2_api.anvil_reset(None).await.unwrap();
    assert_blob_schedule(&bpo2_api, BlobParams::bpo2());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_get_account() {
    let (_api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    let accounts = handle.dev_accounts().collect::<Vec<_>>();

    let alice = accounts[0];
    let bob = accounts[1];

    let init_block = provider.get_block_number().await.unwrap();
    let alice_bal = provider.get_balance(alice).await.unwrap();
    let alice_nonce = provider.get_transaction_count(alice).await.unwrap();
    let alice_acc_init = provider.get_account(alice).await.unwrap();

    assert_eq!(alice_acc_init.balance, alice_bal);
    assert_eq!(alice_acc_init.nonce, alice_nonce);

    let tx = TransactionRequest::default().from(alice).to(bob).value(U256::from(142));

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());
    assert_eq!(init_block + 1, receipt.block_number.unwrap());

    let alice_acc = provider.get_account(alice).await.unwrap();

    assert_eq!(
        alice_acc.balance,
        alice_bal
            - (U256::from(142)
                + U256::from(receipt.gas_used as u128 * receipt.effective_gas_price)),
    );
    assert_eq!(alice_acc.nonce, alice_nonce + 1);

    let alice_acc_prev_block = provider.get_account(alice).number(init_block).await.unwrap();

    assert_eq!(alice_acc_init, alice_acc_prev_block);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_get_account_info() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    let info = provider
        .get_account_info(address!("0x19e53a7397bE5AA7908fE9eA991B03710bdC74Fd"))
        // predates fork
        .number(BLOCK_NUMBER - 1)
        .await
        .unwrap();
    assert_eq!(
        info,
        AccountInfo {
            balance: U256::from(14353753764795095694u64),
            nonce: 6689,
            code: Default::default(),
        }
    );

    // Check account info at block number, see https://github.com/foundry-rs/foundry/issues/12072
    let info = provider
        .get_account_info(address!("0x19e53a7397bE5AA7908fE9eA991B03710bdC74Fd"))
        // predates fork
        .number(BLOCK_NUMBER)
        .await
        .unwrap();
    assert_eq!(
        info,
        AccountInfo {
            balance: U256::from(14352720829244098514u64),
            nonce: 6690,
            code: Default::default(),
        }
    );

    // Mine and check account info at new block number, see https://github.com/foundry-rs/foundry/issues/12148
    api.evm_mine(None).await.unwrap();
    let info = provider
        .get_account_info(address!("0x19e53a7397bE5AA7908fE9eA991B03710bdC74Fd"))
        // predates fork
        .number(BLOCK_NUMBER + 1)
        .await
        .unwrap();
    assert_eq!(
        info,
        AccountInfo {
            balance: U256::from(14352720829244098514u64),
            nonce: 6690,
            code: Default::default(),
        }
    );
}

fn assert_hardfork_config(
    config: &EthConfig,
    expected_blob_params: &BlobParams,
    expected_precompiles: &[Address],
    expected_system_contracts: &BTreeMap<SystemContract, Address>,
) {
    assert!(config.next.is_none());
    assert!(config.last.is_none());

    let current = &config.current;

    assert_eq!(current.activation_time, 0);
    assert_eq!(current.chain_id, 31337);
    assert_eq!(current.fork_id, Bytes::from(vec![0, 0, 0, 0]));

    assert_eq!(&current.blob_schedule, expected_blob_params);

    assert_eq!(
        current.precompiles.values().copied().collect::<BTreeSet<_>>(),
        expected_precompiles.iter().copied().collect::<BTreeSet<_>>(),
    );

    assert_eq!(current.system_contracts, *expected_system_contracts);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_config_with_cancun_hardfork() {
    let (api, _handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()))).await;

    let config = api.config().unwrap();

    let expected_blob_params = BlobParams {
        target_blob_count: 3,
        max_blob_count: 6,
        update_fraction: 3338477,
        min_blob_fee: 1,
        max_blobs_per_tx: 6,
        blob_base_cost: 0,
    };

    // <= Cancun precompiles
    let expected_precompiles = [
        address!("0000000000000000000000000000000000000001"),
        address!("0000000000000000000000000000000000000002"),
        address!("0000000000000000000000000000000000000003"),
        address!("0000000000000000000000000000000000000004"),
        address!("0000000000000000000000000000000000000005"),
        address!("0000000000000000000000000000000000000006"),
        address!("0000000000000000000000000000000000000007"),
        address!("0000000000000000000000000000000000000008"),
        address!("0000000000000000000000000000000000000009"),
        address!("000000000000000000000000000000000000000a"),
    ];

    let expected_system_contracts = BTreeMap::from([(
        SystemContract::BeaconRoots,
        address!("000f3df6d732807ef1319fb7b8bb8522d0beac02"),
    )]);

    assert_hardfork_config(
        &config,
        &expected_blob_params,
        &expected_precompiles,
        &expected_system_contracts,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_config_with_prague_hardfork_with_celo() {
    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_hardfork(Some(EthereumHardfork::Prague.into()))
            .with_networks(NetworkConfigs::with_celo()),
    )
    .await;

    let config = api.config().unwrap();

    let expected_blob_params = BlobParams {
        target_blob_count: 6,
        max_blob_count: 9,
        update_fraction: 5007716,
        min_blob_fee: 1,
        max_blobs_per_tx: 9,
        blob_base_cost: 0,
    };

    // <= Prague + Celo precompiles
    let expected_precompiles = [
        address!("0000000000000000000000000000000000000001"),
        address!("0000000000000000000000000000000000000002"),
        address!("0000000000000000000000000000000000000003"),
        address!("0000000000000000000000000000000000000004"),
        address!("0000000000000000000000000000000000000005"),
        address!("0000000000000000000000000000000000000006"),
        address!("0000000000000000000000000000000000000007"),
        address!("0000000000000000000000000000000000000008"),
        address!("0000000000000000000000000000000000000009"),
        address!("000000000000000000000000000000000000000a"),
        address!("000000000000000000000000000000000000000b"),
        address!("000000000000000000000000000000000000000c"),
        address!("000000000000000000000000000000000000000d"),
        address!("000000000000000000000000000000000000000e"),
        address!("000000000000000000000000000000000000000f"),
        address!("0000000000000000000000000000000000000010"),
        address!("0000000000000000000000000000000000000011"),
        address!("00000000000000000000000000000000000000fd"), // `celo transfer`
    ];

    let expected_system_contracts = BTreeMap::from([
        (SystemContract::BeaconRoots, address!("000f3df6d732807ef1319fb7b8bb8522d0beac02")),
        (
            SystemContract::ConsolidationRequestPredeploy,
            address!("0000bbddc7ce488642fb579f8b00f3a590007251"),
        ),
        (SystemContract::DepositContract, address!("00000000219ab540356cbb839cbe05303d7705fa")),
        (SystemContract::HistoryStorage, address!("0000f90827f1c53a10cb7a02335b175320002935")),
        (
            SystemContract::WithdrawalRequestPredeploy,
            address!("00000961ef480eb55e80d19ad83579a64c007002"),
        ),
    ]);

    assert_hardfork_config(
        &config,
        &expected_blob_params,
        &expected_precompiles,
        &expected_system_contracts,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_config_with_osaka_hardfork() {
    let (api, _handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Osaka.into()))).await;

    let config = api.config().unwrap();

    let expected_blob_params = BlobParams {
        target_blob_count: 6,
        max_blob_count: 9,
        update_fraction: 5007716,
        min_blob_fee: 1,
        max_blobs_per_tx: 6,
        blob_base_cost: 8192,
    };

    // <= Osaka precompiles
    let expected_precompiles = [
        address!("0000000000000000000000000000000000000001"),
        address!("0000000000000000000000000000000000000002"),
        address!("0000000000000000000000000000000000000003"),
        address!("0000000000000000000000000000000000000004"),
        address!("0000000000000000000000000000000000000005"),
        address!("0000000000000000000000000000000000000006"),
        address!("0000000000000000000000000000000000000007"),
        address!("0000000000000000000000000000000000000008"),
        address!("0000000000000000000000000000000000000009"),
        address!("000000000000000000000000000000000000000a"),
        address!("000000000000000000000000000000000000000b"),
        address!("000000000000000000000000000000000000000c"),
        address!("000000000000000000000000000000000000000d"),
        address!("000000000000000000000000000000000000000e"),
        address!("000000000000000000000000000000000000000f"),
        address!("0000000000000000000000000000000000000010"),
        address!("0000000000000000000000000000000000000011"),
        address!("0000000000000000000000000000000000000100"),
    ];

    let expected_system_contracts = BTreeMap::from([
        (SystemContract::BeaconRoots, address!("000f3df6d732807ef1319fb7b8bb8522d0beac02")),
        (
            SystemContract::ConsolidationRequestPredeploy,
            address!("0000bbddc7ce488642fb579f8b00f3a590007251"),
        ),
        (SystemContract::DepositContract, address!("00000000219ab540356cbb839cbe05303d7705fa")),
        (SystemContract::HistoryStorage, address!("0000f90827f1c53a10cb7a02335b175320002935")),
        (
            SystemContract::WithdrawalRequestPredeploy,
            address!("00000961ef480eb55e80d19ad83579a64c007002"),
        ),
    ]);

    assert_hardfork_config(
        &config,
        &expected_blob_params,
        &expected_precompiles,
        &expected_system_contracts,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_config_with_osaka_hardfork_with_precompile_factory() {
    #[derive(Debug)]
    struct CustomPrecompileFactory;

    impl PrecompileFactory for CustomPrecompileFactory {
        fn precompiles(&self) -> Vec<(Address, alloy_evm::precompiles::DynPrecompile)> {
            vec![(
                address!("0x0000000000000000000000000000000000000071"),
                alloy_evm::precompiles::DynPrecompile::from(
                    |input: alloy_evm::precompiles::PrecompileInput<'_>| {
                        Ok(revm::precompile::PrecompileOutput {
                            bytes: Bytes::copy_from_slice(input.data),
                            gas_used: 0,
                            gas_refunded: 0,
                            status: PrecompileStatus::Success,
                            state_gas_used: 0,
                            state_gas_spilled: 0,
                            reservoir: input.reservoir,
                        })
                    },
                ),
            )]
        }
    }

    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_hardfork(Some(EthereumHardfork::Osaka.into()))
            .with_precompile_factory(CustomPrecompileFactory),
    )
    .await;

    let config = api.config().unwrap();

    let expected_blob_params = BlobParams {
        target_blob_count: 6,
        max_blob_count: 9,
        update_fraction: 5007716,
        min_blob_fee: 1,
        max_blobs_per_tx: 6,
        blob_base_cost: 8192,
    };

    // <= Osaka precompiles + custom precompile
    let expected_precompiles = [
        address!("0000000000000000000000000000000000000001"),
        address!("0000000000000000000000000000000000000002"),
        address!("0000000000000000000000000000000000000003"),
        address!("0000000000000000000000000000000000000004"),
        address!("0000000000000000000000000000000000000005"),
        address!("0000000000000000000000000000000000000006"),
        address!("0000000000000000000000000000000000000007"),
        address!("0000000000000000000000000000000000000008"),
        address!("0000000000000000000000000000000000000009"),
        address!("000000000000000000000000000000000000000a"),
        address!("000000000000000000000000000000000000000b"),
        address!("000000000000000000000000000000000000000c"),
        address!("000000000000000000000000000000000000000d"),
        address!("000000000000000000000000000000000000000e"),
        address!("000000000000000000000000000000000000000f"),
        address!("0000000000000000000000000000000000000010"),
        address!("0000000000000000000000000000000000000011"),
        address!("0000000000000000000000000000000000000071"), // `custom_echo`
        address!("0000000000000000000000000000000000000100"),
    ];
    let expected_system_contracts = BTreeMap::from([
        (SystemContract::BeaconRoots, address!("000f3df6d732807ef1319fb7b8bb8522d0beac02")),
        (
            SystemContract::ConsolidationRequestPredeploy,
            address!("0000bbddc7ce488642fb579f8b00f3a590007251"),
        ),
        (SystemContract::DepositContract, address!("00000000219ab540356cbb839cbe05303d7705fa")),
        (SystemContract::HistoryStorage, address!("0000f90827f1c53a10cb7a02335b175320002935")),
        (
            SystemContract::WithdrawalRequestPredeploy,
            address!("00000961ef480eb55e80d19ad83579a64c007002"),
        ),
    ]);

    assert_hardfork_config(
        &config,
        &expected_blob_params,
        &expected_precompiles,
        &expected_system_contracts,
    );
}

// Regression tests: verify that `anvil_setRpcUrl` and `anvil_reset` keep
// `ClientForkConfig.fork_urls` in sync so that subsequent resets don't
// silently revert to stale URLs.

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_rpc_url_syncs_fork_config() {
    // Spawn an origin node and fork off it
    let genesis_timestamp = 1_700_000_000u64;
    let (_origin_api, origin_handle) =
        spawn(NodeConfig::test().with_genesis_timestamp(Some(genesis_timestamp))).await;
    let origin_url = origin_handle.http_endpoint();

    let (api, handle) = spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_url.clone()))).await;

    // Verify initial fork URL
    let fork = api.backend.get_fork().unwrap();
    assert_eq!(fork.config.read().fork_urls, vec![origin_url.clone()]);
    let metadata_before = api.anvil_metadata().await.unwrap();

    // Spawn a second origin to use as the new URL
    let (_origin2_api, origin2_handle) =
        spawn(NodeConfig::test().with_genesis_timestamp(Some(genesis_timestamp))).await;
    let new_url = origin2_handle.http_endpoint();

    // Set RPC URL through the RPC dispatcher to cover its lifecycle-lock path.
    tokio::time::timeout(
        Duration::from_secs(10),
        handle.http_provider().raw_request::<_, ()>("anvil_setRpcUrl".into(), (new_url.clone(),)),
    )
    .await
    .expect("anvil_setRpcUrl deadlocked")
    .unwrap();

    // Verify ClientForkConfig is updated
    let fork = api.backend.get_fork().unwrap();
    assert_eq!(
        fork.config.read().fork_urls,
        vec![new_url.clone()],
        "ClientForkConfig.fork_urls should be updated after anvil_setRpcUrl"
    );
    assert_eq!(api.anvil_metadata().await.unwrap(), metadata_before);
    api.anvil_reset(Some(Forking::default())).await.unwrap();
    assert_eq!(
        api.backend.get_fork().unwrap().config.read().fork_urls,
        vec![new_url],
        "URL-less reset should keep the URL selected by anvil_setRpcUrl"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_reset_rejects_self_reference_without_mutation() {
    let marker = Address::random();
    let marker_balance = U256::from(91_337u64);
    let (origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    origin_api.anvil_set_balance(marker, marker_balance).await.unwrap();
    let origin_url = origin_handle.http_endpoint();
    let (api, handle) = spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_url.clone()))).await;
    let provider = handle.http_provider();
    let metadata_before = api.anvil_metadata().await.unwrap();
    let info_before = api.anvil_node_info().await.unwrap();

    for self_url in [handle.http_endpoint(), handle.ws_endpoint()] {
        let err = api
            .anvil_reset(Some(Forking { json_rpc_url: Some(self_url), block_number: None }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("own RPC endpoint"), "{err}");

        let fork = api.backend.get_fork().unwrap();
        assert_eq!(fork.config.read().fork_urls, vec![origin_url.clone()]);
        assert_eq!(api.anvil_metadata().await.unwrap(), metadata_before);
        assert_eq!(api.anvil_node_info().await.unwrap(), info_before);
        assert_eq!(provider.get_balance(marker).await.unwrap(), marker_balance);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_rpc_url_rejects_self_reference_without_mutation() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let origin_url = origin_handle.http_endpoint();
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_url.clone()))
            .with_fork_block_number(Some(0u64)),
    )
    .await;
    let fork = api.backend.get_fork().unwrap();
    let config_before = fork.config.read().clone();

    let error = api.anvil_set_rpc_url(handle.http_endpoint()).await.unwrap_err();

    assert!(error.to_string().contains("own RPC endpoint"), "{error}");
    {
        let config = fork.config.read();
        assert!(Arc::ptr_eq(&config.provider, &config_before.provider));
        assert_eq!(config.fork_urls, config_before.fork_urls);
    }
    api.anvil_reset(Some(Forking::default())).await.unwrap();
    assert_eq!(api.backend.get_fork().unwrap().eth_rpc_url().as_deref(), Some(origin_url.as_str()));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_rpc_url_rejects_zksync_source_atomically() {
    let (_origin_api, origin_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::Mainnet as u64))).await;
    let origin_url = origin_handle.http_endpoint();
    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_url.clone()))
            .with_fork_block_number(Some(0u64))
            .with_fork_chain_id(Some(U256::from(NamedChain::Mainnet as u64))),
    )
    .await;
    let fork = api.backend.get_fork().unwrap();
    let config_before = fork.config.read().clone();

    let (_zksync_api, zksync_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::ZkSync as u64))).await;
    let error = api.anvil_set_rpc_url(zksync_handle.http_endpoint()).await.unwrap_err();

    assert!(error.to_string().contains("cannot execute native EraVM bytecode"));
    {
        let config = fork.config.read();
        assert_eq!(config.eth_rpc_url(), Some(origin_url.as_str()));
        assert!(Arc::ptr_eq(&config.provider, &config_before.provider));
        assert_eq!(config.fork_chain_id, config_before.fork_chain_id);
        assert_eq!(config.block_hash, config_before.block_hash);
        assert_eq!(config.chain_id, config_before.chain_id);
        assert_eq!(config.execution_chain_id, config_before.execution_chain_id);
    }

    api.anvil_reset(Some(Forking::default())).await.unwrap();
    assert_eq!(api.backend.get_fork().unwrap().eth_rpc_url().as_deref(), Some(origin_url.as_str()));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_rpc_url_rejects_different_chain_atomically() {
    let (_origin_api, origin_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::Mainnet as u64))).await;
    let origin_url = origin_handle.http_endpoint();
    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_url.clone()))
            .with_fork_block_number(Some(0u64)),
    )
    .await;
    let fork = api.backend.get_fork().unwrap();
    let config_before = fork.config.read().clone();
    let (_target_api, target_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::Sepolia as u64))).await;

    api.anvil_set_rpc_url(target_handle.http_endpoint()).await.unwrap_err();

    {
        let config = fork.config.read();
        assert!(Arc::ptr_eq(&config.provider, &config_before.provider));
        assert_eq!(config.fork_urls, config_before.fork_urls);
    }
    api.anvil_reset(Some(Forking::default())).await.unwrap();
    assert_eq!(api.backend.get_fork().unwrap().eth_rpc_url().as_deref(), Some(origin_url.as_str()));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_rpc_url_rejects_mismatched_pinned_block_atomically() {
    let (origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    origin_api.mine_one().await.unwrap();
    let origin_url = origin_handle.http_endpoint();
    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_url.clone()))
            .with_fork_block_number(Some(1u64)),
    )
    .await;
    let fork = api.backend.get_fork().unwrap();
    let config_before = fork.config.read().clone();
    let (target_api, target_handle) = spawn(NodeConfig::test()).await;
    target_api.anvil_set_balance(Address::random(), U256::from(1)).await.unwrap();
    target_api.mine_one().await.unwrap();

    api.anvil_set_rpc_url(target_handle.http_endpoint()).await.unwrap_err();

    {
        let config = fork.config.read();
        assert!(Arc::ptr_eq(&config.provider, &config_before.provider));
        assert_eq!(config.fork_urls, config_before.fork_urls);
    }
    api.anvil_reset(Some(Forking::default())).await.unwrap();
    assert_eq!(api.backend.get_fork().unwrap().eth_rpc_url().as_deref(), Some(origin_url.as_str()));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_rpc_url_keeps_node_info_strict_without_mutation() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let origin_url = origin_handle.http_endpoint();
    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_url.clone()))
            .with_fork_block_number(Some(0u64)),
    )
    .await;
    let fork = api.backend.get_fork().unwrap();
    let config_before = fork.config.read().clone();
    let (_target_api, target_handle) = spawn(NodeConfig::test()).await;
    let target_url =
        spawn_rpc_proxy_rejecting_method_after(target_handle.http_endpoint(), "anvil_nodeInfo", 1)
            .await;

    let error = api.anvil_set_rpc_url(target_url).await.unwrap_err();

    assert!(error.to_string().contains("failed to determine network family"), "{error}");
    {
        let config = fork.config.read();
        assert!(Arc::ptr_eq(&config.provider, &config_before.provider));
        assert_eq!(config.fork_urls, config_before.fork_urls);
    }
    api.anvil_reset(Some(Forking::default())).await.unwrap();
    assert_eq!(api.backend.get_fork().unwrap().eth_rpc_url().as_deref(), Some(origin_url.as_str()));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_reset_with_url_updates_fork_urls() {
    // Spawn an origin node and fork off it
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let origin_url = origin_handle.http_endpoint();

    let (api, _handle) = spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_url.clone()))).await;

    // Spawn a second origin
    let (_origin2_api, origin2_handle) = spawn(NodeConfig::test()).await;
    let new_url = origin2_handle.http_endpoint();

    // Reset fork with a new URL
    api.anvil_reset(Some(Forking { json_rpc_url: Some(new_url.clone()), block_number: None }))
        .await
        .unwrap();

    // Verify the fork config uses the new URL, not the old one
    let fork = api.backend.get_fork().unwrap();
    assert_eq!(
        fork.config.read().fork_urls,
        vec![new_url.clone()],
        "ClientForkConfig.fork_urls should reflect the new URL after anvil_reset"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_reset_updates_inferred_chain_id() {
    let (_mainnet_api, mainnet_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::Mainnet as u64))).await;
    let (api, handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(mainnet_handle.http_endpoint()))).await;
    assert_eq!(handle.http_provider().get_chain_id().await.unwrap(), NamedChain::Mainnet as u64);

    let (_sepolia_api, sepolia_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::Sepolia as u64))).await;
    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(sepolia_handle.http_endpoint()),
        block_number: None,
    }))
    .await
    .unwrap();

    assert_eq!(handle.http_provider().get_chain_id().await.unwrap(), NamedChain::Sepolia as u64);
    let fork = api.backend.get_fork().unwrap();
    assert_eq!(fork.chain_id(), NamedChain::Sepolia as u64);
    assert_eq!(fork.config.read().fork_chain_id, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_reset_without_url_preserves_offline_fork_chain_id() {
    let (_mainnet_api, mainnet_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::Mainnet as u64))).await;
    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(mainnet_handle.http_endpoint()))
            .with_fork_block_number(Some(0u64))
            .with_fork_chain_id(Some(U256::from(NamedChain::Mainnet as u64))),
    )
    .await;

    api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(0) })).await.unwrap();

    let expected = Some(NamedChain::Mainnet as u64);
    let fork = api.backend.get_fork().unwrap();
    assert_eq!(fork.config.read().fork_chain_id, expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_reset_replaces_offline_fork_chain_id() {
    let (_mainnet_api, mainnet_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::Mainnet as u64))).await;
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(mainnet_handle.http_endpoint()))
            .with_fork_block_number(Some(0u64))
            .with_fork_chain_id(Some(U256::from(NamedChain::Mainnet as u64))),
    )
    .await;

    let (_sepolia_api, sepolia_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::Sepolia as u64))).await;
    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(sepolia_handle.http_endpoint()),
        block_number: None,
    }))
    .await
    .unwrap();

    assert_eq!(handle.http_provider().get_chain_id().await.unwrap(), NamedChain::Sepolia as u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_reset_rejects_zksync_source_atomically() {
    let (_origin_api, origin_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::Mainnet as u64))).await;
    let origin_url = origin_handle.http_endpoint();
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(origin_url.clone()))
            .with_fork_block_number(Some(0u64))
            .with_fork_chain_id(Some(U256::from(NamedChain::Mainnet as u64))),
    )
    .await;
    let original_block = handle.http_provider().get_block_number().await.unwrap();
    let original_instance_id = api.instance_id();

    let (_zksync_api, zksync_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::ZkSync as u64))).await;
    let error = api
        .anvil_reset(Some(Forking {
            json_rpc_url: Some(zksync_handle.http_endpoint()),
            block_number: None,
        }))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("cannot execute native EraVM bytecode"));
    let fork = api.backend.get_fork().unwrap();
    assert_eq!(fork.eth_rpc_url().as_deref(), Some(origin_url.as_str()));
    assert_eq!(fork.config.read().fork_chain_id, Some(NamedChain::Mainnet as u64));
    assert_eq!(handle.http_provider().get_block_number().await.unwrap(), original_block);
    assert_eq!(api.instance_id(), original_instance_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_reset_from_local_node_rejects_zksync_source_atomically() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    api.mine_one().await.unwrap();
    let original_block = handle.http_provider().get_block_number().await.unwrap();
    let original_chain_id = handle.http_provider().get_chain_id().await.unwrap();
    let original_instance_id = api.instance_id();

    let (_zksync_api, zksync_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::ZkSync as u64))).await;
    let error = api
        .anvil_reset(Some(Forking {
            json_rpc_url: Some(zksync_handle.http_endpoint()),
            block_number: None,
        }))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("cannot execute native EraVM bytecode"));
    assert!(!api.is_fork());
    assert_eq!(handle.http_provider().get_block_number().await.unwrap(), original_block);
    assert_eq!(handle.http_provider().get_chain_id().await.unwrap(), original_chain_id);
    assert_eq!(api.instance_id(), original_instance_id);
}
