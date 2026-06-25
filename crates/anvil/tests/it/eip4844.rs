use crate::utils::{http_provider, http_provider_with_signer};
use alloy_consensus::{
    BlobTransactionSidecar, EthereumTxEnvelope, SidecarBuilder, SimpleCoder, Transaction,
    TxEip4844, proofs::calculate_transaction_root,
};
use alloy_eips::{
    Typed2718,
    eip2718::Decodable2718,
    eip4844::{
        BLOB_TX_MIN_BLOB_GASPRICE, BYTES_PER_BLOB, DATA_GAS_PER_BLOB, MAX_DATA_GAS_PER_BLOCK_DENCUN,
    },
};
use alloy_network::{EthereumWallet, ReceiptResponse, TransactionBuilder, TransactionBuilder4844};
use alloy_primitives::{Address, Bytes, U64, U256, b256};
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types::{BlockId, TransactionRequest};
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, spawn};
use foundry_evm::hardfork::EthereumHardfork;
use foundry_test_utils::rpc;

#[tokio::test(flavor = "multi_thread")]
async fn can_send_eip4844_transaction() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;

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

    api.mine_one().await;

    let first_receipt = first_tx.get_receipt().await.unwrap();

    api.mine_one().await;
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

/// Mines one block containing a blob tx and a plain tx, then asserts the mined block commits to
/// the canonical (sidecar-less) EIP-2718 transaction encodings, like blocks on a real network.
async fn assert_mined_blob_block_is_canonical(node_config: NodeConfig, use_7594: bool) {
    let (api, handle) = spawn(node_config).await;
    api.anvil_set_auto_mine(false).await.unwrap();

    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let from = wallets[0].address();
    let to = wallets[1].address();
    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(b"canonical root test");

    let blob_tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_nonce(0)
        .with_max_fee_per_blob_gas(gas_price + 1)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .value(U256::from(1));
    let blob_tx = if use_7594 {
        blob_tx.with_blob_sidecar_7594(sidecar.build_7594().unwrap())
    } else {
        blob_tx.with_blob_sidecar_4844(sidecar.build().unwrap())
    };
    let blob_pending = provider.send_transaction(WithOtherFields::new(blob_tx)).await.unwrap();

    let plain_tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_nonce(1)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .value(U256::from(2));
    let plain_pending = provider.send_transaction(WithOtherFields::new(plain_tx)).await.unwrap();

    api.mine_one().await;

    let blob_receipt = blob_pending.get_receipt().await.unwrap();
    let plain_receipt = plain_pending.get_receipt().await.unwrap();
    assert_eq!(blob_receipt.block_number, plain_receipt.block_number);
    let block_number = blob_receipt.block_number.unwrap();

    let block = provider.get_block_by_number(block_number.into()).await.unwrap().unwrap();
    assert_eq!(block.transactions.len(), 2);

    // Fetch the raw EIP-2718 encoding of every transaction in the block. Decoding into the
    // sidecar-less envelope type enforces that mined blob transactions are served in their
    // canonical form rather than the pooled `rlp([tx, blobs, commitments, proofs])` wrapper.
    let mut leaves = Vec::new();
    for index in 0..block.transactions.len() {
        let raw: Bytes = provider
            .client()
            .request(
                "eth_getRawTransactionByBlockNumberAndIndex",
                (U64::from(block_number), U64::from(index as u64)),
            )
            .await
            .unwrap();
        assert!(
            raw.len() < BYTES_PER_BLOB,
            "raw mined tx {index} should not contain blob data ({} bytes)",
            raw.len()
        );
        let envelope = EthereumTxEnvelope::<TxEip4844>::decode_2718(&mut raw.as_ref())
            .unwrap_or_else(|err| panic!("mined tx {index} is not canonically encoded: {err}"));
        leaves.push(envelope);
    }
    assert_eq!(*leaves[0].tx_hash(), blob_receipt.transaction_hash);

    // The header must commit to the canonical transaction trie root and hash to the reported
    // block hash.
    assert_eq!(
        block.header.transactions_root,
        calculate_transaction_root(&leaves),
        "header.transactionsRoot is not the canonical transaction trie root"
    );
    let consensus_header: alloy_consensus::Header = block.header.inner.clone().try_into().unwrap();
    assert_eq!(consensus_header.hash_slow(), block.header.hash);

    // The block's blobs must remain retrievable after mining.
    let blobs = api.anvil_get_blob_by_tx_hash(blob_receipt.transaction_hash).unwrap().unwrap();
    assert!(!blobs.is_empty());

    // Full-block RPC responses must serve the standard transaction shape, without the pooled
    // sidecar fields.
    let full_block: serde_json::Value = provider
        .client()
        .request("eth_getBlockByNumber", (U64::from(block_number), true))
        .await
        .unwrap();
    let tx_json = &full_block["transactions"][0];
    for field in ["blobs", "commitments", "proofs", "cellProofs"] {
        assert!(
            tx_json.get(field).is_none(),
            "mined blob tx RPC response must not expose `{field}`"
        );
    }
}

// <https://github.com/foundry-rs/foundry/issues/15132>
#[tokio::test(flavor = "multi_thread")]
async fn mined_blob_block_has_canonical_transactions_root() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    assert_mined_blob_block_is_canonical(node_config, false).await;
}

// <https://github.com/foundry-rs/foundry/issues/15132>
#[tokio::test(flavor = "multi_thread")]
async fn mined_blob_block_has_canonical_transactions_root_eip7594() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Osaka.into()));
    assert_mined_blob_block_is_canonical(node_config, true).await;
}

// <https://github.com/foundry-rs/foundry/issues/15132>
#[tokio::test(flavor = "multi_thread")]
async fn can_trace_transaction_after_blob_tx() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (api, handle) = spawn(node_config).await;
    api.anvil_set_auto_mine(false).await.unwrap();

    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let from = wallets[0].address();
    let to = wallets[1].address();
    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(b"trace replay");
    let blob_tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_nonce(0)
        .with_max_fee_per_blob_gas(gas_price + 1)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .with_blob_sidecar_4844(sidecar.build().unwrap())
        .value(U256::from(1));
    let blob_pending = provider.send_transaction(WithOtherFields::new(blob_tx)).await.unwrap();

    let plain_tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_nonce(1)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .value(U256::from(2));
    let plain_pending = provider.send_transaction(WithOtherFields::new(plain_tx)).await.unwrap();

    api.mine_one().await;

    let blob_receipt = blob_pending.get_receipt().await.unwrap();
    let plain_receipt = plain_pending.get_receipt().await.unwrap();
    assert_eq!(blob_receipt.block_number, plain_receipt.block_number);

    // Tracing the plain tx replays the preceding blob tx from the stored block body, which
    // requires reattaching its sidecar for pool transaction validation.
    api.debug_trace_transaction(
        plain_receipt.transaction_hash,
        alloy_rpc_types::trace::geth::GethDebugTracingOptions::default(),
    )
    .await
    .unwrap();

    // Tracing the blob tx itself must work as well.
    api.debug_trace_transaction(
        blob_receipt.transaction_hash,
        alloy_rpc_types::trace::geth::GethDebugTracingOptions::default(),
    )
    .await
    .unwrap();
}

// <https://github.com/foundry-rs/foundry/issues/15132>
#[tokio::test(flavor = "multi_thread")]
async fn can_load_state_with_blob_txs() {
    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");

    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (api, handle) = spawn(node_config).await;
    api.anvil_set_auto_mine(false).await.unwrap();

    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let from = wallets[0].address();
    let to = wallets[1].address();
    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(b"dump and reload");
    let sidecar: BlobTransactionSidecar = sidecar.build().unwrap();
    let blob_tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_nonce(0)
        .with_max_fee_per_blob_gas(gas_price + 1)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .with_blob_sidecar_4844(sidecar.clone())
        .value(U256::from(1));
    let blob_pending = provider.send_transaction(WithOtherFields::new(blob_tx)).await.unwrap();

    let plain_tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_nonce(1)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .value(U256::from(2));
    let plain_pending = provider.send_transaction(WithOtherFields::new(plain_tx)).await.unwrap();

    api.mine_one().await;

    let receipt = blob_pending.get_receipt().await.unwrap();
    let plain_receipt = plain_pending.get_receipt().await.unwrap();
    assert_eq!(receipt.block_number, plain_receipt.block_number);
    let tx_hash = receipt.transaction_hash;
    let block_number = receipt.block_number.unwrap();
    let block_hash = receipt.block_hash.unwrap();

    let state = api.serialized_state(true).await.unwrap();
    foundry_common::fs::write_json_file(&state_file, &state).unwrap();

    let node_config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_init_state_path(state_file);
    let (api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());

    // The block hash must be unchanged after the dump/load round trip.
    let block = provider.get_block_by_number(block_number.into()).await.unwrap().unwrap();
    assert_eq!(block.header.hash, block_hash);

    // Blobs must be retrievable from the migrated sidecar store.
    let blobs = api.anvil_get_blob_by_tx_hash(tx_hash).unwrap().unwrap();
    assert_eq!(blobs, sidecar.blobs);

    // The mined blob tx is still served in its canonical raw form.
    let raw: Bytes = provider
        .client()
        .request(
            "eth_getRawTransactionByBlockNumberAndIndex",
            (U64::from(block_number), U64::from(0u64)),
        )
        .await
        .unwrap();
    assert!(raw.len() < BYTES_PER_BLOB);
    EthereumTxEnvelope::<TxEip4844>::decode_2718(&mut raw.as_ref()).unwrap();

    // Tracing a tx mined after the blob tx works on the loaded chain: the replay reattaches
    // the migrated sidecar.
    api.debug_trace_transaction(
        plain_receipt.transaction_hash,
        alloy_rpc_types::trace::geth::GethDebugTracingOptions::default(),
    )
    .await
    .unwrap();
}

// <https://github.com/foundry-rs/foundry/issues/15132>
#[tokio::test(flavor = "multi_thread")]
async fn reverting_snapshot_removes_blob_sidecars() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (api, handle) = spawn(node_config).await;

    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let from = wallets[0].address();
    let to = wallets[1].address();
    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    let snapshot_id = api.evm_snapshot().await.unwrap();

    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(b"reverted away");
    let sidecar: BlobTransactionSidecar = sidecar.build().unwrap();
    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_nonce(0)
        .with_max_fee_per_blob_gas(gas_price + 1)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .with_blob_sidecar_4844(sidecar.clone())
        .value(U256::from(1));
    let receipt = provider
        .send_transaction(WithOtherFields::new(tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let tx_hash = receipt.transaction_hash;
    let versioned_hash = sidecar.versioned_hash_for_blob(0).unwrap();

    assert!(api.anvil_get_blob_by_tx_hash(tx_hash).unwrap().is_some());
    assert!(api.anvil_get_blob_by_versioned_hash(versioned_hash).unwrap().is_some());

    // Reverting to the snapshot must also drop the reverted transactions' sidecars.
    assert!(api.evm_revert(snapshot_id).await.unwrap());
    assert!(api.anvil_get_blob_by_tx_hash(tx_hash).unwrap().is_none());
    assert!(api.anvil_get_blob_by_versioned_hash(versioned_hash).unwrap().is_none());
}
