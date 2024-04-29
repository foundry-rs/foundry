use crate::utils::http_provider;
use alloy_consensus::{
    Blob, BlobTransactionSidecar, Bytes48, SidecarBuilder, SimpleCoder, TxEip4844,
};
use alloy_eips::BlockId;
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

    let gas_limit = provider.estimate_gas(&tx, BlockId::latest()).await.unwrap();

    tx.set_gas_limit(gas_limit);

    println!("Before {:#?}", tx.blob_versioned_hashes);

    tx.populate_blob_hashes();

    println!("After {:#?}", tx.blob_versioned_hashes); // Populated successfully

    let build_tx = tx.can_build();

    assert!(build_tx); // Passes

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    println!("{:#?}", receipt);
}
