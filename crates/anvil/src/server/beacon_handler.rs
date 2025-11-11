use crate::eth::{
    EthApi,
    beacon::{BeaconError, BeaconResponse, GenesisDetails},
};
use alloy_eips::BlockId;
use alloy_primitives::{B256, aliases::B32};
use alloy_rpc_types_beacon::{
    header::Header,
    sidecar::{BlobData, GetBlobsResponse},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
};
use hyper::StatusCode;
use std::{collections::HashMap, str::FromStr as _};

/// Handles incoming Beacon API requests for blob sidecars
///
/// GET /eth/v1/beacon/blob_sidecars/{block_id}
pub async fn handle_get_blob_sidecars(
    State(api): State<EthApi>,
    Path(block_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    // Parse block_id from path parameter
    let Ok(block_id) = BlockId::from_str(&block_id) else {
        return BeaconError::invalid_block_id(block_id).into_response();
    };

    // Parse indices from query parameters
    // Supports both comma-separated (?indices=1,2,3) and repeated parameters (?indices=1&indices=2)
    let indices: Vec<u64> = params
        .get("indices")
        .map(|s| s.split(',').filter_map(|idx| idx.trim().parse::<u64>().ok()).collect())
        .unwrap_or_default();

    // Get the blob sidecars using existing EthApi logic
    match api.anvil_get_blob_sidecars_by_block_id(block_id) {
        Ok(Some(sidecar)) => BeaconResponse::with_flags(
            sidecar
                .into_iter()
                .filter(|blob_item| indices.is_empty() || indices.contains(&blob_item.index))
                .map(|blob_item| BlobData {
                    index: blob_item.index,
                    blob: blob_item.blob,
                    kzg_commitment: blob_item.kzg_commitment,
                    kzg_proof: blob_item.kzg_proof,
                    signed_block_header: Header::default(), // Not available in Anvil
                    kzg_commitment_inclusion_proof: vec![], // Not available in Anvil
                })
                .collect::<Vec<_>>(),
            false, // Not available in Anvil
            false, // Not available in Anvil
        )
        .into_response(),
        Ok(None) => BeaconError::block_not_found().into_response(),
        Err(_) => BeaconError::internal_error().into_response(),
    }
}

/// Handles incoming Beacon API requests for blobs
///
/// GET /eth/v1/beacon/blobs/{block_id}
pub async fn handle_get_blobs(
    State(api): State<EthApi>,
    Path(block_id): Path<String>,
    Query(versioned_hashes): Query<HashMap<String, String>>,
) -> Response {
    // Parse block_id from path parameter
    let Ok(block_id) = BlockId::from_str(&block_id) else {
        return BeaconError::invalid_block_id(block_id).into_response();
    };

    // Parse indices from query parameters
    // Supports both comma-separated (?indices=1,2,3) and repeated parameters (?indices=1&indices=2)
    let versioned_hashes: Vec<B256> = versioned_hashes
        .get("versioned_hashes")
        .map(|s| s.split(',').filter_map(|hash| B256::from_str(hash.trim()).ok()).collect())
        .unwrap_or_default();

    // Get the blob sidecars using existing EthApi logic
    match api.anvil_get_blobs_by_block_id(block_id, versioned_hashes) {
        Ok(Some(blobs)) => (
            StatusCode::OK,
            Json(GetBlobsResponse { execution_optimistic: false, finalized: false, data: blobs }),
        )
            .into_response(),
        Ok(None) => BeaconError::block_not_found().into_response(),
        Err(_) => BeaconError::internal_error().into_response(),
    }
}

/// Handles incoming Beacon API requests for genesis details
///
/// Only returns the `genesis_time`, other fields are set to zero.
///
/// GET /eth/v1/beacon/genesis
pub async fn handle_get_genesis(State(api): State<EthApi>) -> Response {
    match api.anvil_get_genesis_time() {
        Ok(genesis_time) => BeaconResponse::new(GenesisDetails {
            genesis_time,
            genesis_validators_root: B256::ZERO,
            genesis_fork_version: B32::ZERO,
        })
        .into_response(),
        Err(_) => BeaconError::internal_error().into_response(),
    }
}
