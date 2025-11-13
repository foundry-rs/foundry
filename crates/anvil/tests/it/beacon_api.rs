use crate::utils::http_provider;
use alloy_consensus::{SidecarBuilder, SimpleCoder, Transaction};
use alloy_hardforks::EthereumHardfork;
use alloy_network::{TransactionBuilder, TransactionBuilder4844};
use alloy_primitives::{B256, FixedBytes, U256, b256};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use alloy_rpc_types_beacon::{genesis::GenesisResponse, sidecar::GetBlobsResponse};
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, spawn};

#[tokio::test(flavor = "multi_thread")]
async fn test_beacon_api_get_blob_sidecars() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (api, handle) = spawn(node_config).await;

    // Disable auto-mining so we can include multiple transactions in the same block
    api.anvil_set_auto_mine(false).await.unwrap();

    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let from = wallets[0].address();
    let to = wallets[1].address();

    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    // Create multiple blob transactions to be included in the same block
    let blob_data =
        [b"Hello Beacon API - Blob 1", b"Hello Beacon API - Blob 2", b"Hello Beacon API - Blob 3"];

    let mut pending_txs = Vec::new();

    // Send all transactions without waiting for receipts
    for (i, data) in blob_data.iter().enumerate() {
        let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(data.as_slice());
        let sidecar = sidecar.build().unwrap();

        let tx = TransactionRequest::default()
            .with_from(from)
            .with_to(to)
            .with_nonce(i as u64)
            .with_max_fee_per_blob_gas(gas_price + 1)
            .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
            .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
            .with_blob_sidecar(sidecar)
            .value(U256::from(100));

        let mut tx = WithOtherFields::new(tx);
        tx.populate_blob_hashes();

        let pending = provider.send_transaction(tx).await.unwrap();
        pending_txs.push(pending);
    }

    // Mine a block to include all transactions
    api.evm_mine(None).await.unwrap();

    // Get receipts for all transactions
    let mut receipts = Vec::new();
    for pending in pending_txs {
        let receipt = pending.get_receipt().await.unwrap();
        receipts.push(receipt);
    }

    // Verify all transactions were included in the same block
    let block_number = receipts[0].block_number.unwrap();
    for (i, receipt) in receipts.iter().enumerate() {
        assert_eq!(
            receipt.block_number.unwrap(),
            block_number,
            "Transaction {i} was not included in block {block_number}"
        );
    }

    // Test Beacon API endpoint using HTTP client
    let client = reqwest::Client::new();
    let url = format!("{}/eth/v1/beacon/blob_sidecars/{}", handle.http_endpoint(), block_number);

    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let body: serde_json::Value = response.json().await.unwrap();

    // Verify response structure
    assert!(body["data"].is_array());
    assert!(body["execution_optimistic"].is_boolean());
    assert!(body["finalized"].is_boolean());

    // Verify we have blob data from all transactions
    let blobs = body["data"].as_array().unwrap();
    assert_eq!(blobs.len(), 3, "Expected 3 blob sidecars from 3 transactions");

    // Verify blob structure for each blob
    for (i, blob) in blobs.iter().enumerate() {
        assert!(blob["index"].is_string(), "Blob {i} missing index");
        assert!(blob["blob"].is_string(), "Blob {i} missing blob data");
        assert!(blob["kzg_commitment"].is_string(), "Blob {i} missing kzg_commitment");
        assert!(blob["kzg_proof"].is_string(), "Blob {i} missing kzg_proof");
    }

    // Test filtering with indices query parameter - single index
    let url = format!(
        "{}/eth/v1/beacon/blob_sidecars/{}?indices=1",
        handle.http_endpoint(),
        block_number
    );
    let response = client.get(&url).send().await.unwrap();
    let status = response.status();
    if status != reqwest::StatusCode::OK {
        let error_body = response.text().await.unwrap();
        panic!("Expected OK status, got {status}: {error_body}");
    }
    let body: serde_json::Value = response.json().await.unwrap();
    let filtered_blobs = body["data"].as_array().unwrap();
    assert_eq!(filtered_blobs.len(), 1, "Expected 1 blob sidecar when filtering by indices=1");
    assert_eq!(filtered_blobs[0]["index"].as_str().unwrap(), "1");

    // Test filtering with indices query parameter - multiple indices (comma-separated)
    let url = format!(
        "{}/eth/v1/beacon/blob_sidecars/{}?indices=0,2",
        handle.http_endpoint(),
        block_number
    );
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = response.json().await.unwrap();
    let filtered_blobs = body["data"].as_array().unwrap();
    assert_eq!(filtered_blobs.len(), 2, "Expected 2 blob sidecars when filtering by indices=0,2");
    let indices: Vec<String> =
        filtered_blobs.iter().map(|b| b["index"].as_str().unwrap().to_string()).collect();
    assert!(indices.contains(&"0".to_string()), "Expected index 0 in results");
    assert!(indices.contains(&"2".to_string()), "Expected index 2 in results");

    // Test filtering with non-existent index
    let url = format!(
        "{}/eth/v1/beacon/blob_sidecars/{}?indices=99",
        handle.http_endpoint(),
        block_number
    );
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = response.json().await.unwrap();
    let filtered_blobs = body["data"].as_array().unwrap();
    assert_eq!(
        filtered_blobs.len(),
        0,
        "Expected 0 blob sidecars when filtering by non-existent index"
    );

    // Test with special block identifiers
    let test_ids = vec!["latest", "finalized", "safe", "earliest"];
    for block_id in test_ids {
        let url = format!("{}/eth/v1/beacon/blob_sidecars/{}", handle.http_endpoint(), block_id);
        assert_eq!(client.get(&url).send().await.unwrap().status(), reqwest::StatusCode::OK);
    }
    let url = format!("{}/eth/v1/beacon/blob_sidecars/pending", handle.http_endpoint());
    assert_eq!(client.get(&url).send().await.unwrap().status(), reqwest::StatusCode::NOT_FOUND);

    // Test with hex block number
    let url = format!("{}/eth/v1/beacon/blob_sidecars/0x{block_number:x}", handle.http_endpoint());
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    // Test with non-existent block
    let url = format!("{}/eth/v1/beacon/blob_sidecars/999999", handle.http_endpoint());
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_beacon_api_get_blobs() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (api, handle) = spawn(node_config).await;

    // Disable auto-mining so we can include multiple transactions in the same block
    api.anvil_set_auto_mine(false).await.unwrap();

    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let from = wallets[0].address();
    let to = wallets[1].address();

    let provider = http_provider(&handle.http_endpoint());

    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    // Create multiple blob transactions to be included in the same block
    let blob_data =
        [b"Hello Beacon API - Blob 1", b"Hello Beacon API - Blob 2", b"Hello Beacon API - Blob 3"];

    let mut pending_txs = Vec::new();

    // Send all transactions without waiting for receipts
    for (i, data) in blob_data.iter().enumerate() {
        let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(data.as_slice());
        let sidecar = sidecar.build().unwrap();

        let tx = TransactionRequest::default()
            .with_from(from)
            .with_to(to)
            .with_nonce(i as u64)
            .with_max_fee_per_blob_gas(gas_price + 1)
            .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
            .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
            .with_blob_sidecar(sidecar)
            .value(U256::from(100));

        let mut tx = WithOtherFields::new(tx);
        tx.populate_blob_hashes();

        let pending = provider.send_transaction(tx).await.unwrap();
        pending_txs.push(pending);
    }

    // Mine a block to include all transactions
    api.evm_mine(None).await.unwrap();

    // Get receipts for all transactions
    let mut receipts = Vec::new();
    for pending in pending_txs {
        let receipt = pending.get_receipt().await.unwrap();
        receipts.push(receipt);
    }

    // Verify all transactions were included in the same block
    let block_number = receipts[0].block_number.unwrap();
    for (i, receipt) in receipts.iter().enumerate() {
        assert_eq!(
            receipt.block_number.unwrap(),
            block_number,
            "Transaction {i} was not included in block {block_number}"
        );
    }

    // Extract the actual versioned hashes from the mined transactions
    let mut actual_versioned_hashes = Vec::new();
    for receipt in &receipts {
        let tx = provider.get_transaction_by_hash(receipt.transaction_hash).await.unwrap().unwrap();
        if let Some(blob_versioned_hashes) = tx.blob_versioned_hashes() {
            actual_versioned_hashes.extend(blob_versioned_hashes.iter().copied());
        }
    }

    // Test Beacon API endpoint using HTTP client
    let client = reqwest::Client::new();
    let url = format!("{}/eth/v1/beacon/blobs/{}", handle.http_endpoint(), block_number);

    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let blobs_response: GetBlobsResponse = response.json().await.unwrap();

    // Verify response structure
    assert!(!blobs_response.execution_optimistic);
    assert!(!blobs_response.finalized);

    // Verify we have blob data from all transactions
    assert_eq!(blobs_response.data.len(), 3, "Expected 3 blobs from 3 transactions");

    // Test filtering with versioned_hashes query parameter - single hash
    let url = format!(
        "{}/eth/v1/beacon/blobs/{}?versioned_hashes={}",
        handle.http_endpoint(),
        block_number,
        actual_versioned_hashes[1]
    );
    let response = client.get(&url).send().await.unwrap();
    let status = response.status();
    if status != reqwest::StatusCode::OK {
        let error_body = response.text().await.unwrap();
        panic!("Expected OK status, got {status}: {error_body}");
    }
    let blobs_response: GetBlobsResponse = response.json().await.unwrap();
    assert_eq!(
        blobs_response.data.len(),
        1,
        "Expected 1 blob when filtering by single versioned_hash"
    );

    // Test filtering with versioned_hashes query parameter - multiple versioned_hashes
    // (comma-separated)
    let url = format!(
        "{}/eth/v1/beacon/blobs/{}?versioned_hashes={},{}",
        handle.http_endpoint(),
        block_number,
        actual_versioned_hashes[0],
        actual_versioned_hashes[2]
    );
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let blobs_response: GetBlobsResponse = response.json().await.unwrap();
    assert_eq!(
        blobs_response.data.len(),
        2,
        "Expected 2 blobs when filtering by two versioned_hashes"
    );

    // Test filtering with non-existent versioned_hash
    let non_existent_hash =
        b256!("0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
    let url = format!(
        "{}/eth/v1/beacon/blobs/{}?versioned_hashes={}",
        handle.http_endpoint(),
        block_number,
        non_existent_hash
    );
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let blobs_response: GetBlobsResponse = response.json().await.unwrap();
    assert_eq!(
        blobs_response.data.len(),
        0,
        "Expected 0 blobs when filtering by non-existent versioned_hash"
    );

    // Test with special block identifiers
    let test_ids = vec!["latest", "finalized", "safe", "earliest"];
    for block_id in test_ids {
        let url = format!("{}/eth/v1/beacon/blobs/{}", handle.http_endpoint(), block_id);
        assert_eq!(client.get(&url).send().await.unwrap().status(), reqwest::StatusCode::OK);
    }
    let url = format!("{}/eth/v1/beacon/blobs/pending", handle.http_endpoint());
    assert_eq!(client.get(&url).send().await.unwrap().status(), reqwest::StatusCode::NOT_FOUND);

    // Test with hex block number
    let url = format!("{}/eth/v1/beacon/blobs/0x{block_number:x}", handle.http_endpoint());
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    // Test with non-existent block
    let url = format!("{}/eth/v1/beacon/blobs/999999", handle.http_endpoint());
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_beacon_api_get_genesis() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;

    // Test Beacon API genesis endpoint using HTTP client
    let client = reqwest::Client::new();
    let url = format!("{}/eth/v1/beacon/genesis", handle.http_endpoint());

    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let genesis_response: GenesisResponse = response.json().await.unwrap();

    assert!(genesis_response.data.genesis_time > 0);
    assert_eq!(genesis_response.data.genesis_validators_root, B256::ZERO);
    assert_eq!(
        genesis_response.data.genesis_fork_version,
        FixedBytes::from([0x00, 0x00, 0x00, 0x00])
    );
}
