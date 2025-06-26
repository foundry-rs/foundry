use crate::utils::http_provider;
use alloy_consensus::{transaction::TxEip7702, SignableTransaction};
use alloy_eips::Encodable2718;
use alloy_hardforks::EthereumHardfork;
use alloy_network::{ReceiptResponse, TransactionBuilder, TxSignerSync};
use alloy_primitives::{b256, bytes, Bytes, U256};
use alloy_provider::{PendingTransactionConfig, Provider};
use alloy_rpc_types::{Authorization, TransactionRequest};
use alloy_serde::WithOtherFields;
use alloy_signer::SignerSync;
use anvil::{spawn, NodeConfig};
use op_alloy_rpc_types::OpTransactionFields;

#[tokio::test(flavor = "multi_thread")]
async fn can_send_eip7702_tx() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());

    let wallets = handle.dev_wallets().collect::<Vec<_>>();

    // deploy simple contract forwarding calldata to LOG0
    // PUSH7(CALLDATASIZE PUSH0 PUSH0 CALLDATACOPY CALLDATASIZE PUSH0 LOG0) PUSH0 MSTORE PUSH1(7)
    // PUSH1(25) RETURN
    let logger_bytecode = bytes!("66365f5f37365fa05f5260076019f3");

    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();

    let from = wallets[0].address();
    let tx = TransactionRequest::default()
        .with_from(from)
        .into_create()
        .with_nonce(0)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .with_input(logger_bytecode);

    let receipt = provider
        .send_transaction(WithOtherFields::new(tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    assert!(receipt.status());

    let contract = receipt.contract_address.unwrap();
    let authorization = Authorization {
        chain_id: U256::from(31337u64),
        address: contract,
        nonce: provider.get_transaction_count(from).await.unwrap(),
    };
    let signature = wallets[0].sign_hash_sync(&authorization.signature_hash()).unwrap();
    let authorization = authorization.into_signed(signature);

    let log_data = bytes!("11112222");
    let mut tx = TxEip7702 {
        max_fee_per_gas: eip1559_est.max_fee_per_gas,
        max_priority_fee_per_gas: eip1559_est.max_priority_fee_per_gas,
        gas_limit: 100000,
        chain_id: 31337,
        to: from,
        input: bytes!("11112222"),
        authorization_list: vec![authorization],
        ..Default::default()
    };
    let signature = wallets[1].sign_transaction_sync(&mut tx).unwrap();

    let tx = tx.into_signed(signature);
    let mut encoded = Vec::new();
    tx.eip2718_encode(&mut encoded);

    let receipt =
        provider.send_raw_transaction(&encoded).await.unwrap().get_receipt().await.unwrap();
    let log = &receipt.inner.inner.logs()[0];
    // assert that log was from EOA which signed authorization
    assert_eq!(log.address(), from);
    assert_eq!(log.topics().len(), 0);
    assert_eq!(log.data().data, log_data);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_send_eip7702_request() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());

    let wallets = handle.dev_wallets().collect::<Vec<_>>();

    // deploy simple contract forwarding calldata to LOG0
    // PUSH7(CALLDATASIZE PUSH0 PUSH0 CALLDATACOPY CALLDATASIZE PUSH0 LOG0) PUSH0 MSTORE PUSH1(7)
    // PUSH1(25) RETURN
    let logger_bytecode = bytes!("66365f5f37365fa05f5260076019f3");

    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();

    let from = wallets[0].address();
    let tx = TransactionRequest::default()
        .with_from(from)
        .into_create()
        .with_nonce(0)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .with_input(logger_bytecode);

    let receipt = provider
        .send_transaction(WithOtherFields::new(tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    assert!(receipt.status());

    let contract = receipt.contract_address.unwrap();
    let authorization = Authorization {
        chain_id: U256::from(31337u64),
        address: contract,
        nonce: provider.get_transaction_count(from).await.unwrap(),
    };
    let signature = wallets[0].sign_hash_sync(&authorization.signature_hash()).unwrap();
    let authorization = authorization.into_signed(signature);

    let log_data = bytes!("11112222");
    let tx = TxEip7702 {
        max_fee_per_gas: eip1559_est.max_fee_per_gas,
        max_priority_fee_per_gas: eip1559_est.max_priority_fee_per_gas,
        gas_limit: 100000,
        chain_id: 31337,
        to: from,
        input: bytes!("11112222"),
        authorization_list: vec![authorization],
        ..Default::default()
    };

    let sender = wallets[1].address();
    let request = TransactionRequest::from_transaction(tx).with_from(sender);

    api.anvil_impersonate_account(sender).await.unwrap();
    let txhash = api.send_transaction(WithOtherFields::new(request)).await.unwrap();

    let txhash = provider
        .watch_pending_transaction(PendingTransactionConfig::new(txhash))
        .await
        .unwrap()
        .await
        .unwrap();

    let receipt = provider.get_transaction_receipt(txhash).await.unwrap().unwrap();
    let log = &receipt.inner.inner.logs()[0];
    // assert that log was from EOA which signed authorization
    assert_eq!(log.address(), from);
    assert_eq!(log.topics().len(), 0);
    assert_eq!(log.data().data, log_data);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_send_tx_sync() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (api, handle) = spawn(node_config).await;
    let accounts: Vec<_> = handle.dev_wallets().collect();
    let signer: alloy_network::EthereumWallet = accounts[0].clone().into();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let send_value = U256::from(1234);
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
    let tx_encoded = Bytes::from(tx_buffer);
    let receipt = api.send_raw_transaction_sync(tx_encoded).await.unwrap();
    assert!(receipt.block_number.is_some());
}
