use crate::utils::{http_provider, http_provider_with_signer};
use alloy_consensus::{
    BlobTransactionSidecar, EthereumTxEnvelope, SidecarBuilder, SimpleCoder, Transaction,
    TxEip4844, proofs::calculate_transaction_root,
};
use alloy_eips::{
    eip2718::{Decodable2718, EIP4844_TX_TYPE_ID, Typed2718},
    eip4844::{
        BLOB_TX_MIN_BLOB_GASPRICE, DATA_GAS_PER_BLOB, MAX_DATA_GAS_PER_BLOCK_DENCUN,
        TARGET_DATA_GAS_PER_BLOCK_DENCUN,
    },
    eip7840::BlobParams,
};
use alloy_network::{
    AnyRpcTransaction, AnyTxEnvelope, EthereumWallet, ReceiptResponse, TransactionBuilder,
    TransactionBuilder4844,
};
use alloy_primitives::{Address, Bytes, U256, b256};
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types::{Authorization, BlockId, TransactionRequest};
use alloy_serde::WithOtherFields;
use alloy_signer::SignerSync;
use anvil::{NodeConfig, spawn};
use foundry_evm::hardfork::EthereumHardfork;
use foundry_test_utils::rpc;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn non_blob_receipt_omits_blob_fields() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());
    let accounts = handle.dev_accounts().collect::<Vec<_>>();

    let tx = TransactionRequest::default().with_from(accounts[0]).with_to(accounts[1]);
    let receipt = provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();

    assert_eq!(receipt.blob_gas_used, None);
    assert_eq!(receipt.blob_gas_price, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_send_eip4844_transaction() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (api, handle) = spawn(node_config).await;

    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let from = wallets[0].address();
    let to = wallets[1].address();
    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();
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
        .with_blob_sidecar_4844(sidecar)
        .value(U256::from(5));

    let tx = WithOtherFields::new(tx);

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert_eq!(receipt.blob_gas_used, Some(131072));
    assert_eq!(receipt.blob_gas_price, Some(0x1)); // 1 wei

    let raw: Bytes = provider
        .client()
        .request("eth_getRawTransactionByHash", (receipt.transaction_hash,))
        .await
        .unwrap();
    let canonical = EthereumTxEnvelope::<TxEip4844>::decode_2718(&mut raw.as_ref()).unwrap();
    let block = provider
        .get_block_by_number(receipt.block_number.unwrap().into())
        .full()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(block.header.transactions_root, calculate_transaction_root(&[canonical]));
    let tx = serde_json::to_value(&block.transactions.as_transactions().unwrap()[0]).unwrap();
    for field in ["blobs", "commitments", "proofs", "cellProofs"] {
        assert!(tx.get(field).is_none());
    }

    let raw_transactions: Vec<Bytes> = provider
        .client()
        .request("debug_getRawTransactions", (BlockId::number(block.header.number),))
        .await
        .unwrap();
    assert_eq!(raw_transactions.len(), 1);
    EthereumTxEnvelope::<TxEip4844>::decode_2718(&mut raw_transactions[0].as_ref()).unwrap();

    let raw_block: Bytes = provider
        .client()
        .request("debug_getRawBlock", (BlockId::number(block.header.number),))
        .await
        .unwrap();
    assert_eq!(block.header.size, Some(U256::from(raw_block.len())));
    let decoded: alloy_consensus::Block<EthereumTxEnvelope<TxEip4844>> =
        alloy_rlp::Decodable::decode(&mut raw_block.as_ref()).unwrap();
    assert_eq!(decoded.body.transactions.len(), 1);

    assert!(api.anvil_get_blob_by_tx_hash(receipt.transaction_hash).unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_send_eip4844_transaction_fork() {
    let node_config = NodeConfig::test()
        .with_eth_rpc_url(Some(rpc::next_http_archive_rpc_url()))
        .with_fork_block_number(Some(23432306u64))
        .with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (api, handle) = spawn(node_config).await;
    let provider = handle.http_provider();
    let accounts = provider.get_accounts().await.unwrap();
    let alice = accounts[0];
    let bob = accounts[1];

    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(b"Blobs are fun!");
    let sidecar: BlobTransactionSidecar = sidecar.build().unwrap();

    let tx = TransactionRequest::default()
        .with_from(alice)
        .with_to(bob)
        .with_blob_sidecar_4844(sidecar.clone());

    let pending_tx = provider.send_transaction(tx.into()).await.unwrap();
    let receipt = pending_tx.get_receipt().await.unwrap();
    let tx_hash = receipt.transaction_hash;

    let _blobs = api.anvil_get_blob_by_tx_hash(tx_hash).unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_send_eip4844_transaction_eth_send_transaction() {
    let node_config = NodeConfig::test()
        .with_eth_rpc_url(Some(rpc::next_http_archive_rpc_url()))
        .with_fork_block_number(Some(23552208u64))
        .with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (api, handle) = spawn(node_config).await;
    let provider = ProviderBuilder::new().connect(handle.http_endpoint().as_str()).await.unwrap();
    let accounts = provider.get_accounts().await.unwrap();
    let alice = accounts[0];
    let bob = accounts[1];

    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(b"Blobs are fun!");
    let sidecar: BlobTransactionSidecar = sidecar.build().unwrap();

    let tx = TransactionRequest::default()
        .with_from(alice)
        .with_to(bob)
        .with_blob_sidecar_4844(sidecar.clone());

    let pending_tx = provider.send_transaction(tx).await.unwrap();
    let receipt = pending_tx.get_receipt().await.unwrap();
    let tx_hash = receipt.transaction_hash;

    let _blobs = api.anvil_get_blob_by_tx_hash(tx_hash).unwrap().unwrap();
}

// <https://github.com/foundry-rs/foundry/issues/13217>
#[tokio::test(flavor = "multi_thread")]
async fn can_send_eip4844_transaction_with_eip7594_sidecar_format() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Osaka.into()));
    let (api, handle) = spawn(node_config).await;
    let provider = ProviderBuilder::new().connect(handle.http_endpoint().as_str()).await.unwrap();
    let accounts = provider.get_accounts().await.unwrap();
    let alice = accounts[0];
    let bob = accounts[1];

    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(b"Blobs are fun!");
    let sidecar = sidecar.build_7594().unwrap();

    let tx =
        TransactionRequest::default().with_from(alice).with_to(bob).with_blob_sidecar_7594(sidecar);

    let pending_tx = provider.send_transaction(tx).await.unwrap();
    let receipt = pending_tx.get_receipt().await.unwrap();
    let tx_hash = receipt.transaction_hash;

    let _blobs = api.anvil_get_blob_by_tx_hash(tx_hash).unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_send_multiple_blobs_in_one_tx() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;

    let wallets = handle.dev_wallets().collect::<Vec<_>>();

    let from = wallets[0].address();
    let to = wallets[1].address();

    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();
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
        .with_blob_sidecar_4844(sidecar);
    let tx = WithOtherFields::new(tx);

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert_eq!(receipt.blob_gas_used, Some(MAX_DATA_GAS_PER_BLOCK_DENCUN));
    assert_eq!(receipt.blob_gas_price, Some(0x1)); // 1 wei
}

#[tokio::test(flavor = "multi_thread")]
async fn cannot_exceed_six_blobs() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;

    let wallets = handle.dev_wallets().collect::<Vec<_>>();

    let from = wallets[0].address();
    let to = wallets[1].address();

    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();
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
        .with_blob_sidecar_4844(sidecar);
    let tx = WithOtherFields::new(tx);

    let err = provider.send_transaction(tx).await.unwrap_err();

    assert!(err.to_string().contains("too many blobs"));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_mine_blobs_when_exceeds_max_blobs() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (api, handle) = spawn(node_config).await;
    api.anvil_set_auto_mine(false).await.unwrap();

    let wallets = handle.dev_wallets().collect::<Vec<_>>();

    let from = wallets[0].address();
    let to = wallets[1].address();

    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    let first_batch = vec![1u8; DATA_GAS_PER_BLOB as usize * 3];
    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(&first_batch);

    let num_blobs_first = sidecar.clone().take().len() as u64;

    let sidecar = sidecar.build().unwrap();

    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_nonce(0)
        .with_max_fee_per_blob_gas(gas_price + 1)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .with_blob_sidecar_4844(sidecar);
    let mut tx = WithOtherFields::new(tx);

    let first_tx = provider.send_transaction(tx.clone()).await.unwrap();

    let second_batch = vec![1u8; DATA_GAS_PER_BLOB as usize * 2];

    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(&second_batch);

    let num_blobs_second = sidecar.clone().take().len() as u64;

    let sidecar = sidecar.build().unwrap();
    tx.set_blob_sidecar_4844(sidecar);
    tx.set_nonce(1);
    let second_tx = provider.send_transaction(tx).await.unwrap();

    api.mine_one().await.unwrap();

    let first_receipt = first_tx.get_receipt().await.unwrap();

    api.mine_one().await.unwrap();
    let second_receipt = second_tx.get_receipt().await.unwrap();

    let (first_block, second_block) = tokio::join!(
        provider.get_block_by_number(first_receipt.block_number.unwrap().into()),
        provider.get_block_by_number(second_receipt.block_number.unwrap().into())
    );
    assert_eq!(
        first_block.unwrap().unwrap().header.blob_gas_used,
        Some(DATA_GAS_PER_BLOB * num_blobs_first)
    );

    assert_eq!(
        second_block.unwrap().unwrap().header.blob_gas_used,
        Some(DATA_GAS_PER_BLOB * num_blobs_second)
    );
    // Mined in two different blocks
    assert_eq!(first_receipt.block_number.unwrap() + 1, second_receipt.block_number.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_check_blob_fields_on_genesis() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;

    let provider = http_provider(&handle.http_endpoint());

    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();

    assert_eq!(block.header.blob_gas_used, Some(0));
    assert_eq!(block.header.excess_blob_gas, Some(0));
}

#[expect(clippy::disallowed_macros)]
#[tokio::test(flavor = "multi_thread")]
async fn can_correctly_estimate_blob_gas_with_recommended_fillers() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;

    let provider = http_provider(&handle.http_endpoint());

    let accounts = provider.get_accounts().await.unwrap();
    let alice = accounts[0];
    let bob = accounts[1];

    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(b"Blobs are fun!");
    let sidecar = sidecar.build().unwrap();

    let tx = TransactionRequest::default().with_to(bob).with_blob_sidecar_4844(sidecar);
    let tx = WithOtherFields::new(tx);

    // Send the transaction and wait for the broadcast.
    let pending_tx = provider.send_transaction(tx).await.unwrap();

    println!("Pending transaction... {}", pending_tx.tx_hash());

    // Wait for the transaction to be included and get the receipt.
    let receipt = pending_tx.get_receipt().await.unwrap();

    // Grab the processed transaction.
    let tx = provider.get_transaction_by_hash(receipt.transaction_hash).await.unwrap().unwrap();

    println!(
        "Transaction included in block {}",
        receipt.block_number.expect("Failed to get block number")
    );

    assert!(tx.max_fee_per_blob_gas().unwrap() >= BLOB_TX_MIN_BLOB_GASPRICE);
    assert_eq!(receipt.from, alice);
    assert_eq!(receipt.to, Some(bob));
    assert_eq!(
        receipt.blob_gas_used.expect("Expected to be EIP-4844 transaction"),
        DATA_GAS_PER_BLOB
    );
}

#[expect(clippy::disallowed_macros)]
#[tokio::test(flavor = "multi_thread")]
async fn can_correctly_estimate_blob_gas_with_recommended_fillers_with_signer() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;

    let signer = handle.dev_wallets().next().unwrap();
    let wallet: EthereumWallet = signer.clone().into();

    let provider = http_provider_with_signer(&handle.http_endpoint(), wallet);

    let accounts = provider.get_accounts().await.unwrap();
    let alice = accounts[0];
    let bob = accounts[1];

    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(b"Blobs are fun!");
    let sidecar = sidecar.build().unwrap();

    let tx = TransactionRequest::default().with_to(bob).with_blob_sidecar_4844(sidecar);
    let tx = WithOtherFields::new(tx);

    // Send the transaction and wait for the broadcast.
    let pending_tx = provider.send_transaction(tx).await.unwrap();

    println!("Pending transaction... {}", pending_tx.tx_hash());

    // Wait for the transaction to be included and get the receipt.
    let receipt = pending_tx.get_receipt().await.unwrap();

    // Grab the processed transaction.
    let tx = provider.get_transaction_by_hash(receipt.transaction_hash).await.unwrap().unwrap();

    println!(
        "Transaction included in block {}",
        receipt.block_number.expect("Failed to get block number")
    );

    assert!(tx.max_fee_per_blob_gas().unwrap() >= BLOB_TX_MIN_BLOB_GASPRICE);
    assert_eq!(receipt.from, alice);
    assert_eq!(receipt.to, Some(bob));
    assert_eq!(
        receipt.blob_gas_used.expect("Expected to be EIP-4844 transaction"),
        DATA_GAS_PER_BLOB
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_malformed_eip4844_transaction() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;

    let err = handle.http_provider().send_raw_transaction(&[EIP4844_TX_TYPE_ID]).await.unwrap_err();

    assert!(err.to_string().contains("Failed to decode transaction"));
}

// <https://github.com/foundry-rs/foundry/issues/9924>
#[tokio::test]
async fn can_bypass_sidecar_requirement() {
    crate::init_tracing();
    let node_config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_auto_impersonate(true);
    let (api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    let from = Address::random();
    let to = Address::random();

    api.anvil_set_balance(from, U256::from(60262144030131080_u128)).await.unwrap();

    let tx = TransactionRequest {
        from: Some(from),
        to: Some(alloy_primitives::TxKind::Call(to)),
        nonce: Some(0),
        value: Some(U256::from(0)),
        max_fee_per_blob_gas: Some(gas_price + 1),
        max_fee_per_gas: Some(eip1559_est.max_fee_per_gas),
        max_priority_fee_per_gas: Some(eip1559_est.max_priority_fee_per_gas),
        blob_versioned_hashes: Some(vec![b256!(
            "0x01d5446006b21888d0267829344ab8624fdf1b425445a8ae1ca831bf1b8fbcd4"
        )]),
        sidecar: None,
        transaction_type: Some(3),
        ..Default::default()
    };

    let receipt = provider
        .send_transaction(WithOtherFields::new(tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    assert!(receipt.status());

    let tx = provider.get_transaction_by_hash(receipt.transaction_hash).await.unwrap().unwrap();

    assert_eq!(tx.inner.ty(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_blobs_by_versioned_hash() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (api, handle) = spawn(node_config).await;

    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let from = wallets[0].address();
    let to = wallets[1].address();
    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(b"Hello World");

    let sidecar: BlobTransactionSidecar = sidecar.build().unwrap();
    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_nonce(0)
        .with_max_fee_per_blob_gas(gas_price + 1)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .with_blob_sidecar_4844(sidecar.clone())
        .value(U256::from(5));

    let tx = WithOtherFields::new(tx);

    let _receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let hash = sidecar.versioned_hash_for_blob(0).unwrap();
    // api.anvil_set_auto_mine(true).await.unwrap();
    let blob = api.anvil_get_blob_by_versioned_hash(hash).unwrap().unwrap();
    assert_eq!(blob, sidecar.blobs[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_blobs_by_tx_hash() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (api, handle) = spawn(node_config).await;

    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let from = wallets[0].address();
    let to = wallets[1].address();
    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(b"Hello World");

    let sidecar: BlobTransactionSidecar = sidecar.build().unwrap();
    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_nonce(0)
        .with_max_fee_per_blob_gas(gas_price + 1)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .with_blob_sidecar_4844(sidecar.clone())
        .value(U256::from(5));

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    let hash = receipt.transaction_hash;
    api.anvil_set_auto_mine(true).await.unwrap();
    let blobs = api.anvil_get_blob_by_tx_hash(hash).unwrap().unwrap();
    assert_eq!(blobs, sidecar.blobs);
}

#[tokio::test(flavor = "multi_thread")]
async fn simulate_v1_derives_blob_hashes_from_sidecars() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());
    let accounts = provider.get_accounts().await.unwrap();
    let contract = Address::with_last_byte(0x42);
    let sidecar: BlobTransactionSidecar =
        SidecarBuilder::<SimpleCoder>::from_slice(b"Hello World").build().unwrap();
    let versioned_hash = sidecar.versioned_hash_for_blob(0).unwrap();
    let request = TransactionRequest::default()
        .with_from(accounts[0])
        .with_to(contract)
        .with_blob_sidecar_4844(sidecar);
    let payload = json!({
        "blockStateCalls": [{
            "stateOverrides": {
                contract.to_string(): {
                    "code": "0x5f495f5260205ff3"
                }
            },
            "calls": [request]
        }],
        "returnFullTransactions": true
    });

    let response: Value =
        provider.client().request("eth_simulateV1", (payload, "latest")).await.unwrap();
    let block = &response[0];

    assert_eq!(block["calls"][0]["returnData"], versioned_hash.to_string());
    let transaction = &block["transactions"][0];
    for field in ["blobs", "commitments", "proofs", "cellProofs"] {
        assert!(transaction.get(field).is_none(), "unexpected pooled field {field}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn simulate_v1_defaults_blob_fee_cap_to_zero() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());
    let accounts = provider.get_accounts().await.unwrap();
    let contract = Address::with_last_byte(0x42);
    let sidecar: BlobTransactionSidecar =
        SidecarBuilder::<SimpleCoder>::from_slice(b"Hello World").build().unwrap();
    let request = TransactionRequest {
        from: Some(accounts[0]),
        to: Some(contract.into()),
        blob_versioned_hashes: Some(vec![sidecar.versioned_hash_for_blob(0).unwrap()]),
        max_fee_per_gas: Some(2_000_000_000),
        max_priority_fee_per_gas: Some(0),
        ..Default::default()
    };
    let block = |blob_base_fee: Option<u64>| {
        json!({
            "blockOverrides": blob_base_fee.map(|blob_base_fee| {
                json!({"blobBaseFee": format!("0x{blob_base_fee:x}")})
            }),
            "stateOverrides": {
                contract.to_string(): {
                    "code": "0x4a5f5260205ff3"
                }
            },
            "calls": [request.clone()]
        })
    };
    let payload = json!({
        "blockStateCalls": [block(None)],
        "validation": true,
        "returnFullTransactions": true
    });

    let response: Result<Value, _> =
        provider.client().request("eth_simulateV1", (payload, "latest")).await;
    let error = response.unwrap_err();
    let error = error.as_error_resp().unwrap();
    assert_eq!(error.code, -32003);
    assert_eq!(
        error.message,
        "Block `blob_gas_price` is greater than tx-specified `max_fee_per_blob_gas`"
    );

    let payload = json!({
        "blockStateCalls": [block(None)],
        "validation": false,
        "returnFullTransactions": true
    });
    let response: Value =
        provider.client().request("eth_simulateV1", (payload, "latest")).await.unwrap();
    assert_eq!(response[0]["calls"][0]["returnData"], format!("0x{:064x}", 1));
    assert_eq!(response[0]["transactions"][0]["maxFeePerBlobGas"], "0x0");

    let payload = json!({
        "blockStateCalls": [block(Some(21))],
        "validation": false,
        "returnFullTransactions": true
    });
    let response: Value =
        provider.client().request("eth_simulateV1", (payload, "latest")).await.unwrap();
    assert_eq!(response[0]["calls"][0]["returnData"], format!("0x{:064x}", 21));
    assert_eq!(response[0]["transactions"][0]["maxFeePerBlobGas"], "0x0");

    let sender = Address::with_last_byte(0x43);
    let payload = |max_fee_per_blob_gas, balance| {
        let mut request = request.clone();
        request.from = Some(sender);
        request.gas = Some(30_000);
        request.max_fee_per_gas = Some(0);
        request.max_priority_fee_per_gas = Some(0);
        request.max_fee_per_blob_gas = max_fee_per_blob_gas;
        json!({
            "blockStateCalls": [{
                "blockOverrides": {"blobBaseFee": "0x15"},
                "stateOverrides": {
                    sender.to_string(): {"balance": format!("0x{balance:x}")},
                    contract.to_string(): {"code": "0x4a5f5260205ff3"}
                },
                "calls": [request]
            }],
            "validation": false,
            "returnFullTransactions": true
        })
    };

    for max_fee_per_blob_gas in [None, Some(0)] {
        let response: Value = provider
            .client()
            .request("eth_simulateV1", (payload(max_fee_per_blob_gas, 0), "latest"))
            .await
            .unwrap();
        assert_eq!(response[0]["calls"][0]["returnData"], format!("0x{:064x}", 21));
        assert_eq!(response[0]["transactions"][0]["maxFeePerBlobGas"], "0x0");
    }

    let response: Result<Value, _> =
        provider.client().request("eth_simulateV1", (payload(Some(20), 4_000_000), "latest")).await;
    assert_eq!(response.unwrap_err().as_error_resp().unwrap().code, -32003);

    let response: Result<Value, _> =
        provider.client().request("eth_simulateV1", (payload(Some(30), 3_000_000), "latest")).await;
    assert_eq!(response.unwrap_err().as_error_resp().unwrap().code, -38014);

    let response: Value = provider
        .client()
        .request("eth_simulateV1", (payload(Some(30), 4_000_000), "latest"))
        .await
        .unwrap();
    assert_eq!(response[0]["calls"][0]["returnData"], format!("0x{:064x}", 21));
    assert_eq!(response[0]["transactions"][0]["maxFeePerBlobGas"], "0x1e");
}

#[tokio::test(flavor = "multi_thread")]
async fn simulate_v1_executes_the_canonical_transaction_type() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Berlin.into()));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());
    let accounts = provider.get_accounts().await.unwrap();
    let request = TransactionRequest {
        from: Some(accounts[0]),
        to: Some(accounts[1].into()),
        access_list: Some(Default::default()),
        ..Default::default()
    };
    let payload = json!({
        "blockStateCalls": [{"calls": [request]}],
        "returnFullTransactions": true
    });

    let response: Result<Value, _> =
        provider.client().request("eth_simulateV1", (payload, "latest")).await;

    assert!(response.is_err(), "canonical EIP-1559 transaction executed before London");
}

#[tokio::test(flavor = "multi_thread")]
async fn simulate_v1_only_accounts_blobs_for_canonical_eip4844_transactions() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());
    let accounts = provider.get_accounts().await.unwrap();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let sidecar: BlobTransactionSidecar =
        SidecarBuilder::<SimpleCoder>::from_slice(b"Hello World").build().unwrap();
    let authorization =
        Authorization { chain_id: U256::from(31337), address: accounts[2], nonce: 0 };
    let signature = wallets[1].sign_hash_sync(&authorization.signature_hash()).unwrap();
    let mut request = TransactionRequest::default()
        .with_from(accounts[0])
        .with_to(accounts[1])
        .with_blob_sidecar_4844(sidecar);
    request.authorization_list = Some(vec![authorization.into_signed(signature)]);
    let payload = json!({
        "blockStateCalls": [{"calls": [request]}],
        "returnFullTransactions": true
    });

    let response: Value =
        provider.client().request("eth_simulateV1", (payload, "latest")).await.unwrap();

    let transaction = &response[0]["transactions"][0];
    assert_eq!(transaction["type"], "0x4");
    for field in ["blobVersionedHashes", "blobs", "commitments", "proofs", "cellProofs"] {
        assert!(transaction.get(field).is_none(), "unexpected blob field {field}");
    }
    assert_eq!(response[0]["blobGasUsed"], "0x0");
}

#[tokio::test(flavor = "multi_thread")]
async fn simulate_v1_uses_canonical_blob_transaction_root() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());
    let accounts = provider.get_accounts().await.unwrap();
    let sidecar: BlobTransactionSidecar =
        SidecarBuilder::<SimpleCoder>::from_slice(b"Hello World").build().unwrap();
    let request = TransactionRequest {
        from: Some(accounts[0]),
        to: Some(accounts[1].into()),
        transaction_type: Some(3),
        blob_versioned_hashes: Some(vec![sidecar.versioned_hash_for_blob(0).unwrap()]),
        max_fee_per_blob_gas: Some(BLOB_TX_MIN_BLOB_GASPRICE),
        ..Default::default()
    };
    let payload = json!({
        "blockStateCalls": [{"calls": [request]}],
        "returnFullTransactions": true
    });

    let response: Value =
        provider.client().request("eth_simulateV1", (payload, "latest")).await.unwrap();
    let block = &response[0];
    let transaction: AnyRpcTransaction =
        serde_json::from_value(block["transactions"][0].clone()).unwrap();
    let canonical = AnyTxEnvelope::from(transaction);

    assert_eq!(block["transactionsRoot"], calculate_transaction_root(&[canonical]).to_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn simulate_v1_rejects_cumulative_blob_gas_overflow() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());
    let accounts = provider.get_accounts().await.unwrap();
    let sidecar: BlobTransactionSidecar =
        SidecarBuilder::<SimpleCoder>::from_slice(b"Hello World").build().unwrap();
    let versioned_hash = sidecar.versioned_hash_for_blob(0).unwrap();
    let full_request = TransactionRequest {
        from: Some(accounts[0]),
        to: Some(accounts[1].into()),
        blob_versioned_hashes: Some(vec![versioned_hash; 6]),
        ..Default::default()
    };
    let mut sidecar_request = TransactionRequest::default()
        .with_from(accounts[0])
        .with_to(accounts[1])
        .with_blob_sidecar_4844(sidecar);
    sidecar_request.blob_versioned_hashes = Some(Vec::new());
    let payload = json!({
        "blockStateCalls": [{"calls": [full_request, sidecar_request]}]
    });

    let response: Result<Value, _> =
        provider.client().request("eth_simulateV1", (payload, "latest")).await;
    let error = response.unwrap_err();
    let error = error.as_error_resp().unwrap();

    assert_eq!(error.code, -32602);
    assert_eq!(
        error.message,
        format!(
            "blob gas usage exceeds the limit of {MAX_DATA_GAS_PER_BLOCK_DENCUN} gas per block."
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn simulate_v1_uses_bpo_blob_gas_limits() {
    for (hardfork, blob_params) in
        [(EthereumHardfork::Bpo1, BlobParams::bpo1()), (EthereumHardfork::Bpo2, BlobParams::bpo2())]
    {
        let mut node_config = NodeConfig::test().with_hardfork(Some(hardfork.into()));
        if hardfork == EthereumHardfork::Bpo2 {
            node_config = node_config
                .with_chain_id(Some(1u64))
                .with_genesis_timestamp(EthereumHardfork::Bpo1.mainnet_activation_timestamp());
        }
        let (_api, handle) = spawn(node_config).await;
        let provider = http_provider(&handle.http_endpoint());
        let accounts = provider.get_accounts().await.unwrap();
        let sidecar: BlobTransactionSidecar =
            SidecarBuilder::<SimpleCoder>::from_slice(b"Hello World").build().unwrap();
        let versioned_hash = sidecar.versioned_hash_for_blob(0).unwrap();
        let request = |blob_count| TransactionRequest {
            from: Some(accounts[0]),
            to: Some(accounts[1].into()),
            blob_versioned_hashes: Some(vec![versioned_hash; blob_count]),
            ..Default::default()
        };
        let max_blobs = blob_params.max_blob_count as usize;
        let calls = (0..max_blobs.div_ceil(blob_params.max_blobs_per_tx as usize))
            .map(|idx| {
                request(
                    (max_blobs - idx * blob_params.max_blobs_per_tx as usize)
                        .min(blob_params.max_blobs_per_tx as usize),
                )
            })
            .collect::<Vec<_>>();
        let payload = json!({"blockStateCalls": [{"calls": calls}]});

        let response: Value =
            provider.client().request("eth_simulateV1", (payload, "latest")).await.unwrap();
        assert_eq!(
            response[0]["blobGasUsed"],
            format!("0x{:x}", blob_params.max_blob_gas_per_block())
        );

        let mut over_limit_calls = (0..max_blobs.div_ceil(blob_params.max_blobs_per_tx as usize))
            .map(|idx| {
                request(
                    (max_blobs - idx * blob_params.max_blobs_per_tx as usize)
                        .min(blob_params.max_blobs_per_tx as usize),
                )
            })
            .collect::<Vec<_>>();
        over_limit_calls.push(request(1));
        let payload = json!({"blockStateCalls": [{"calls": over_limit_calls}]});
        let response: Result<Value, _> =
            provider.client().request("eth_simulateV1", (payload, "latest")).await;
        let error = response.unwrap_err();

        assert_eq!(error.as_error_resp().unwrap().code, -32602);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn simulate_v1_updates_blob_schedule_at_bpo_timestamp() {
    let bpo1_timestamp = EthereumHardfork::Bpo1.mainnet_activation_timestamp().unwrap();
    let node_config = NodeConfig::test()
        .with_chain_id(Some(1u64))
        .with_hardfork(Some(EthereumHardfork::Osaka.into()))
        .with_genesis_timestamp(Some(bpo1_timestamp - 24));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());
    let accounts = provider.get_accounts().await.unwrap();
    let sidecar: BlobTransactionSidecar =
        SidecarBuilder::<SimpleCoder>::from_slice(b"Hello World").build().unwrap();
    let versioned_hash = sidecar.versioned_hash_for_blob(0).unwrap();
    let request = |blob_count| TransactionRequest {
        from: Some(accounts[0]),
        to: Some(accounts[1].into()),
        blob_versioned_hashes: Some(vec![versioned_hash; blob_count]),
        ..Default::default()
    };
    let payload = json!({
        "blockStateCalls": [
            {"calls": [request(6), request(3)]},
            {
                "blockOverrides": {"time": format!("0x{bpo1_timestamp:x}")},
                "calls": [request(6), request(4)]
            }
        ]
    });

    let response: Value =
        provider.client().request("eth_simulateV1", (payload, "latest")).await.unwrap();

    assert_eq!(
        response[1]["blobGasUsed"],
        format!("0x{:x}", BlobParams::bpo1().target_blob_gas_per_block())
    );
    assert_eq!(response[1]["excessBlobGas"], "0x0");
}

#[tokio::test(flavor = "multi_thread")]
async fn simulate_v1_advances_excess_blob_gas() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());
    let accounts = provider.get_accounts().await.unwrap();
    let sidecar: BlobTransactionSidecar =
        SidecarBuilder::<SimpleCoder>::from_slice(b"Hello World").build().unwrap();
    let versioned_hash = sidecar.versioned_hash_for_blob(0).unwrap();
    let request = |blob_count| TransactionRequest {
        from: Some(accounts[0]),
        to: Some(accounts[1].into()),
        blob_versioned_hashes: Some(vec![versioned_hash; blob_count]),
        ..Default::default()
    };
    let payload = json!({
        "blockStateCalls": [
            {"calls": [request(6)]},
            {"calls": [request(1)]}
        ]
    });

    let response: Value =
        provider.client().request("eth_simulateV1", (payload, "latest")).await.unwrap();

    assert_eq!(response[0]["blobGasUsed"], format!("0x{MAX_DATA_GAS_PER_BLOCK_DENCUN:x}"));
    assert_eq!(response[0]["excessBlobGas"], "0x0");
    assert_eq!(response[1]["blobGasUsed"], format!("0x{DATA_GAS_PER_BLOB:x}"));
    assert_eq!(response[1]["excessBlobGas"], format!("0x{TARGET_DATA_GAS_PER_BLOCK_DENCUN:x}"));
}
