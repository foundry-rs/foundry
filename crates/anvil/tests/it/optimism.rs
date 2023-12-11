//! Tests for OP chain support.

use anvil::{spawn, Hardfork, NodeConfig};
use ethers::{
    abi::Address,
    providers::Middleware,
    types::{
        transaction::{eip2718::TypedTransaction, optimism::DepositTransaction},
        TransactionRequest, U256,
    },
};
use ethers_core::types::{Bytes, H256};
use foundry_common::types::ToAlloy;
use std::str::FromStr;

#[tokio::test(flavor = "multi_thread")]
async fn test_deposits_not_supported_if_optimism_disabled() {
    // optimism disabled by default
    let (_, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_http_provider();

    let from_addr: Address = "cf7f9e66af820a19257a2108375b180b0ec49167".parse().unwrap();
    let to_addr: Address = "71562b71999873db5b286df957af199ec94617f7".parse().unwrap();
    let deposit_tx: TypedTransaction = TypedTransaction::DepositTransaction(DepositTransaction {
        tx: TransactionRequest {
            chain_id: None,
            from: Some(from_addr),
            to: Some(ethers::types::NameOrAddress::Address(to_addr)),
            value: Some("1234".parse().unwrap()),
            gas: Some(U256::from(21000)),
            gas_price: None,
            data: Some(Bytes::default()),
            nonce: None,
        },
        source_hash: H256::from_str(
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap(),
        mint: Some(U256::zero()),
        is_system_tx: true,
    });

    // sending the deposit transaction should fail with error saying not supported
    let res = provider.send_transaction(deposit_tx.clone(), None).await;
    assert!(res
        .unwrap_err()
        .to_string()
        .contains("op-stack deposit tx received but is not supported"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_send_value_deposit_transaction() {
    // enable the Optimism flag
    let (api, handle) =
        spawn(NodeConfig::test().with_optimism(true).with_hardfork(Some(Hardfork::Paris))).await;
    let provider = handle.ethers_http_provider();

    let send_value = U256::from(1234);
    let from_addr: Address = "cf7f9e66af820a19257a2108375b180b0ec49167".parse().unwrap();
    let to_addr: Address = "71562b71999873db5b286df957af199ec94617f7".parse().unwrap();

    // fund the sender
    api.anvil_set_balance(from_addr.to_alloy(), send_value.to_alloy()).await.unwrap();

    let deposit_tx: TypedTransaction = TypedTransaction::DepositTransaction(DepositTransaction {
        tx: TransactionRequest {
            chain_id: None,
            from: Some(from_addr),
            to: Some(ethers::types::NameOrAddress::Address(to_addr)),
            value: Some(send_value),
            gas: Some(U256::from(21000)),
            gas_price: None,
            data: Some(Bytes::default()),
            nonce: None,
        },
        source_hash: H256::from_str(
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap(),
        mint: Some(U256::zero()),
        is_system_tx: true,
    });

    let pending = provider.send_transaction(deposit_tx.clone(), None).await.unwrap();

    // mine block
    api.evm_mine(None).await.unwrap();

    let receipt = provider.get_transaction_receipt(pending.tx_hash()).await.unwrap().unwrap();
    assert_eq!(receipt.from, from_addr);
    assert_eq!(receipt.to, Some(to_addr));

    // the recipient should have received the value
    let balance = provider.get_balance(to_addr, None).await.unwrap();
    assert_eq!(balance, send_value);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_send_value_raw_deposit_transaction() {
    // enable the Optimism flag
    let (api, handle) =
        spawn(NodeConfig::test().with_optimism(true).with_hardfork(Some(Hardfork::Paris))).await;
    let provider = handle.ethers_http_provider();

    let send_value = U256::from(1234);
    let from_addr: Address = "cf7f9e66af820a19257a2108375b180b0ec49167".parse().unwrap();
    let to_addr: Address = "71562b71999873db5b286df957af199ec94617f7".parse().unwrap();

    // fund the sender
    api.anvil_set_balance(from_addr.to_alloy(), send_value.to_alloy()).await.unwrap();

    let deposit_tx: TypedTransaction = TypedTransaction::DepositTransaction(DepositTransaction {
        tx: TransactionRequest {
            chain_id: None,
            from: Some(from_addr),
            to: Some(ethers::types::NameOrAddress::Address(to_addr)),
            value: Some(send_value),
            gas: Some(U256::from(21000)),
            gas_price: None,
            data: Some(Bytes::default()),
            nonce: None,
        },
        source_hash: H256::from_str(
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap(),
        mint: Some(U256::zero()),
        is_system_tx: true,
    });

    let rlpbytes = deposit_tx.rlp();
    let pending = provider.send_raw_transaction(rlpbytes).await.unwrap();

    // mine block
    api.evm_mine(None).await.unwrap();

    let receipt = provider.get_transaction_receipt(pending.tx_hash()).await.unwrap().unwrap();
    assert_eq!(receipt.from, from_addr);
    assert_eq!(receipt.to, Some(to_addr));

    // the recipient should have received the value
    let balance = provider.get_balance(to_addr, None).await.unwrap();
    assert_eq!(balance, send_value);
}
