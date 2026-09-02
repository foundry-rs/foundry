//! Tests for OP chain support.

use crate::utils::{http_provider, http_provider_with_signer};
use alloy_consensus::{Eip658Value, Receipt, proofs::calculate_receipt_root};
use alloy_eips::{calc_next_block_base_fee, eip1559::BaseFeeParams, eip2718::Encodable2718};
use alloy_network::{EthereumWallet, NetworkTransactionBuilder, TransactionBuilder};
use alloy_primitives::{Address, B256, Bloom, Bytes, TxHash, TxKind, U256, address, b256};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, TransactionRequest, anvil::Forking};
use alloy_serde::{OtherFields, WithOtherFields};
use anvil::{NodeConfig, eth::fees::INITIAL_BASE_FEE, spawn};
use axum::{Json, Router, routing::post};
use foundry_evm::hardfork::OpHardfork;
use foundry_evm_networks::NetworkConfigs;
use foundry_primitives::FoundryReceiptEnvelope;
use op_alloy_consensus::{OpDepositReceipt, OpDepositReceiptWithBloom, TxDeposit};
use op_alloy_rpc_types::OpTransactionFields;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn inferred_optimism_forks_allow_non_monad_source_resets() {
    let (_optimism_api, optimism_handle) = spawn(NodeConfig::test().with_optimism()).await;
    let (ethereum_api, _) = spawn(NodeConfig::test()).await;

    ethereum_api
        .anvil_reset(Some(Forking {
            json_rpc_url: Some(optimism_handle.http_endpoint()),
            block_number: Some(0),
        }))
        .await
        .unwrap();
    let node_info = ethereum_api.anvil_node_info().await.unwrap();
    assert_eq!(node_info.network.as_deref(), Some("ethereum"));
    assert_eq!(node_info.fork_config.fork_url, Some(optimism_handle.http_endpoint()));

    let (optimism_api, _) = spawn(
        NodeConfig::test()
            .with_no_storage_caching(true)
            .with_eth_rpc_url(Some(optimism_handle.http_endpoint()))
            .with_fork_block_number(Some(0u64)),
    )
    .await;
    let (_ethereum_origin_api, ethereum_origin) = spawn(NodeConfig::test()).await;
    optimism_api
        .anvil_reset(Some(Forking {
            json_rpc_url: Some(ethereum_origin.http_endpoint()),
            block_number: Some(0),
        }))
        .await
        .unwrap();
    let node_info = optimism_api.anvil_node_info().await.unwrap();
    assert_eq!(node_info.network.as_deref(), Some("optimism"));
    assert_eq!(node_info.fork_config.fork_url, Some(ethereum_origin.http_endpoint()));
}

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
async fn test_tempo_fields_do_not_override_op_deposit_classification() {
    let (_api, handle) =
        spawn(NodeConfig::test().with_networks(NetworkConfigs::with_optimism())).await;
    let provider = handle.http_provider();
    let accounts: Vec<_> = handle.dev_wallets().collect();

    let op_fields = OpTransactionFields {
        source_hash: Some(b256!(
            "0x0000000000000000000000000000000000000000000000000000000000000000"
        )),
        mint: Some(0),
        is_system_tx: Some(true),
        deposit_receipt_version: None,
    };
    let mut other = OtherFields::try_from(serde_json::to_value(op_fields).unwrap()).unwrap();
    other.insert("calls".to_string(), json!([]));
    let tx = WithOtherFields {
        inner: TransactionRequest::default()
            .with_from(accounts[0].address())
            .with_to(accounts[1].address())
            .with_gas_limit(21_000),
        other,
    };

    provider.call(tx).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_call_does_not_charge_operator_fee() {
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_networks(NetworkConfigs::with_optimism())
            .with_hardfork(Some(OpHardfork::Isthmus.into())),
    )
    .await;
    let provider = handle.http_provider();
    let caller = Address::random();
    let target = Address::random();
    let l1_block = address!("0x4200000000000000000000000000000000000015");
    let mut operator_fee_params = [0u8; 32];
    operator_fee_params[24..].copy_from_slice(&1_351_351_351_351u64.to_be_bytes());

    api.anvil_set_storage_at(l1_block, U256::from(8), B256::from(operator_fee_params))
        .await
        .unwrap();
    api.anvil_set_code(target, Bytes::from_static(&[0x00])).await.unwrap();

    let request = WithOtherFields::new(
        TransactionRequest::default().with_from(caller).with_to(target).with_gas_limit(21_000),
    );
    provider.call(request.clone()).await.unwrap();
    provider.call(WithOtherFields::new(request.inner.clone().with_gas_price(0))).await.unwrap();

    let priced_request = WithOtherFields::new(request.inner.with_gas_price(1_000_000_000));
    let err = provider.call(priced_request).await.unwrap_err();
    assert!(err.to_string().contains("Insufficient funds for gas * price + value"), "{err}");

    let value_request = WithOtherFields::new(
        TransactionRequest::default()
            .with_from(caller)
            .with_to(target)
            .with_gas_limit(21_000)
            .with_value(U256::ONE),
    );
    let err = provider.call(value_request).await.unwrap_err();
    assert!(err.to_string().contains("Insufficient funds for gas * price + value"), "{err}");

    for validation in [false, true] {
        let err = provider
            .raw_request::<_, Value>(
                "eth_simulateV1".into(),
                (json!({
                    "blockStateCalls": [{
                        "calls": [{
                            "from": caller,
                            "to": target,
                            "gas": "0x5208",
                            "maxFeePerGas": "0x3b9aca00",
                            "maxPriorityFeePerGas": "0x0"
                        }]
                    }],
                    "validation": validation,
                }),),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Insufficient funds for gas * price + value"), "{err}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simulated_op_deposit_receipt_root_includes_canyon_fields() {
    let (_api, handle) = spawn(
        NodeConfig::test()
            .with_networks(NetworkConfigs::with_optimism())
            .with_hardfork(Some(OpHardfork::Canyon.into())),
    )
    .await;
    let provider = handle.http_provider();
    let accounts: Vec<_> = handle.dev_wallets().collect();
    let response = provider
        .raw_request::<_, Value>(
            "eth_simulateV1".into(),
            (json!({
                "blockStateCalls": [{
                    "calls": [{
                        "from": accounts[0].address(),
                        "to": accounts[1].address(),
                        "gas": "0x5208",
                        "sourceHash": b256!(
                            "0x0000000000000000000000000000000000000000000000000000000000000000"
                        ),
                        "mint": "0x0",
                        "isSystemTx": false,
                        "calls": [],
                    }]
                }],
                "returnFullTransactions": true,
            }),),
        )
        .await
        .unwrap();

    assert_eq!(response[0]["transactions"][0]["type"], "0x7e");
    let gas_used = u64::from_str_radix(
        response[0]["calls"][0]["gasUsed"].as_str().unwrap().trim_start_matches("0x"),
        16,
    )
    .unwrap();
    let receipt =
        Receipt { status: Eip658Value::Eip658(true), cumulative_gas_used: gas_used, logs: vec![] }
            .with_bloom();
    let receipt = FoundryReceiptEnvelope::Deposit(OpDepositReceiptWithBloom {
        receipt: OpDepositReceipt {
            inner: receipt.receipt,
            deposit_nonce: Some(0),
            deposit_receipt_version: Some(1),
        },
        logs_bloom: receipt.logs_bloom,
    });
    assert_eq!(response[0]["receiptsRoot"], json!(calculate_receipt_root(&[receipt])));
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
    let tx_envelope: alloy_network::AnyTxEnvelope = tx.build(&signer).await.unwrap();
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

async fn spawn_rpc_proxy_with_extra_data(endpoint: String, extra_data: &'static str) -> String {
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
                    block.insert("extraData".to_string(), Value::String(extra_data.to_string()));
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
async fn jovian_mining_and_simulation_use_da_footprint() {
    const DA_FOOTPRINT_SCALAR: u16 = 400;
    const EXPECTED_DA_FOOTPRINT: u64 = 100 * DA_FOOTPRINT_SCALAR as u64;
    const JOVIAN_EXTRA_DATA: &str = "0x01000000fa000000020000000000000000";
    const TRANSACTION_COUNT: usize = 30;
    const OVERFLOW_TRANSACTION_COUNT: usize = 26;

    let (origin_api, origin) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(8453u64))
            .with_networks(NetworkConfigs::with_optimism())
            .with_hardfork(Some(OpHardfork::Jovian.into()))
            .with_gas_limit(Some(30_000_000)),
    )
    .await;
    let l1_block = address!("0x4200000000000000000000000000000000000015");
    let mut scalar_slot = [0u8; 32];
    scalar_slot[18..20].copy_from_slice(&DA_FOOTPRINT_SCALAR.to_be_bytes());
    origin_api
        .anvil_set_storage_at(l1_block, U256::from(8), B256::from(scalar_slot))
        .await
        .unwrap();

    let fork_url = spawn_rpc_proxy_with_extra_data(origin.http_endpoint(), JOVIAN_EXTRA_DATA).await;
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_no_storage_caching(true)
            .with_eth_rpc_url(Some(fork_url))
            .with_fork_block_number(Some(0u64))
            .with_hardfork(Some(OpHardfork::Jovian.into())),
    )
    .await;
    let wallet = handle.dev_wallets().next().unwrap();
    let to = Address::random();
    let provider = http_provider_with_signer(&handle.http_endpoint(), wallet.clone().into());

    let simulated = provider
        .raw_request::<_, Value>(
            "eth_simulateV1".into(),
            (json!({
                "blockStateCalls": [{
                    "calls": [{
                        "from": wallet.address(),
                        "to": to,
                        "gas": "0x5208",
                    }]
                }]
            }),),
        )
        .await
        .unwrap();
    assert_eq!(simulated[0]["blobGasUsed"], json!(format!("0x{EXPECTED_DA_FOOTPRINT:x}")));
    assert_eq!(simulated[0]["excessBlobGas"], "0x0");

    let calls = (0..TRANSACTION_COUNT)
        .map(|_| json!({ "from": wallet.address(), "to": to, "gas": "0x5208" }))
        .collect::<Vec<_>>();
    let simulated = provider
        .raw_request::<_, Value>(
            "eth_simulateV1".into(),
            (json!({ "blockStateCalls": [{ "calls": calls }] }),),
        )
        .await
        .unwrap();
    assert_eq!(
        simulated[0]["blobGasUsed"],
        json!(format!("0x{:x}", TRANSACTION_COUNT as u64 * EXPECTED_DA_FOOTPRINT))
    );

    let calls = (0..OVERFLOW_TRANSACTION_COUNT)
        .map(|_| json!({ "from": wallet.address(), "to": to, "gas": "0x5208" }))
        .collect::<Vec<_>>();
    let overflow: Result<Value, _> = provider
        .raw_request(
            "eth_simulateV1".into(),
            (json!({
                "blockStateCalls": [{
                    "blockOverrides": { "gasLimit": "0xf4240" },
                    "calls": calls,
                }]
            }),),
        )
        .await;
    let error = overflow.unwrap_err();
    assert!(
        error.as_error_resp().unwrap().message.starts_with("blob gas usage exceeds the limit of ")
    );

    api.anvil_set_auto_mine(false).await.unwrap();
    let mut pending = Vec::new();
    for _ in 0..TRANSACTION_COUNT {
        pending.push(
            provider
                .send_transaction(WithOtherFields::new(
                    TransactionRequest::default().with_to(to).with_value(U256::ONE),
                ))
                .await
                .unwrap(),
        );
    }
    api.mine_one().await.unwrap();
    pending.remove(0).get_receipt().await.unwrap();
    let transaction_block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(transaction_block.transactions.len(), TRANSACTION_COUNT);
    assert_eq!(
        transaction_block.header.blob_gas_used,
        Some(TRANSACTION_COUNT as u64 * EXPECTED_DA_FOOTPRINT)
    );
    assert_eq!(transaction_block.header.excess_blob_gas, Some(0));

    api.mine_one().await.unwrap();
    let next_block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(
        next_block.header.base_fee_per_gas,
        Some(calc_next_block_base_fee(
            transaction_block.header.blob_gas_used.unwrap(),
            transaction_block.header.gas_limit,
            transaction_block.header.base_fee_per_gas.unwrap(),
            BaseFeeParams::new(250, 2),
        )),
        "Jovian should price the next block from its DA footprint",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn local_jovian_blocks_include_dynamic_fee_parameters() {
    const DA_FOOTPRINT_SCALAR: u16 = 6_000;

    let (api, handle) = spawn(
        NodeConfig::test()
            .with_networks(NetworkConfigs::with_optimism())
            .with_hardfork(Some(OpHardfork::Jovian.into()))
            .with_gas_limit(Some(1_000_000)),
    )
    .await;
    let l1_block = address!("0x4200000000000000000000000000000000000015");
    let mut scalar_slot = [0u8; 32];
    scalar_slot[18..20].copy_from_slice(&DA_FOOTPRINT_SCALAR.to_be_bytes());
    let wallet = handle.dev_wallets().next().unwrap();
    let provider = http_provider_with_signer(&handle.http_endpoint(), wallet.clone().into());
    let transaction = WithOtherFields::new(
        TransactionRequest::default().with_to(Address::random()).with_value(U256::ONE),
    );

    api.anvil_set_storage_at(l1_block, U256::from(8), B256::from(scalar_slot)).await.unwrap();
    provider.send_transaction(transaction.clone()).await.unwrap().get_receipt().await.unwrap();
    let transaction_block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    api.mine_one().await.unwrap();
    let next_block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert!(next_block.header.base_fee_per_gas > transaction_block.header.base_fee_per_gas);

    api.anvil_reset(None).await.unwrap();
    api.anvil_set_storage_at(l1_block, U256::from(8), B256::from(scalar_slot)).await.unwrap();
    let provider = http_provider_with_signer(&handle.http_endpoint(), wallet.into());
    provider.send_transaction(transaction).await.unwrap().get_receipt().await.unwrap();
    let transaction_block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    api.mine_one().await.unwrap();

    let block = handle.http_provider().get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(block.header.extra_data.len(), 17);
    assert_eq!(block.header.extra_data[0], 1);
    assert!(block.header.base_fee_per_gas > transaction_block.header.base_fee_per_gas);
    assert_eq!(block.header.base_fee_per_gas, next_block.header.base_fee_per_gas);
}

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

#[tokio::test(flavor = "multi_thread")]
async fn inferred_optimism_fork_uses_optimism_base_fee_params() {
    let (_origin_api, origin_handle) = spawn(
        NodeConfig::test()
            .with_networks(NetworkConfigs::with_optimism())
            .with_base_fee(Some(INITIAL_BASE_FEE))
            .with_gas_limit(Some(GAS_TRANSFER)),
    )
    .await;
    let origin_wallet = origin_handle.dev_wallets().next().unwrap();
    let origin_provider =
        http_provider_with_signer(&origin_handle.http_endpoint(), origin_wallet.into());
    let tx = TransactionRequest::default().to(Address::random()).with_value(U256::from(1));
    origin_provider
        .send_transaction(WithOtherFields::new(tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let (fork_api, fork_handle) = spawn(
        NodeConfig::test()
            .with_no_storage_caching(true)
            .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
            .with_fork_block_number(Some(1u64)),
    )
    .await;
    assert_eq!(fork_api.anvil_node_info().await.unwrap().network.as_deref(), Some("optimism"));

    let fork_wallet = fork_handle.dev_wallets().next().unwrap();
    let fork_provider = http_provider_with_signer(&fork_handle.http_endpoint(), fork_wallet.into());
    let tx = TransactionRequest::default().to(Address::random()).with_value(U256::from(1));
    fork_provider
        .send_transaction(WithOtherFields::new(tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let block = fork_provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(block.header.base_fee_per_gas, Some(INITIAL_BASE_FEE + INITIAL_BASE_FEE * 5 / 250));
}
