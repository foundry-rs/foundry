use crate::utils::http_provider;
use alloy_consensus::{Blob, BlobTransactionSidecar, SidecarBuilder, SimpleCoder, Transaction};
use alloy_network::{TransactionBuilder, TransactionBuilder4844};
use alloy_primitives::{B256, FixedBytes, U256, b256};
use alloy_provider::Provider;
use alloy_rpc_types::{TransactionRequest, anvil::MineOptions};
use alloy_rpc_types_beacon::{genesis::GenesisResponse, sidecar::GetBlobsResponse};
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, spawn};
use foundry_evm::hardfork::EthereumHardfork;
use ssz::Decode;
use std::time::Duration;

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
        let sidecar = sidecar.build::<BlobTransactionSidecar>().unwrap();

        let tx = TransactionRequest::default()
            .with_from(from)
            .with_to(to)
            .with_nonce(i as u64)
            .with_max_fee_per_blob_gas(gas_price + 1)
            .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
            .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
            .with_blob_sidecar_4844(sidecar)
            .value(U256::from(100));

        let tx = WithOtherFields::new(tx);

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

    // Test filtering with versioned_hashes query parameter - multiple versioned_hashes
    // (repeated keys, as sent by beacon clients such as op-node).
    // Request in reverse order to verify that the response retains block order.
    let url = format!(
        "{}/eth/v1/beacon/blobs/{}?versioned_hashes={}&versioned_hashes={}",
        handle.http_endpoint(),
        block_number,
        actual_versioned_hashes[1],
        actual_versioned_hashes[0]
    );
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let blobs_response: GetBlobsResponse = response.json().await.unwrap();
    assert_eq!(
        blobs_response.data,
        decoded_blobs[..2],
        "Expected the requested blobs in block order when filtering by repeated versioned_hashes keys"
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

#[tokio::test(flavor = "multi_thread")]
async fn test_beacon_api_get_blobs_by_slot() {
    const GENESIS: u64 = 1_700_000_000;
    const SLOT: u64 = 3600;

    // The long block time keeps the interval miner idle while the test places blocks by hand.
    let node_config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Cancun.into()))
        .with_genesis_timestamp(Some(GENESIS))
        .with_blocktime(Some(Duration::from_secs(SLOT)));
    let (api, handle) = spawn(node_config).await;

    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let from = wallets[0].address();
    let to = wallets[1].address();
    let provider = http_provider(&handle.http_endpoint());
    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();

    // Mine one blob per block: two distinct blobs in slot 2, then the same blob twice in slot 3.
    let timestamps = [
        GENESIS + 2 * SLOT + 5,
        GENESIS + 2 * SLOT + 6,
        GENESIS + 3 * SLOT,
        GENESIS + 3 * SLOT + 1,
    ];
    let mut blobs = Vec::new();
    let mut hashes = Vec::new();
    for (nonce, timestamp) in timestamps.into_iter().enumerate() {
        let sidecar =
            SidecarBuilder::<SimpleCoder>::from_slice(format!("blob {}", nonce.min(2)).as_bytes())
                .build::<BlobTransactionSidecar>()
                .unwrap();
        blobs.push(sidecar.blobs[0]);
        hashes.extend(sidecar.versioned_hashes());
        let tx = TransactionRequest::default()
            .with_from(from)
            .with_to(to)
            .with_nonce(nonce as u64)
            .with_max_fee_per_blob_gas(gas_price + 1)
            .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
            .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
            .with_blob_sidecar_4844(sidecar);
        let pending = provider.send_transaction(WithOtherFields::new(tx)).await.unwrap();
        api.evm_mine(Some(MineOptions::Timestamp(Some(timestamp)))).await.unwrap();
        assert_eq!(pending.get_receipt().await.unwrap().block_number, Some(nonce as u64 + 1));
    }

    let client = reqwest::Client::new();
    let get_blobs = async |path: &str| {
        let url = format!("{}/eth/v1/beacon/blobs/{path}", handle.http_endpoint());
        let response = client.get(&url).send().await.unwrap();
        match response.status() {
            reqwest::StatusCode::OK => {
                Some(response.json::<GetBlobsResponse>().await.unwrap().data)
            }
            reqwest::StatusCode::NOT_FOUND => None,
            status => panic!("unexpected status {status} for {path}"),
        }
    };

    assert!(get_blobs("2").await.as_deref() == Some(&blobs[..2]));
    assert!(
        get_blobs(&format!("2?versioned_hashes={}", hashes[1])).await.as_deref()
            == Some(&blobs[1..2])
    );
    assert!(
        get_blobs(&format!("2?versioned_hashes={},{}", hashes[1], hashes[0])).await.as_deref()
            == Some(&blobs[..2])
    );
    assert!(
        get_blobs(&format!("2?versioned_hashes={}", B256::ZERO)).await.as_deref() == Some(&[][..])
    );
    assert!(get_blobs("3").await.as_deref() == Some(&blobs[2..]));
    // Hashes and tags still address one execution block, not the whole slot.
    assert!(get_blobs(&api.backend.best_hash().to_string()).await.as_deref() == Some(&blobs[3..]));
    assert!(get_blobs("latest").await.as_deref() == Some(&blobs[3..]));
    // Filtered requests return only the requested number of occurrences, across all blocks.
    assert!(
        get_blobs(&format!("3?versioned_hashes={}", hashes[2])).await.as_deref()
            == Some(&blobs[2..3])
    );
    assert!(
        get_blobs(&format!("3?versioned_hashes={},{}", hashes[2], hashes[2])).await.as_deref()
            == Some(&blobs[2..])
    );
    // Slot 0 holds the genesis block, which has no blobs.
    assert!(get_blobs("0").await.as_deref() == Some(&[][..]));
    // Slots without a block are not found, even when a block has that number.
    assert!(get_blobs("1").await.is_none());
    assert!(get_blobs("4").await.is_none());
    assert!(get_blobs(&u64::MAX.to_string()).await.is_none());

    // The slot length stays the configured block time when the mining mode changes.
    api.anvil_set_interval_mining(0).unwrap();
    assert!(get_blobs("2").await.as_deref() == Some(&blobs[..2]));

    // A block mined after moving the time back to slot 1 does not hide the later slots.
    api.evm_set_time(GENESIS + SLOT).unwrap();
    api.evm_mine(None).await.unwrap();
    assert!(get_blobs("1").await.as_deref() == Some(&[][..]));
    assert!(get_blobs("2").await.as_deref() == Some(&blobs[..2]));

    // Loading a legacy dump without block history can leave a gap before its checkpoint.
    // Previously stored canonical blocks must remain visible across that gap.
    let mut state = api.serialized_state(false).await.unwrap();
    state.blocks.clear();
    state.best_block_number = Some(7);
    state.block.as_mut().unwrap().number = U256::from(7);
    api.anvil_load_state(serde_json::to_vec(&state).unwrap().into()).await.unwrap();
    assert!(get_blobs("2").await.as_deref() == Some(&blobs[..2]));
    assert!(get_blobs("3").await.as_deref() == Some(&blobs[2..]));
}
