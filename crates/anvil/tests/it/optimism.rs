//! Tests for OP chain support.

use crate::utils::{http_provider, http_provider_with_signer};
use alloy_eips::eip2718::Encodable2718;
use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{b256, Address, TxHash, TxKind, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{optimism::OptimismTransactionFields, TransactionRequest};
use alloy_serde::WithOtherFields;
use anvil::{spawn, Hardfork, NodeConfig};
use anvil_core::eth::transaction::optimism::DepositTransaction;

#[tokio::test(flavor = "multi_thread")]
async fn test_deposits_not_supported_if_optimism_disabled() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_value(U256::from(1234))
        .with_gas_limit(21000);
    let tx = WithOtherFields {
        inner: tx,
        other: OptimismTransactionFields {
            source_hash: Some(b256!(
                "0000000000000000000000000000000000000000000000000000000000000000"
            )),
            mint: Some(0),
            is_system_tx: Some(true),
        }
        .into(),
    };

    let err = provider.send_transaction(tx).await.unwrap_err();
    let s = err.to_string();
    assert!(s.contains("op-stack deposit tx received but is not supported"), "{s:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_send_value_deposit_transaction() {
    // enable the Optimism flag
    let (api, handle) =
        spawn(NodeConfig::test().with_optimism(true).with_hardfork(Some(Hardfork::Paris))).await;

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let signer: EthereumWallet = accounts[0].clone().into();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let send_value = U256::from(1234);
    let before_balance_to = provider.get_balance(to).await.unwrap();

    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_value(send_value)
        .with_gas_limit(21000);
    let tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: tx,
        other: OptimismTransactionFields {
            source_hash: Some(b256!(
                "0000000000000000000000000000000000000000000000000000000000000000"
            )),
            mint: Some(0),
            is_system_tx: Some(true),
        }
        .into(),
    };

    let pending = provider.send_transaction(tx).await.unwrap().register().await.unwrap();

    // mine block
    api.evm_mine(None).await.unwrap();

    let receipt =
        provider.get_transaction_receipt(pending.tx_hash().to_owned()).await.unwrap().unwrap();
    assert_eq!(receipt.from, from);
    assert_eq!(receipt.to, Some(to));

    // the recipient should have received the value
    let after_balance_to = provider.get_balance(to).await.unwrap();
    assert_eq!(after_balance_to, before_balance_to + send_value);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_send_value_raw_deposit_transaction() {
    // enable the Optimism flag
    let (api, handle) =
        spawn(NodeConfig::test().with_optimism(true).with_hardfork(Some(Hardfork::Paris))).await;

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let signer: EthereumWallet = accounts[0].clone().into();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer.clone());

    let send_value = U256::from(1234);
    let before_balance_to = provider.get_balance(to).await.unwrap();

    let tx = TransactionRequest::default()
        .with_chain_id(31337)
        .with_nonce(0)
        .with_from(from)
        .with_to(to)
        .with_value(send_value)
        .with_gas_limit(21_000)
        .with_max_fee_per_gas(20_000_000_000)
        .with_max_priority_fee_per_gas(1_000_000_000);
    let tx = WithOtherFields {
        inner: tx,
        other: OptimismTransactionFields {
            source_hash: Some(b256!(
                "0000000000000000000000000000000000000000000000000000000000000000"
            )),
            mint: Some(0),
            is_system_tx: Some(true),
        }
        .into(),
    };
    let tx_envelope = tx.build(&signer).await.unwrap();
    let mut tx_buffer = Vec::with_capacity(tx_envelope.encode_2718_len());
    tx_envelope.encode_2718(&mut tx_buffer);
    let tx_encoded = tx_buffer.as_slice();

    let pending =
        provider.send_raw_transaction(tx_encoded).await.unwrap().register().await.unwrap();

    // mine block
    api.evm_mine(None).await.unwrap();

    let receipt =
        provider.get_transaction_receipt(pending.tx_hash().to_owned()).await.unwrap().unwrap();
    assert_eq!(receipt.from, from);
    assert_eq!(receipt.to, Some(to));

    // the recipient should have received the value
    let after_balance_to = provider.get_balance(to).await.unwrap();
    assert_eq!(after_balance_to, before_balance_to + send_value);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_deposit_transaction_hash_matches_sepolia() {
    // enable the Optimism flag
    let (_api, handle) =
        spawn(NodeConfig::test().with_optimism(true).with_hardfork(Some(Hardfork::Paris))).await;

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let signer: EthereumWallet = accounts[0].clone().into();
    // https://sepolia-optimism.etherscan.io/tx/0xbf8b5f08c43e4b860715cd64fc0849bbce0d0ea20a76b269e7bc8886d112fca7
    let tx_hash: TxHash = "0xbf8b5f08c43e4b860715cd64fc0849bbce0d0ea20a76b269e7bc8886d112fca7"
        .parse::<TxHash>()
        .unwrap();

    // https://sepolia-optimism.etherscan.io/getRawTx?tx=0xbf8b5f08c43e4b860715cd64fc0849bbce0d0ea20a76b269e7bc8886d112fca7
    let raw_deposit_tx = alloy_primitives::hex::decode(
        "7ef861a0dfd7ae78bf3c414cfaa77f13c0205c82eb9365e217b2daa3448c3156b69b27ac94778f2146f48179643473b82931c4cd7b8f153efd94778f2146f48179643473b82931c4cd7b8f153efd872386f26fc10000872386f26fc10000830186a08080",
    )
    .unwrap();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer.clone());

    let receipt = provider
        .send_raw_transaction(raw_deposit_tx.as_slice())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    assert_eq!(receipt.transaction_hash, tx_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_deposit_tx_checks_sufficient_funds_after_applying_deposited_value() {
    // enable the Optimism flag
    let (_api, handle) =
        spawn(NodeConfig::test().with_optimism(true).with_hardfork(Some(Hardfork::Paris))).await;

    let provider = http_provider(&handle.http_endpoint());

    let sender = Address::random();
    let recipient = Address::random();
    let send_value = 1_000_000_000_u128;

    let sender_prev_balance = provider.get_balance(sender).await.unwrap();
    assert_eq!(sender_prev_balance, U256::from(0));

    let recipient_prev_balance = provider.get_balance(recipient).await.unwrap();
    assert_eq!(recipient_prev_balance, U256::from(0));

    let deposit_tx = DepositTransaction {
        source_hash: b256!("0000000000000000000000000000000000000000000000000000000000000000"),
        from: sender,
        nonce: 0,
        kind: TxKind::Call(recipient),
        mint: U256::from(send_value),
        value: U256::from(send_value),
        gas_limit: 21_000,
        is_system_tx: false,
        input: Vec::new().into(),
    };

    let mut tx_buffer = Vec::new();
    deposit_tx.encode_2718(&mut tx_buffer);

    provider.send_raw_transaction(&tx_buffer).await.unwrap().get_receipt().await.unwrap();

    let sender_new_balance = provider.get_balance(sender).await.unwrap();
    // sender should've sent the entire deposited value to recipient
    assert_eq!(sender_new_balance, U256::from(0));

    let recipient_new_balance = provider.get_balance(recipient).await.unwrap();
    // recipient should've received the entire deposited value
    assert_eq!(recipient_new_balance, U256::from(send_value));
}
