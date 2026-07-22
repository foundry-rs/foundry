//! Gas related tests

use crate::utils::http_provider_with_signer;
use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{Address, U64, U256, uint};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, BlockNumberOrTag, TransactionRequest};
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, eth::fees::INITIAL_BASE_FEE, spawn};

const GAS_TRANSFER: u64 = 21_000;

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_limit_applied_from_config() {
    let (api, _handle) = spawn(NodeConfig::test().with_gas_limit(Some(10_000_000))).await;

    assert_eq!(api.gas_limit(), uint!(10_000_000_U256));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_limit_disabled_from_config() {
    let (api, _handle) = spawn(NodeConfig::test().disable_block_gas_limit(true)).await;

    // see https://github.com/foundry-rs/foundry/pull/8933
    assert_eq!(api.gas_limit(), U256::from(U64::MAX));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_basefee_full_block() {
    let (_api, handle) = spawn(
        NodeConfig::test().with_base_fee(Some(INITIAL_BASE_FEE)).with_gas_limit(Some(GAS_TRANSFER)),
    )
    .await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumWallet = wallet.clone().into();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let tx = TransactionRequest::default().to(Address::random()).with_value(U256::from(1337));
    let tx = WithOtherFields::new(tx);

    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let base_fee = provider
        .get_block(BlockId::latest())
        .await
        .unwrap()
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let next_base_fee = provider
        .get_block(BlockId::latest())
        .await
        .unwrap()
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    assert!(next_base_fee > base_fee);

    // max increase, full block
    assert_eq!(next_base_fee, INITIAL_BASE_FEE + 125_000_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_basefee_half_block() {
    let (_api, handle) = spawn(
        NodeConfig::test()
            .with_base_fee(Some(INITIAL_BASE_FEE))
            .with_gas_limit(Some(GAS_TRANSFER * 2)),
    )
    .await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumWallet = wallet.clone().into();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let tx = TransactionRequest::default().to(Address::random()).with_value(U256::from(1337));
    let tx = WithOtherFields::new(tx);

    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let tx = TransactionRequest::default().to(Address::random()).with_value(U256::from(1337));
    let tx = WithOtherFields::new(tx);

    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let next_base_fee = provider
        .get_block(BlockId::latest())
        .await
        .unwrap()
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    // unchanged, half block
    assert_eq!(next_base_fee, { INITIAL_BASE_FEE });
}

#[tokio::test(flavor = "multi_thread")]
async fn test_basefee_empty_block() {
    let (api, handle) = spawn(NodeConfig::test().with_base_fee(Some(INITIAL_BASE_FEE))).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumWallet = wallet.clone().into();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let tx = TransactionRequest::default().with_to(Address::random()).with_value(U256::from(1337));
    let tx = WithOtherFields::new(tx);

    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let base_fee = provider
        .get_block(BlockId::latest())
        .await
        .unwrap()
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    // mine empty block
    api.mine_one().await;

    let next_base_fee = provider
        .get_block(BlockId::latest())
        .await
        .unwrap()
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    // empty block, decreased base fee
    assert!(next_base_fee < base_fee);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_respect_base_fee() {
    let base_fee = 50u128;
    let (_api, handle) = spawn(NodeConfig::test().with_base_fee(Some(base_fee as u64))).await;

    let provider = handle.http_provider();

    let tx = TransactionRequest::default().with_to(Address::random()).with_value(U256::from(100));
    let mut tx = WithOtherFields::new(tx);

    let mut underpriced = tx.clone();
    underpriced.set_gas_price(base_fee - 1);

    let res = provider.send_transaction(underpriced).await;
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("max fee per gas less than block base fee"));

    tx.set_gas_price(base_fee);
    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tip_above_fee_cap() {
    let base_fee = 50u128;
    let (_api, handle) = spawn(NodeConfig::test().with_base_fee(Some(base_fee as u64))).await;

    let provider = handle.http_provider();

    let tx = TransactionRequest::default()
        .max_fee_per_gas(base_fee)
        .max_priority_fee_per_gas(base_fee + 1)
        .with_to(Address::random())
        .with_value(U256::from(100));
    let tx = WithOtherFields::new(tx);

    let res = provider.send_transaction(tx.clone()).await;
    assert!(res.is_err());
    assert!(
        res.unwrap_err()
            .to_string()
            .contains("max priority fee per gas higher than max fee per gas")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_can_use_fee_history() {
    let base_fee = 50u128;
    let (_api, handle) = spawn(NodeConfig::test().with_base_fee(Some(base_fee as u64))).await;
    let provider = handle.http_provider();

    for _ in 0..10 {
        let fee_history = provider.get_fee_history(1, Default::default(), &[]).await.unwrap();
        let next_base_fee = *fee_history.base_fee_per_gas.last().unwrap();

        let tx = TransactionRequest::default()
            .with_to(Address::random())
            .with_value(U256::from(100))
            .with_gas_price(next_base_fee);
        let tx = WithOtherFields::new(tx);

        let receipt =
            provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();
        assert!(receipt.inner.inner.is_success());

        let fee_history_after = provider.get_fee_history(1, Default::default(), &[]).await.unwrap();
        let latest_fee_history_fee = *fee_history_after.base_fee_per_gas.first().unwrap() as u64;
        let latest_block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();

        assert_eq!(latest_block.header.base_fee_per_gas.unwrap(), latest_fee_history_fee);
        assert_eq!(latest_fee_history_fee, next_base_fee as u64);
    }
}

// Regression test for <https://github.com/foundry-rs/foundry/issues/13680>.
//
// `eth_feeHistory` reads from a cache that the `FeeHistoryService` populates asynchronously, so it
// can lag the chain head. Burst-mining many blocks makes the cache trail behind; the response must
// still cover the full requested range instead of silently dropping the not-yet-cached blocks.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_complete_when_cache_lags() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    // Mine a burst of blocks. The cache is updated by a separate task and will not have caught up
    // by the time the RPC call below runs.
    api.anvil_mine(Some(U256::from(100)), None).await.unwrap();

    let count = 10u64;
    let fee_history =
        api.fee_history(U256::from(count), BlockNumberOrTag::Latest, vec![]).await.unwrap();

    // Per-block arrays must have exactly `count` entries; `base_fee_per_gas` additionally includes
    // the next block, so it has `count + 1`.
    assert_eq!(fee_history.gas_used_ratio.len(), count as usize, "incomplete gas_used_ratio");
    assert_eq!(
        fee_history.base_fee_per_gas.len(),
        count as usize + 1,
        "incomplete base_fee_per_gas"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_gas_empty_data() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let accounts = handle.dev_accounts().collect::<Vec<_>>();
    let from = accounts[0];
    let to = accounts[1];

    let tx_without_data =
        TransactionRequest::default().with_from(from).with_to(to).with_value(U256::from(1));

    let gas_without_data = api
        .estimate_gas(WithOtherFields::new(tx_without_data), None, Default::default())
        .await
        .unwrap();

    let tx_with_empty_data = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_value(U256::from(1))
        .with_input(vec![]);

    let gas_with_empty_data = api
        .estimate_gas(WithOtherFields::new(tx_with_empty_data), None, Default::default())
        .await
        .unwrap();

    let tx_with_data = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_value(U256::from(1))
        .with_input(vec![0x12, 0x34]);

    let gas_with_data = api
        .estimate_gas(WithOtherFields::new(tx_with_data), None, Default::default())
        .await
        .unwrap();

    assert_eq!(gas_without_data, U256::from(GAS_TRANSFER));
    assert_eq!(gas_with_empty_data, U256::from(GAS_TRANSFER));
    assert!(gas_with_data > U256::from(GAS_TRANSFER));
    assert_eq!(gas_without_data, gas_with_empty_data);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_gas_simple_transfer_checks_funds() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let to = handle.dev_accounts().next().unwrap();
    let from = Address::random();

    let tx = TransactionRequest::default().with_from(from).with_to(to).with_value(U256::from(1));
    let err =
        api.estimate_gas(WithOtherFields::new(tx), None, Default::default()).await.unwrap_err();

    assert!(err.to_string().contains("Insufficient funds for gas * price + value"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_gas_simple_transfer_without_from_uses_transfer_fast_path() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let to = handle.dev_accounts().next().unwrap();

    let tx = TransactionRequest::default().with_to(to).with_value(U256::from(1));
    let gas = api.estimate_gas(WithOtherFields::new(tx), None, Default::default()).await.unwrap();

    assert_eq!(gas, U256::from(GAS_TRANSFER));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_gas_without_from_with_gas_price_uses_transfer_fast_path() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let to = handle.dev_accounts().next().unwrap();

    let tx = TransactionRequest::default().with_to(to).with_gas_price(INITIAL_BASE_FEE as u128);
    let gas = api.estimate_gas(WithOtherFields::new(tx), None, Default::default()).await.unwrap();

    assert_eq!(gas, U256::from(GAS_TRANSFER));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_gas_fee_token_does_not_skip_funds_check_outside_tempo() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let to = handle.dev_accounts().next().unwrap();
    let from = Address::random();

    let tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default().with_from(from).with_to(to).with_value(U256::from(1)),
        other: [(
            "feeToken".to_string(),
            serde_json::json!("0x20c0000000000000000000000000000000000001"),
        )]
        .into_iter()
        .collect(),
    };
    let err = api.estimate_gas(tx, None, Default::default()).await.unwrap_err();

    assert!(err.to_string().contains("Insufficient funds for gas * price + value"));
}
