use crate::utils::http_provider;
use alloy_consensus::{Blob, SidecarBuilder, SimpleCoder, Transaction};
use alloy_network::{TransactionBuilder, TransactionBuilder4844};
use alloy_primitives::{B256, FixedBytes, U256, b256};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use alloy_rpc_types_beacon::{genesis::GenesisResponse, sidecar::GetBlobsResponse};
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, spawn};
use foundry_evm::hardfork::EthereumHardfork;
use ssz::Decode;

#[tokio::test(flavor = "multi_thread")]
async fn test_beacon_api_get_blob_sidecars() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;

    // Test Beacon API endpoint using HTTP client
    let client = reqwest::Client::new();
    let url = format!("{}/eth/v1/beacon/blob_sidecars/latest", handle.http_endpoint());

    // This endpoint is deprecated, so we expect a 410 Gone response
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(
        response.text().await.unwrap(),
        r#"{"code":410,"message":"This endpoint is deprecated. Use `GET /eth/v1/beacon/blobs/{block_id}` instead."}"#,
        "Expected deprecation message for blob_sidecars endpoint"
    );
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
    assert_eq!(
        response.headers().get("content-type").and_then(|h| h.to_str().ok()),
        Some("application/json"),
        "Expected application/json content-type header"
    );

    let blobs_response: GetBlobsResponse = response.json().await.unwrap();
    // Verify response structure
    assert!(!blobs_response.execution_optimistic);
    assert!(!blobs_response.finalized);

    // Verify we have blob data from all transactions
    assert_eq!(blobs_response.data.len(), 3, "Expected 3 blobs from 3 transactions");

    // Test response with SSZ encoding
    let url = format!("{}/eth/v1/beacon/blobs/{}", handle.http_endpoint(), block_number);
    let response = client
        .get(&url)
        .header(axum::http::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").and_then(|h| h.to_str().ok()),
        Some("application/octet-stream"),
        "Expected application/octet-stream content-type header"
    );

    let body_bytes = response.bytes().await.unwrap();

    // Decode the SSZ-encoded blobs in a spawned thread with larger stack to handle recursion
    let decoded_blobs = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024) // 8MB stack for SSZ decoding of large blobs
        .spawn(move || Vec::<Blob>::from_ssz_bytes(&body_bytes))
        .expect("Failed to spawn decode thread")
        .join()
        .expect("Decode thread panicked")
        .expect("Failed to decode SSZ-encoded blobs");

    // Verify we got exactly 3 blobs
    assert_eq!(
        decoded_blobs.len(),
        3,
        "Expected 3 blobs from SSZ-encoded response, got {}",
        decoded_blobs.len()
    );

    // Verify the decoded blobs match the JSON response blobs
    for (i, (decoded, json)) in decoded_blobs.iter().zip(blobs_response.data.iter()).enumerate() {
        assert_eq!(decoded, json, "Blob {i} mismatch between SSZ and JSON responses");
    }

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
