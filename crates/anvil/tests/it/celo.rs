//! CIP-64 envelope compatibility with native fee accounting.

use alloy_consensus::SignableTransaction;
use alloy_network::{ReceiptResponse, TxSignerSync, eip2718::Encodable2718};
use alloy_primitives::{Address, Signature, U256, address, bytes};
use alloy_provider::{PendingTransactionBuilder, Provider};
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, spawn};
use anvil_core::types::{ReorgOptions, TransactionData};
use foundry_evm_networks::NetworkConfigs;
use foundry_primitives::{FoundryTransactionRequest, FoundryTxEnvelope, FoundryTypedTx, TxCip64};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn cip64_fill_raw_send_and_native_fees() {
    let (api, handle) = spawn(NodeConfig::test().with_networks(NetworkConfigs::with_celo())).await;
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let sender = wallets[0].address();
    let recipient = wallets[1].address();
    let currency = address!("2222222222222222222222222222222222222222");
    let request = json!({"from":sender,"to":recipient,"value":"0x1","feeCurrency":currency});
    let filled = api.fill_transaction(serde_json::from_value(request).unwrap()).await.unwrap();
    let filled_json = serde_json::to_value(&filled.tx).unwrap();
    assert_eq!(filled_json["feeCurrency"], json!(currency));
    assert_eq!(filled_json["type"], "0x7b");
    assert!(filled_json.get("gasPrice").is_none());
    let request: FoundryTransactionRequest = serde_json::from_value(filled_json).unwrap();
    let FoundryTypedTx::Celo(mut tx) = request.build_typed_tx().unwrap() else {
        panic!("CIP-64 request")
    };
    let signature = wallets[0].sign_transaction_sync(&mut tx).unwrap();
    let signed = tx.into_signed(signature);
    let expected_hash = *signed.hash();
    let balance = provider.get_balance(sender).await.unwrap();
    let snapshot = api.evm_snapshot().await.unwrap();
    let envelope = FoundryTxEnvelope::Celo(signed);
    let pending = provider.send_raw_transaction(&envelope.encoded_2718()).await.unwrap();
    assert_eq!(*pending.tx_hash(), expected_hash);
    let receipt = pending.get_receipt().await.unwrap();
    let fee = U256::from(receipt.gas_used()) * U256::from(receipt.effective_gas_price());
    assert_eq!(provider.get_balance(sender).await.unwrap(), balance - fee - U256::ONE);
    let receipt_json = serde_json::to_value(&receipt).unwrap();
    assert_eq!(receipt_json["type"], "0x7b");
    assert_eq!(receipt_json["feeCurrency"], json!(currency));
    let mined = provider.get_transaction_by_hash(expected_hash).await.unwrap().unwrap();
    assert_eq!(serde_json::to_value(&mined).unwrap()["feeCurrency"], json!(currency));
    assert_eq!(FoundryTxEnvelope::try_from(mined).unwrap(), envelope);
    let trace: serde_json::Value = provider
        .raw_request(
            "debug_traceTransaction".into(),
            (expected_hash, json!({"tracer":"callTracer"})),
        )
        .await
        .unwrap();
    assert_eq!(trace["type"], "CALL");
    assert_eq!(trace["from"], json!(sender));
    assert_eq!(trace["to"], json!(recipient));

    assert!(api.evm_revert(snapshot).await.unwrap());
    assert_eq!(provider.get_balance(sender).await.unwrap(), balance);
    assert_eq!(provider.get_transaction_count(sender).await.unwrap(), 0);

    // The unlocked-account path must also preserve the currency and use the CIP-64 signer.
    let request: WithOtherFields<alloy_rpc_types::TransactionRequest> =
        serde_json::from_value(json!({"from":sender,"to":recipient,"feeCurrency":currency}))
            .unwrap();
    let hash = api.send_transaction(request).await.unwrap();
    let receipt =
        PendingTransactionBuilder::new(provider.root().clone(), hash).get_receipt().await.unwrap();
    assert_eq!(serde_json::to_value(receipt).unwrap()["type"], "0x7b");
}

#[tokio::test(flavor = "multi_thread")]
async fn cip64_requires_celo_mode() {
    let (api, _handle) = spawn(NodeConfig::test()).await;
    let request = serde_json::from_value(json!({"type":"0x7b","to":Address::ZERO})).unwrap();
    let error = api.fill_transaction(request).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid transaction request: CIP-64 transactions require Celo mode (--celo)"
    );
    let envelope = FoundryTxEnvelope::Celo(TxCip64::default().into_signed(Signature::new(
        U256::ONE,
        U256::from(2),
        false,
    )));
    let error = api.send_raw_transaction(envelope.encoded_2718().into()).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid transaction request: CIP-64 transactions require Celo mode (--celo)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cip64_revert_charges_native_fees() {
    let (api, handle) = spawn(NodeConfig::test().with_networks(NetworkConfigs::with_celo())).await;
    let provider = handle.http_provider();
    let wallet = handle.dev_wallets().next().unwrap();
    let sender = wallet.address();
    let contract = address!("3333333333333333333333333333333333333333");
    api.anvil_set_code(contract, bytes!("60006000fd")).await.unwrap();
    let request = json!({"from":sender,"to":contract,"value":"0x1","feeCurrency":"0x2222222222222222222222222222222222222222"});
    assert!(api.fill_transaction(serde_json::from_value(request.clone()).unwrap()).await.is_err());
    let mut request = request;
    request["gas"] = json!("0x186a0");
    let balance = provider.get_balance(sender).await.unwrap();
    let hash = api.send_transaction(serde_json::from_value(request).unwrap()).await.unwrap();
    let receipt =
        PendingTransactionBuilder::new(provider.root().clone(), hash).get_receipt().await.unwrap();
    assert!(!receipt.status());
    assert_eq!(
        provider.get_balance(sender).await.unwrap(),
        balance - U256::from(receipt.gas_used()) * U256::from(receipt.effective_gas_price())
    );
    assert_eq!(provider.get_balance(contract).await.unwrap(), U256::ZERO);
    assert_eq!(provider.get_transaction_count(sender).await.unwrap(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn cip64_impersonated_hashes() {
    let (api, handle) = spawn(NodeConfig::test().with_networks(NetworkConfigs::with_celo())).await;
    let provider = handle.http_provider();
    let sender = handle.dev_wallets().next().unwrap().address();
    api.anvil_impersonate_account(sender).await.unwrap();
    api.anvil_set_auto_mine(false).await.unwrap();
    let request = json!({"from":sender,"to":Address::ZERO,"feeCurrency":"0x2222222222222222222222222222222222222222"});
    let hash = api.send_transaction(serde_json::from_value(request).unwrap()).await.unwrap();
    let pending: serde_json::Value =
        provider.raw_request("eth_getTransactionByHash".into(), (hash,)).await.unwrap();
    assert_eq!(pending["hash"], json!(hash));
    api.mine_one().await.unwrap();
    let mined: serde_json::Value =
        provider.raw_request("eth_getTransactionByHash".into(), (hash,)).await.unwrap();
    assert_eq!(mined["hash"], json!(hash));
    let receipt = provider.get_transaction_receipt(hash).await.unwrap().unwrap();
    assert_eq!(receipt.transaction_hash(), hash);
    let block: serde_json::Value =
        provider.raw_request("eth_getBlockByNumber".into(), ("latest", true)).await.unwrap();
    assert_eq!(block["transactions"][0]["hash"], json!(hash));
}

#[tokio::test(flavor = "multi_thread")]
async fn cip64_reorg_and_state_import_require_celo_mode() {
    let (source, handle) =
        spawn(NodeConfig::test().with_networks(NetworkConfigs::with_celo())).await;
    let provider = handle.http_provider();
    let wallet = handle.dev_wallets().next().unwrap();
    let sender = wallet.address();
    let currency = address!("2222222222222222222222222222222222222222");
    let mut tx = TxCip64 {
        inner: alloy_consensus::TxEip1559 {
            chain_id: source.chain_id(),
            gas_limit: 21_000,
            max_fee_per_gas: 2_000_000_000,
            max_priority_fee_per_gas: 1,
            to: Address::ZERO.into(),
            ..Default::default()
        },
        fee_currency: Some(currency),
    };
    let signature = wallet.sign_transaction_sync(&mut tx).unwrap();
    let raw = FoundryTxEnvelope::Celo(tx.into_signed(signature)).encoded_2718();
    let receipt = provider.send_raw_transaction(&raw).await.unwrap().get_receipt().await.unwrap();
    let hash = receipt.transaction_hash();
    let dump = source.anvil_dump_state(Some(true)).await.unwrap();
    let (restored, restored_handle) =
        spawn(NodeConfig::test().with_networks(NetworkConfigs::with_celo())).await;
    assert!(restored.anvil_load_state(dump.clone()).await.unwrap());
    let restored_provider = restored_handle.http_provider();
    let transaction = restored_provider.get_transaction_by_hash(hash).await.unwrap().unwrap();
    assert_eq!(serde_json::to_value(transaction).unwrap()["feeCurrency"], json!(currency));
    let receipt = restored_provider.get_transaction_receipt(hash).await.unwrap().unwrap();
    assert_eq!(serde_json::to_value(receipt).unwrap()["feeCurrency"], json!(currency));
    let trace: serde_json::Value = restored_provider
        .raw_request("debug_traceTransaction".into(), (hash, json!({"tracer":"callTracer"})))
        .await
        .unwrap();
    assert_eq!(trace["from"], json!(sender));
    let json_request: TransactionData = serde_json::from_value(json!({"from":sender,"to":Address::ZERO,"gas":"0x5208","maxFeePerGas":"0x77359400","maxPriorityFeePerGas":"0x1","feeCurrency":currency})).unwrap();
    source
        .anvil_reorg(ReorgOptions { depth: 1, tx_block_pairs: vec![(json_request.clone(), 0)] })
        .await
        .unwrap();
    let block: serde_json::Value =
        provider.raw_request("eth_getBlockByNumber".into(), ("latest", true)).await.unwrap();
    assert_eq!(block["transactions"][0]["type"], "0x7b");
    assert_eq!(block["transactions"][0]["feeCurrency"], json!(currency));
    let profiles = [
        NetworkConfigs::with_ethereum(),
        NetworkConfigs::with_tempo(),
        #[cfg(feature = "base")]
        NetworkConfigs::with_base(),
        #[cfg(feature = "optimism")]
        NetworkConfigs::with_optimism(),
        #[cfg(feature = "monad")]
        NetworkConfigs::with_monad(),
    ];
    for profile in profiles {
        let (api, target) = spawn(NodeConfig::test().with_networks(profile)).await;
        let provider = target.http_provider();
        api.mine_one().await.unwrap();
        let before = provider
            .get_block_by_number(alloy_rpc_types::BlockNumberOrTag::Latest)
            .await
            .unwrap()
            .unwrap();
        let balance = provider.get_balance(sender).await.unwrap();
        for transaction in [TransactionData::Raw(raw.clone().into()), json_request.clone()] {
            let error = api
                .anvil_reorg(ReorgOptions { depth: 1, tx_block_pairs: vec![(transaction, 0)] })
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Invalid transaction request: CIP-64 transactions require Celo mode (--celo)"
            );
        }
        let error = api.anvil_load_state(dump.clone()).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "Invalid transaction request: CIP-64 transactions require Celo mode (--celo)"
        );
        assert_eq!(
            provider
                .get_block_by_number(alloy_rpc_types::BlockNumberOrTag::Latest)
                .await
                .unwrap()
                .unwrap()
                .header
                .hash,
            before.header.hash
        );
        assert_eq!(provider.get_balance(sender).await.unwrap(), balance);
        assert_eq!(provider.get_transaction_count(sender).await.unwrap(), 0);
    }
}
