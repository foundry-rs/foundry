use crate::utils::http_provider;
use alloy_consensus::{SidecarBuilder, SimpleCoder};
use alloy_eips::eip4844::{DATA_GAS_PER_BLOB, MAX_DATA_GAS_PER_BLOCK};
use alloy_network::TransactionBuilder;
use alloy_primitives::U256;
use alloy_provider::Provider;
use alloy_rpc_types::{TransactionRequest, WithOtherFields};
use anvil::{spawn, Hardfork, NodeConfig};

#[tokio::test(flavor = "multi_thread")]
async fn can_send_eip4844_transaction() {
    let node_config = NodeConfig::default().with_hardfork(Some(Hardfork::Cancun));
    let (_api, handle) = spawn(node_config).await;

    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let from = wallets[0].address();
    let to = wallets[1].address();
    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees(None).await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice("Hello World".as_bytes());

    let sidecar = sidecar.build().unwrap();
    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_nonce(0)
        .with_max_fee_per_blob_gas(gas_price + 1)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .with_blob_sidecar(sidecar)
        .value(U256::from(5));

    let mut tx = WithOtherFields::new(tx);

    tx.populate_blob_hashes();

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert_eq!(receipt.blob_gas_used, Some(131072));
    assert_eq!(receipt.blob_gas_price, Some(0x1)); // 1 wei
}

#[tokio::test(flavor = "multi_thread")]
async fn can_send_multiple_blobs_in_one_tx() {
    let node_config = NodeConfig::default().with_hardfork(Some(Hardfork::Cancun));
    let (_api, handle) = spawn(node_config).await;

    let wallets = handle.dev_wallets().collect::<Vec<_>>();

    let from = wallets[0].address();
    let to = wallets[1].address();

    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees(None).await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    let large_data = vec![1u8; DATA_GAS_PER_BLOB as usize * 5]; // 131072 is DATA_GAS_PER_BLOB and also BYTE_PER_BLOB
    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(&large_data);

    let sidecar = sidecar.build().unwrap();

    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_nonce(0)
        .with_max_fee_per_blob_gas(gas_price + 1)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .with_blob_sidecar(sidecar);
    let mut tx = WithOtherFields::new(tx);

    tx.populate_blob_hashes();

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert_eq!(receipt.blob_gas_used, Some(MAX_DATA_GAS_PER_BLOCK as u128));
    assert_eq!(receipt.blob_gas_price, Some(0x1)); // 1 wei
}

#[tokio::test(flavor = "multi_thread")]
async fn cannot_exceed_six_blobs() {
    let node_config = NodeConfig::default().with_hardfork(Some(Hardfork::Cancun));
    let (_api, handle) = spawn(node_config).await;

    let wallets = handle.dev_wallets().collect::<Vec<_>>();

    let from = wallets[0].address();
    let to = wallets[1].address();

    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees(None).await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    let large_data = vec![1u8; DATA_GAS_PER_BLOB as usize * 6]; // 131072 is DATA_GAS_PER_BLOB and also BYTE_PER_BLOB
    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(&large_data);

    let sidecar = sidecar.build().unwrap();

    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_nonce(0)
        .with_max_fee_per_blob_gas(gas_price + 1)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .with_blob_sidecar(sidecar);
    let mut tx = WithOtherFields::new(tx);

    tx.populate_blob_hashes();

    let err = provider.send_transaction(tx).await.unwrap_err();

    assert!(err.to_string().contains("too many blobs"));
}
