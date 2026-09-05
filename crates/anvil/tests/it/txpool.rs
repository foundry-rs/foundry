//! txpool related tests

use alloy_consensus::Transaction;
use alloy_network::{AnyRpcTransaction, ReceiptResponse, TransactionBuilder, TransactionResponse};
use alloy_primitives::{TxHash, U256};
use alloy_provider::{Provider, ext::TxPoolApi};
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, spawn};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test(flavor = "multi_thread")]
async fn geth_txpool() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    api.anvil_set_auto_mine(false).await.unwrap();

    let account = provider.get_accounts().await.unwrap().remove(0);
    let value = U256::from(42);
    let gas_price = 221435145689u128;

    let tx = TransactionRequest::default()
        .with_to(account)
        .with_from(account)
        .with_value(value)
        .with_gas_price(gas_price);
    let tx = WithOtherFields::new(tx);

    // send a few transactions
    for _ in 0..10 {
        let _ = provider.send_transaction(tx.clone()).await.unwrap();
    }

    // we gave a 20s block time, should be plenty for us to get the txpool's content
    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.pending, 10);
    assert_eq!(status.queued, 0);

    let inspect = provider.txpool_inspect().await.unwrap();
    assert!(inspect.queued.is_empty());
    let summary = inspect.pending.get(&account).unwrap();
    for i in 0..10 {
        let tx_summary = summary.get(&i.to_string()).unwrap();
        assert_eq!(tx_summary.gas_price, gas_price);
        assert_eq!(tx_summary.value, value);
        assert_eq!(tx_summary.gas, 21000);
        assert_eq!(tx_summary.to.unwrap(), account);
    }

    let content = provider.txpool_content().await.unwrap();
    assert!(content.queued.is_empty());
    let content = content.pending.get(&account).unwrap();

    for nonce in 0..10 {
        assert!(content.contains_key(&nonce.to_string()));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn geth_txpool_separates_queued_transactions() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    api.anvil_set_auto_mine(false).await.unwrap();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let account = accounts[0].address();
    let recipient = accounts[1].address();
    let gas_price = 221435145689u128;

    let pending_value = U256::from(42);
    let pending_tx = TransactionRequest::default()
        .with_to(recipient)
        .with_from(account)
        .with_value(pending_value)
        .with_gas_price(gas_price)
        .with_nonce(0);
    let pending_tx = WithOtherFields::new(pending_tx);

    let queued_value = U256::from(84);
    let queued_tx = TransactionRequest::default()
        .with_to(recipient)
        .with_from(account)
        .with_value(queued_value)
        .with_gas_price(gas_price)
        .with_nonce(2);
    let queued_tx = WithOtherFields::new(queued_tx);

    let _ = provider.send_transaction(pending_tx).await.unwrap();
    let _ = provider.send_transaction(queued_tx).await.unwrap();

    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.pending, 1);
    assert_eq!(status.queued, 1);

    let inspect = provider.txpool_inspect().await.unwrap();
    let pending = inspect.pending.get(&account).unwrap();
    assert_eq!(pending.len(), 1);
    assert!(!pending.contains_key("2"));
    let pending_summary = pending.get("0").unwrap();
    assert_eq!(pending_summary.gas_price, gas_price);
    assert_eq!(pending_summary.value, pending_value);
    assert_eq!(pending_summary.gas, 21000);
    assert_eq!(pending_summary.to.unwrap(), recipient);

    let queued = inspect.queued.get(&account).unwrap();
    assert_eq!(queued.len(), 1);
    assert!(!queued.contains_key("0"));
    let queued_summary = queued.get("2").unwrap();
    assert_eq!(queued_summary.gas_price, gas_price);
    assert_eq!(queued_summary.value, queued_value);
    assert_eq!(queued_summary.gas, 21000);
    assert_eq!(queued_summary.to.unwrap(), recipient);

    let content = provider.txpool_content().await.unwrap();
    let pending = content.pending.get(&account).unwrap();
    assert_eq!(pending.len(), 1);
    assert!(pending.contains_key("0"));
    assert!(!pending.contains_key("2"));

    let queued = content.queued.get(&account).unwrap();
    assert_eq!(queued.len(), 1);
    assert!(queued.contains_key("2"));
    assert!(!queued.contains_key("0"));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_debug_clear_txpool() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    api.anvil_set_auto_mine(false).await.unwrap();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let account = accounts[0].address();
    let recipient = accounts[1].address();
    let gas_price = 221435145689u128;

    let pending_tx = TransactionRequest::default()
        .with_to(recipient)
        .with_from(account)
        .with_value(U256::from(42))
        .with_gas_price(gas_price)
        .with_nonce(0);
    let queued_tx = TransactionRequest::default()
        .with_to(recipient)
        .with_from(account)
        .with_value(U256::from(84))
        .with_gas_price(gas_price)
        .with_nonce(2);

    let _ = provider.send_transaction(WithOtherFields::new(pending_tx)).await.unwrap();
    let _ = provider.send_transaction(WithOtherFields::new(queued_tx)).await.unwrap();

    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.pending, 1);
    assert_eq!(status.queued, 1);

    let _: () = provider.client().request("debug_clearTxpool", ()).await.unwrap();

    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.pending, 0);
    assert_eq!(status.queued, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn geth_txpool_content_from_filters_sender() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    api.anvil_set_auto_mine(false).await.unwrap();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let sender = accounts[0].address();
    let other_sender = accounts[1].address();
    let recipient = accounts[2].address();
    let empty_sender = accounts[3].address();
    let gas_price = 221435145689u128;

    let sender_pending_tx = TransactionRequest::default()
        .with_to(recipient)
        .with_from(sender)
        .with_value(U256::from(42))
        .with_gas_price(gas_price)
        .with_nonce(0);
    let sender_pending_tx = WithOtherFields::new(sender_pending_tx);

    let sender_queued_tx = TransactionRequest::default()
        .with_to(recipient)
        .with_from(sender)
        .with_value(U256::from(84))
        .with_gas_price(gas_price)
        .with_nonce(2);
    let sender_queued_tx = WithOtherFields::new(sender_queued_tx);

    let other_pending_tx = TransactionRequest::default()
        .with_to(recipient)
        .with_from(other_sender)
        .with_value(U256::from(126))
        .with_gas_price(gas_price)
        .with_nonce(0);
    let other_pending_tx = WithOtherFields::new(other_pending_tx);

    let _ = provider.send_transaction(sender_pending_tx).await.unwrap();
    let _ = provider.send_transaction(sender_queued_tx).await.unwrap();
    let _ = provider.send_transaction(other_pending_tx).await.unwrap();

    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.pending, 2);
    assert_eq!(status.queued, 1);

    let content = provider.txpool_content_from(sender).await.unwrap();
    assert_eq!(content.pending.len(), 1);
    assert_eq!(content.queued.len(), 1);

    let pending = content.pending.get("0").unwrap();
    assert_eq!(pending.from(), sender);
    assert!(!content.pending.contains_key("2"));

    let queued = content.queued.get("2").unwrap();
    assert_eq!(queued.from(), sender);
    assert!(!content.queued.contains_key("0"));

    let other_content = provider.txpool_content_from(other_sender).await.unwrap();
    assert_eq!(other_content.pending.len(), 1);
    assert_eq!(other_content.pending.get("0").unwrap().from(), other_sender);
    assert!(other_content.queued.is_empty());

    let empty_content = provider.txpool_content_from(empty_sender).await.unwrap();
    assert!(empty_content.pending.is_empty());
    assert!(empty_content.queued.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_filter_full_pending_transactions() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    api.anvil_set_auto_mine(false).await.unwrap();

    let account = provider.get_accounts().await.unwrap().remove(0);
    let value = U256::from(42);
    let tx = TransactionRequest::default().with_to(account).with_from(account).with_value(value);
    let tx = WithOtherFields::new(tx);

    let hash_filter: String =
        provider.client().request("eth_newPendingTransactionFilter", (false,)).await.unwrap();
    let full_filter: String =
        provider.client().request("eth_newPendingTransactionFilter", (true,)).await.unwrap();

    let pending = provider.send_transaction(tx).await.unwrap();
    let tx_hash = *pending.tx_hash();

    let mut hash_changes = Vec::new();
    for _ in 0..100 {
        let changes: Vec<TxHash> = provider
            .client()
            .request("eth_getFilterChanges", (hash_filter.clone(),))
            .await
            .unwrap();
        if !changes.is_empty() {
            hash_changes = changes;
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }

    let mut full_changes = Vec::new();
    for _ in 0..100 {
        let changes: Vec<AnyRpcTransaction> = provider
            .client()
            .request("eth_getFilterChanges", (full_filter.clone(),))
            .await
            .unwrap();
        if !changes.is_empty() {
            full_changes = changes;
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }

    assert_eq!(hash_changes, vec![tx_hash]);
    assert_eq!(full_changes.len(), 1);

    let full_tx = &full_changes[0];
    assert_eq!(full_tx.inner.tx_hash(), tx_hash);
    assert_eq!(full_tx.inner.value(), value);
}

// Cf. https://github.com/foundry-rs/foundry/issues/11239
#[tokio::test(flavor = "multi_thread")]
async fn accepts_spend_after_funding_when_pool_checks_disabled() {
    // Spawn with pool balance checks disabled
    let (api, handle) = spawn(NodeConfig::test().with_disable_pool_balance_checks(true)).await;
    let provider = handle.http_provider();

    // Work with pending pool (no automine)
    api.anvil_set_auto_mine(false).await.unwrap();

    // Funder is a dev account controlled by the node
    let funder = provider.get_accounts().await.unwrap().remove(0);

    // Recipient/spender is a random address with zero balance that we'll impersonate
    let spender = alloy_primitives::Address::random();
    api.anvil_set_balance(spender, U256::from(0u64)).await.unwrap();
    api.anvil_impersonate_account(spender).await.unwrap();

    // Ensure tx1 (funding) has higher gas price so it's mined before tx2 within the same block
    let gas_price_fund = 2_000_000_000_000u128; // 2_000 gwei
    let gas_price_spend = 1_000_000_000u128; // 1 gwei

    let fund_value = U256::from(1_000_000_000_000_000_000u128); // 1 ether

    // tx1: fund spender from funder
    let tx1 = TransactionRequest::default()
        .with_from(funder)
        .with_to(spender)
        .with_value(fund_value)
        .with_gas_price(gas_price_fund);
    let tx1 = WithOtherFields::new(tx1);

    // tx2: spender attempts to send value greater than their pre-funding balance (0),
    // which would normally be rejected by pool balance checks, but should be accepted when disabled
    let spend_value = fund_value - U256::from(21_000u64) * U256::from(gas_price_spend);
    let tx2 = TransactionRequest::default()
        .with_from(spender)
        .with_to(funder)
        .with_value(spend_value)
        .with_gas_limit(21_000)
        .with_gas_price(gas_price_spend);
    let tx2 = WithOtherFields::new(tx2);

    // Publish both transactions (funding first, then spend-before-funding-is-mined)
    let sent1 = provider.send_transaction(tx1).await.unwrap();
    let sent2 = provider.send_transaction(tx2).await.unwrap();

    // Both should be accepted into the pool (pending)
    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.pending, 2);
    assert_eq!(status.queued, 0);

    // Mine a block and ensure both succeed
    api.evm_mine(None).await.unwrap();

    let receipt1 = sent1.get_receipt().await.unwrap();
    let receipt2 = sent2.get_receipt().await.unwrap();
    assert!(receipt1.status());
    assert!(receipt2.status());
}

/// Replacing a *queued* (future-nonce) transaction must remove the old one, not stack it -
/// otherwise the pool grows unbounded and only one of the stacked entries can ever be mined.
#[tokio::test(flavor = "multi_thread")]
async fn queued_tx_replacement_removes_old_tx() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    _api.anvil_set_auto_mine(false).await.unwrap();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let account = accounts[0].address();
    let recipient = accounts[1].address();
    let gas_price_base = 221435145689u128;

    // account's current nonce is 0, so nonce 5 always lands in the queued/waiting pool, never
    // the ready pool - isolates the `PendingTransactions::add_transaction` path under test.
    let make_tx = |gas_price: u128| {
        WithOtherFields::new(
            TransactionRequest::default()
                .with_to(recipient)
                .with_from(account)
                .with_value(U256::from(1))
                .with_gas_price(gas_price)
                .with_nonce(5),
        )
    };

    let first = provider.send_transaction(make_tx(gas_price_base)).await.unwrap();
    let first_hash = *first.tx_hash();
    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.queued, 1, "first tx should be queued");

    // replace with a higher-priced tx at the same (sender, nonce)
    let second = provider.send_transaction(make_tx(gas_price_base * 2)).await.unwrap();
    let second_hash = *second.tx_hash();

    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.queued, 1, "replacement must remove the old queued tx, not stack it");
    assert!(provider.get_transaction_by_hash(first_hash).await.unwrap().is_none());
    assert!(provider.get_transaction_by_hash(second_hash).await.unwrap().is_some());

    // replace a second time - proves the marker bookkeeping wasn't corrupted by the first
    // removal (the fix trap: removing the old marker entry *after* inserting the new one would
    // delete the new tx's own marker instead, silently disabling this exact check downstream)
    let third = provider.send_transaction(make_tx(gas_price_base * 3)).await.unwrap();
    let third_hash = *third.tx_hash();
    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.queued, 1);
    assert!(provider.get_transaction_by_hash(second_hash).await.unwrap().is_none());
    assert!(provider.get_transaction_by_hash(third_hash).await.unwrap().is_some());

    // an underpriced replacement attempt at the same slot must still be rejected
    let underpriced_err =
        provider.send_transaction(make_tx(gas_price_base * 2 + 1)).await.unwrap_err();
    let msg = format!("{underpriced_err:?}").to_lowercase();
    assert!(msg.contains("underpriced"), "expected underpriced rejection, got: {msg}");

    // still only one queued tx, still the third one
    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.queued, 1);
    assert!(provider.get_transaction_by_hash(third_hash).await.unwrap().is_some());

    // fill nonces 0-4 and mine: this exercises `mark_and_unlock`, which walks
    // `required_markers` for the surviving (third) queued tx. If replacement's marker
    // cleanup were incomplete (e.g. a stale hash left behind in `required_markers`), this
    // would either panic (the `expect` in `mark_and_unlock`) or fail to promote/mine the
    // third tx - not just leave a stray pool-accounting mismatch.
    for nonce in 0..5u64 {
        let filler = WithOtherFields::new(
            TransactionRequest::default()
                .with_to(recipient)
                .with_from(account)
                .with_value(U256::from(1))
                .with_gas_price(gas_price_base)
                .with_nonce(nonce),
        );
        let _ = provider.send_transaction(filler).await.unwrap();
    }
    _api.evm_mine(None).await.unwrap();
    _api.evm_mine(None).await.unwrap();

    let receipt = provider.get_transaction_receipt(third_hash).await.unwrap();
    assert!(
        receipt.is_some_and(|r| r.status()),
        "the surviving replacement tx must mine successfully once its nonce gap is filled"
    );
}

/// `anvil_dropTransaction` must remove a *queued* (future-nonce) transaction too, not just a
/// ready one - the pool's other bulk-remove paths (`remove_invalid`,
/// `remove_transactions_by_address`) already touch both pools; this one didn't.
#[tokio::test(flavor = "multi_thread")]
async fn anvil_drop_transaction_removes_queued_tx() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    api.anvil_set_auto_mine(false).await.unwrap();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let account = accounts[0].address();
    let recipient = accounts[1].address();

    let tx = WithOtherFields::new(
        TransactionRequest::default()
            .with_to(recipient)
            .with_from(account)
            .with_value(U256::from(1))
            .with_gas_price(221435145689u128)
            .with_nonce(5),
    );
    let sent = provider.send_transaction(tx).await.unwrap();
    let hash = *sent.tx_hash();

    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.queued, 1);

    let dropped = api.anvil_drop_transaction(hash).await.unwrap();
    assert_eq!(dropped, Some(hash), "anvil_dropTransaction should report the queued tx as dropped");

    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.queued, 0, "the queued tx must actually be removed from the pool");
    assert!(provider.get_transaction_by_hash(hash).await.unwrap().is_none());
}
