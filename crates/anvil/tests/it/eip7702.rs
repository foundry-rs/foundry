use crate::utils::http_provider;
use alloy_consensus::{transaction::TxEip7702, SignableTransaction};
use alloy_network::{ReceiptResponse, TransactionBuilder, TxSignerSync};
use alloy_primitives::{bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{Authorization, TransactionRequest};
use alloy_serde::WithOtherFields;
use alloy_signer::SignerSync;
use anvil::{spawn, EthereumHardfork, NodeConfig};

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

    let eip1559_est = provider.estimate_eip1559_fees(None).await.unwrap();

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
    tx.tx().encode_with_signature(tx.signature(), &mut encoded, false);

    let receipt =
        provider.send_raw_transaction(&encoded).await.unwrap().get_receipt().await.unwrap();
    let log = &receipt.inner.inner.logs()[0];
    // assert that log was from EOA which signed authorization
    assert_eq!(log.address(), from);
    assert_eq!(log.topics().len(), 0);
    assert_eq!(log.data().data, log_data);
}
