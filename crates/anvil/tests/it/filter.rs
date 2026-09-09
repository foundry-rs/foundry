//! Tests for the polling-based `eth_*Filter` RPC surface.
//!
//! Log filter *changes* are covered in [`crate::logs`]; these tests cover filter installation,
//! draining, `eth_getFilterLogs`, uninstallation, and the error paths for unknown ids.

use crate::{abi::SimpleStorage, utils::http_provider_with_signer};
use alloy_network::EthereumWallet;
use alloy_primitives::B256;
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, Filter, Log};
use anvil::{NodeConfig, spawn};

/// An id that was never handed out by `eth_new*Filter`.
const UNKNOWN_FILTER_ID: &str = "0xdeadbeefdeadbeefdeadbeefdeadbeef";

#[tokio::test(flavor = "multi_thread")]
async fn block_filter_drains_new_block_hashes() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let filter_id: String = provider.client().request_noparams("eth_newBlockFilter").await.unwrap();

    // A filter installed on an idle node has nothing to report yet.
    let changes: Vec<B256> =
        provider.client().request("eth_getFilterChanges", (filter_id.clone(),)).await.unwrap();
    assert!(changes.is_empty(), "expected no blocks before mining, got {changes:?}");

    api.mine_one().await.unwrap();
    api.mine_one().await.unwrap();

    let changes: Vec<B256> =
        provider.client().request("eth_getFilterChanges", (filter_id.clone(),)).await.unwrap();
    let mut expected = Vec::new();
    for number in [1u64, 2] {
        let block =
            provider.get_block_by_number(BlockNumberOrTag::Number(number)).await.unwrap().unwrap();
        expected.push(block.header.hash);
    }
    assert_eq!(changes, expected);

    // Polling drains the filter: the same blocks are not reported twice.
    let changes: Vec<B256> =
        provider.client().request("eth_getFilterChanges", (filter_id,)).await.unwrap();
    assert!(changes.is_empty(), "expected drained filter, got {changes:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn new_filter_seeds_historic_logs_only_with_from_block() {
    let (_api, handle) = spawn(NodeConfig::test()).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumWallet = wallet.into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    // The constructor emits a `ValueChanged` log before either filter is installed.
    let contract =
        SimpleStorage::deploy(provider.clone(), "initial value".to_string()).await.unwrap();

    // A filter with a past `fromBlock` replays the logs that predate it.
    let historic =
        Filter::new().address(*contract.address()).from_block(BlockNumberOrTag::Earliest);
    let historic_id: String =
        provider.client().request("eth_newFilter", (historic,)).await.unwrap();

    // A filter without a block range only reports logs emitted from now on.
    let live = Filter::new().address(*contract.address());
    let live_id: String = provider.client().request("eth_newFilter", (live,)).await.unwrap();

    let changes: Vec<Log> =
        provider.client().request("eth_getFilterChanges", (historic_id,)).await.unwrap();
    assert_eq!(changes.len(), 1, "expected the constructor log to be replayed");

    let changes: Vec<Log> =
        provider.client().request("eth_getFilterChanges", (live_id,)).await.unwrap();
    assert!(changes.is_empty(), "expected no historic logs, got {changes:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_filter_logs_ignores_poll_position() {
    let (_api, handle) = spawn(NodeConfig::test()).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let account = wallet.address();
    let signer: EthereumWallet = wallet.into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let contract =
        SimpleStorage::deploy(provider.clone(), "initial value".to_string()).await.unwrap();

    let filter = Filter::new().address(*contract.address()).from_block(BlockNumberOrTag::Earliest);
    let filter_id: String = provider.client().request("eth_newFilter", (filter,)).await.unwrap();

    contract
        .setValue("hi".to_string())
        .from(account)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    // Drain the filter so that a subsequent `eth_getFilterChanges` would report nothing.
    let changes: Vec<Log> =
        provider.client().request("eth_getFilterChanges", (filter_id.clone(),)).await.unwrap();
    assert_eq!(changes.len(), 2, "expected the constructor and `setValue` logs");

    let changes: Vec<Log> =
        provider.client().request("eth_getFilterChanges", (filter_id.clone(),)).await.unwrap();
    assert!(changes.is_empty(), "expected drained filter, got {changes:?}");

    // `eth_getFilterLogs` re-evaluates the filter instead of continuing from the poll position.
    let logs: Vec<Log> =
        provider.client().request("eth_getFilterLogs", (filter_id,)).await.unwrap();
    assert_eq!(logs.len(), 2, "expected all matching logs regardless of prior polling");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_filter_logs_rejects_non_log_filters() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let block_filter: String =
        provider.client().request_noparams("eth_newBlockFilter").await.unwrap();
    let err = provider
        .client()
        .request::<_, Vec<Log>>("eth_getFilterLogs", (block_filter,))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("filter not found"), "unexpected error: {err}");

    let pending_filter: String =
        provider.client().request("eth_newPendingTransactionFilter", (false,)).await.unwrap();
    let err = provider
        .client()
        .request::<_, Vec<Log>>("eth_getFilterLogs", (pending_filter,))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("filter not found"), "unexpected error: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn uninstall_filter_reports_whether_a_filter_was_removed() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let filter_id: String = provider.client().request_noparams("eth_newBlockFilter").await.unwrap();

    let removed: bool =
        provider.client().request("eth_uninstallFilter", (filter_id.clone(),)).await.unwrap();
    assert!(removed);

    // Uninstalling twice is not an error, but reports that nothing was removed.
    let removed: bool =
        provider.client().request("eth_uninstallFilter", (filter_id.clone(),)).await.unwrap();
    assert!(!removed);

    let removed: bool =
        provider.client().request("eth_uninstallFilter", (UNKNOWN_FILTER_ID,)).await.unwrap();
    assert!(!removed);

    // An uninstalled filter can no longer be polled.
    let err = provider
        .client()
        .request::<_, Vec<B256>>("eth_getFilterChanges", (filter_id,))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("filter not found"), "unexpected error: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_filter_changes_rejects_unknown_ids() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let err = provider
        .client()
        .request::<_, Vec<B256>>("eth_getFilterChanges", (UNKNOWN_FILTER_ID,))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("filter not found"), "unexpected error: {err}");
}
