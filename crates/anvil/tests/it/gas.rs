//! Gas related tests

use crate::utils::http_provider;
use alloy_network::{EthereumSigner, TransactionBuilder};
use alloy_primitives::{Address, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, TransactionRequest, WithOtherFields};
use anvil::{eth::fees::INITIAL_BASE_FEE, spawn, NodeConfig};

const GAS_TRANSFER: u128 = 21_000;

#[tokio::test(flavor = "multi_thread")]
async fn test_basefee_full_block() {
    let (_api, handle) = spawn(
        NodeConfig::test().with_base_fee(Some(INITIAL_BASE_FEE)).with_gas_limit(Some(GAS_TRANSFER)),
    )
    .await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();

    let provider = http_provider(&handle.http_endpoint());
    let provider_with_signer = http_provider_with_signer(&handle.http_endpoint(), signer);

    let tx = TransactionRequest::default()
        .with_to(Address::random().into())
        .with_value(U256::from(1337));
    let tx = WithOtherFields::new(tx);

    provider_with_signer.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let base_fee = provider
        .get_block(BlockId::latest(), false)
        .await
        .unwrap()
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    provider_with_signer.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let next_base_fee = provider
        .get_block(BlockId::latest(), false)
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
    let signer: EthereumSigner = wallet.clone().into();

    let provider = http_provider(&handle.http_endpoint());
    let provider_with_signer = http_provider_with_signer(&handle.http_endpoint(), signer);

    let tx = TransactionRequest::default()
        .with_to(Address::random().into())
        .with_value(U256::from(1337));
    let tx = WithOtherFields::new(tx);

    provider_with_signer.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let tx = TransactionRequest::default()
        .with_to(Address::random().into())
        .with_value(U256::from(1337));
    let tx = WithOtherFields::new(tx);

    provider_with_signer.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let next_base_fee = provider
        .get_block(BlockId::latest(), false)
        .await
        .unwrap()
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    // unchanged, half block
    assert_eq!(next_base_fee, INITIAL_BASE_FEE);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_basefee_empty_block() {
    let (api, handle) = spawn(NodeConfig::test().with_base_fee(Some(INITIAL_BASE_FEE))).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();

    let provider = http_provider(&handle.http_endpoint());
    let provider_with_signer = http_provider_with_signer(&handle.http_endpoint(), signer);

    let tx = TransactionRequest::default()
        .with_to(Address::random().into())
        .with_value(U256::from(1337));
    let tx = WithOtherFields::new(tx);

    provider_with_signer.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let base_fee = provider
        .get_block(BlockId::latest(), false)
        .await
        .unwrap()
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    // mine empty block
    api.mine_one().await;

    let next_base_fee = provider
        .get_block(BlockId::latest(), false)
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
    let (_api, handle) = spawn(NodeConfig::test().with_base_fee(Some(base_fee))).await;

    let provider = http_provider(&handle.http_endpoint());

    let tx =
        TransactionRequest::default().with_to(Address::random().into()).with_value(U256::from(100));
    let mut tx = WithOtherFields::new(tx);

    let mut underpriced = tx.clone();
    underpriced.set_gas_price(base_fee - 1);

    let res = provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await;
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("max fee per gas less than block base fee"));

    tx.set_gas_price(base_fee);
    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tip_above_fee_cap() {
    let base_fee = 50u128;
    let (_api, handle) = spawn(NodeConfig::test().with_base_fee(Some(base_fee))).await;

    let provider = http_provider(&handle.http_endpoint());

    let tx = TransactionRequest::default()
        .max_fee_per_gas(base_fee)
        .max_priority_fee_per_gas(base_fee + 1)
        .with_to(Address::random().into())
        .with_value(U256::from(100));
    let tx = WithOtherFields::new(tx);

    let res = provider.send_transaction(tx.clone()).await;
    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .to_string()
        .contains("max priority fee per gas higher than max fee per gas"));
}
