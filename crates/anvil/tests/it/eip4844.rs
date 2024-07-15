use crate::utils::http_provider;
use alloy_consensus::{SidecarBuilder, SimpleCoder};
use alloy_eips::eip4844::{DATA_GAS_PER_BLOB, MAX_DATA_GAS_PER_BLOCK};
use alloy_network::TransactionBuilder;
use alloy_primitives::U256;
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, TransactionRequest};
use alloy_serde::WithOtherFields;
use anvil::{spawn, Hardfork, NodeConfig};

#[tokio::test(flavor = "multi_thread")]
async fn can_send_eip4844_transaction() {
    let node_config = NodeConfig::test().with_hardfork(Some(Hardfork::Cancun));
    let (_api, handle) = spawn(node_config).await;

    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let from = wallets[0].address();
    let to = wallets[1].address();
    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees(None).await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(b"Hello World");

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
    let node_config = NodeConfig::test().with_hardfork(Some(Hardfork::Cancun));
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
    let node_config = NodeConfig::test().with_hardfork(Some(Hardfork::Cancun));
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

#[tokio::test(flavor = "multi_thread")]
async fn can_mine_blobs_when_exceeds_max_blobs() {
    let node_config = NodeConfig::test().with_hardfork(Some(Hardfork::Cancun));
    let (api, handle) = spawn(node_config).await;
    api.anvil_set_auto_mine(false).await.unwrap();

    let wallets = handle.dev_wallets().collect::<Vec<_>>();

    let from = wallets[0].address();
    let to = wallets[1].address();

    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees(None).await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    let first_batch = vec![1u8; DATA_GAS_PER_BLOB as usize * 3];
    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(&first_batch);

    let num_blobs_first = sidecar.clone().take().len();

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

    let first_tx = provider.send_transaction(tx.clone()).await.unwrap();

    let second_batch = vec![1u8; DATA_GAS_PER_BLOB as usize * 2];

    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(&second_batch);

    let num_blobs_second = sidecar.clone().take().len();

    let sidecar = sidecar.build().unwrap();
    tx.set_blob_sidecar(sidecar);
    tx.set_nonce(1);
    tx.populate_blob_hashes();
    let second_tx = provider.send_transaction(tx).await.unwrap();

    api.mine_one().await;

    let first_receipt = first_tx.get_receipt().await.unwrap();

    api.mine_one().await;
    let second_receipt = second_tx.get_receipt().await.unwrap();

    let (first_block, second_block) = tokio::join!(
        provider.get_block_by_number(first_receipt.block_number.unwrap().into(), false),
        provider.get_block_by_number(second_receipt.block_number.unwrap().into(), false)
    );
    assert_eq!(
        first_block.unwrap().unwrap().header.blob_gas_used,
        Some(DATA_GAS_PER_BLOB as u128 * num_blobs_first as u128)
    );

    assert_eq!(
        second_block.unwrap().unwrap().header.blob_gas_used,
        Some(DATA_GAS_PER_BLOB as u128 * num_blobs_second as u128)
    );
    // Mined in two different blocks
    assert_eq!(first_receipt.block_number.unwrap() + 1, second_receipt.block_number.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_check_blob_fields_on_genesis() {
    let node_config = NodeConfig::test().with_hardfork(Some(Hardfork::Cancun));
    let (_api, handle) = spawn(node_config).await;

    let provider = http_provider(&handle.http_endpoint());

    let block = provider.get_block(BlockId::latest(), false.into()).await.unwrap().unwrap();

    assert_eq!(block.header.blob_gas_used, Some(0));
    assert_eq!(block.header.excess_blob_gas, Some(0));
}
