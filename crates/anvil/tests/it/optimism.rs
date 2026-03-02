//! Tests for OP chain support.

use crate::utils::{http_provider, http_provider_with_signer};
use alloy_eips::eip2718::Encodable2718;
use alloy_network::{EthereumWallet, NetworkTransactionBuilder, TransactionBuilder};
use alloy_primitives::{Address, Bloom, TxHash, TxKind, U256, b256};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, TransactionRequest};
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, eth::fees::INITIAL_BASE_FEE, spawn};
use foundry_evm_networks::NetworkConfigs;
use op_alloy_consensus::TxDeposit;
use op_alloy_rpc_types::OpTransactionFields;
use serde_json::{Value, json};

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

    let op_fields = OpTransactionFields {
        source_hash: Some(b256!(
            "0x0000000000000000000000000000000000000000000000000000000000000000"
        )),
        mint: Some(0),
        is_system_tx: Some(true),
        deposit_receipt_version: None,
    };

    let other = serde_json::to_value(op_fields).unwrap().try_into().unwrap();

    let tx = WithOtherFields { inner: tx, other };

    let err = provider.send_transaction(tx).await.unwrap_err();
    let s = err.to_string();
    assert!(s.contains("op-stack deposit tx received but is not supported"), "{s:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_send_value_deposit_transaction() {
    // enable the Optimism flag
    let (api, handle) =
        spawn(NodeConfig::test().with_networks(NetworkConfigs::with_optimism())).await;

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let signer: EthereumWallet = accounts[0].clone().into();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let send_value = U256::from(1234);
    let before_balance_to = provider.get_balance(to).await.unwrap();

    let op_fields = OpTransactionFields {
        source_hash: Some(b256!(
            "0x0000000000000000000000000000000000000000000000000000000000000000"
        )),
        mint: Some(0),
        is_system_tx: Some(true),
        deposit_receipt_version: None,
    };

    let other = serde_json::to_value(op_fields).unwrap().try_into().unwrap();
    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_value(send_value)
        .with_gas_limit(21000);
    let tx: WithOtherFields<TransactionRequest> = WithOtherFields { inner: tx, other };

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
        spawn(NodeConfig::test().with_networks(NetworkConfigs::with_optimism())).await;

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

    let op_fields = OpTransactionFields {
        source_hash: Some(b256!(
            "0x0000000000000000000000000000000000000000000000000000000000000000"
        )),
        mint: Some(0),
        is_system_tx: Some(true),
        deposit_receipt_version: None,
    };
    let other = serde_json::to_value(op_fields).unwrap().try_into().unwrap();
    let tx = WithOtherFields { inner: tx, other };
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
        spawn(NodeConfig::test().with_networks(NetworkConfigs::with_optimism())).await;

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
        spawn(NodeConfig::test().with_networks(NetworkConfigs::with_optimism())).await;

    let provider = http_provider(&handle.http_endpoint());

    let sender = Address::random();
    let recipient = Address::random();
    let send_value = 1_000_000_000_u128;

    let sender_prev_balance = provider.get_balance(sender).await.unwrap();
    assert_eq!(sender_prev_balance, U256::from(0));

    let recipient_prev_balance = provider.get_balance(recipient).await.unwrap();
    assert_eq!(recipient_prev_balance, U256::from(0));

    let deposit_tx = TxDeposit {
        source_hash: b256!("0x0000000000000000000000000000000000000000000000000000000000000000"),
        from: sender,
        to: TxKind::Call(recipient),
        mint: send_value,
        value: U256::from(send_value),
        gas_limit: 21_000,
        is_system_transaction: false,
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

#[test]
fn preserves_op_fields_in_convert_to_anvil_receipt() {
    let receipt_json = json!({
        "status": "0x1",
        "cumulativeGasUsed": "0x74e483",
        "logs": [],
        "logsBloom": Bloom::default(),
        "type": "0x2",
        "transactionHash": "0x91181b0dca3b29aa136eeb2f536be5ce7b0aebc949be1c44b5509093c516097d",
        "transactionIndex": "0x10",
        "blockHash": "0x54bafb12e8cea9bb355fbf03a4ac49e42a2a1a80fa6cf4364b342e2de6432b5d",
        "blockNumber": "0x7b1ab93",
        "gasUsed": "0xc222",
        "effectiveGasPrice": "0x18961",
        "from": "0x2d815240a61731c75fa01b2793e1d3ed09f289d0",
        "to":   "0x4200000000000000000000000000000000000000",
        "contractAddress": Value::Null,
        "l1BaseFeeScalar":     "0x146b",
        "l1BlobBaseFee":       "0x6a83078",
        "l1BlobBaseFeeScalar": "0xf79c5",
        "l1Fee":               "0x51a9af7fd3",
        "l1GasPrice":          "0x972fe4acc",
        "l1GasUsed":           "0x640",
    });

    let receipt: alloy_network::AnyTransactionReceipt =
        serde_json::from_value(receipt_json).expect("valid receipt json");

    let converted =
        foundry_primitives::FoundryTxReceipt::try_from(receipt).expect("conversion should succeed");
    let converted_json = serde_json::to_value(&converted).expect("serialize to json");

    for (key, expected) in [
        ("l1Fee", "0x51a9af7fd3"),
        ("l1GasPrice", "0x972fe4acc"),
        ("l1GasUsed", "0x640"),
        ("l1BaseFeeScalar", "0x146b"),
        ("l1BlobBaseFee", "0x6a83078"),
        ("l1BlobBaseFeeScalar", "0xf79c5"),
    ] {
        let got = converted_json.get(key).and_then(Value::as_str);
        assert_eq!(got, Some(expected), "field `{key}` mismatch");
    }
}

const GAS_TRANSFER: u64 = 21_000;

/// Test that Optimism uses Canyon base fee params instead of Ethereum params.
///
/// Optimism Canyon uses different EIP-1559 parameters:
/// - elasticity_multiplier: 6 (vs Ethereum's 2)
/// - base_fee_max_change_denominator: 250 (vs Ethereum's 8)
///
/// This means with a full block:
/// - Ethereum: base_fee increases by base_fee * 1 / 8 = 12.5%
/// - Optimism: base_fee increases by base_fee * 5 / 250 = 2%
#[tokio::test(flavor = "multi_thread")]
async fn test_optimism_base_fee_params() {
    // Spawn an Optimism node with a gas limit equal to one transfer (full block scenario)
    let (_api, handle) = spawn(
        NodeConfig::test()
            .with_networks(NetworkConfigs::with_optimism())
            .with_base_fee(Some(INITIAL_BASE_FEE))
            .with_gas_limit(Some(GAS_TRANSFER)),
    )
    .await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumWallet = wallet.clone().into();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let tx = TransactionRequest::default().to(Address::random()).with_value(U256::from(1337));
    let tx = WithOtherFields::new(tx);

    // Send first transaction to fill the block
    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let base_fee = provider
        .get_block(BlockId::latest())
        .await
        .unwrap()
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    // Send second transaction to fill the next block
    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let next_base_fee = provider
        .get_block(BlockId::latest())
        .await
        .unwrap()
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    assert!(next_base_fee > base_fee, "base fee should increase with full block");

    // Optimism Canyon formula: base_fee * (elasticity - 1) / denominator = base_fee * 5 / 250
    // = INITIAL_BASE_FEE * 5 / 250 = 1_000_000_000 * 5 / 250 = 20_000_000
    //
    // Note: Ethereum would be INITIAL_BASE_FEE + 125_000_000 (12.5% increase)
    let expected_op_increase = INITIAL_BASE_FEE * 5 / 250; // 2% increase = 20_000_000
    assert_eq!(
        next_base_fee,
        INITIAL_BASE_FEE + expected_op_increase,
        "Optimism should use Canyon base fee params (2% max increase), not Ethereum's (12.5%)"
    );

    // Explicitly verify it's NOT using Ethereum params (which would give 12.5% increase)
    let ethereum_increase = INITIAL_BASE_FEE / 8; // 125_000_000
    assert_ne!(
        next_base_fee,
        INITIAL_BASE_FEE + ethereum_increase,
        "Should not be using Ethereum base fee params"
    );
}
