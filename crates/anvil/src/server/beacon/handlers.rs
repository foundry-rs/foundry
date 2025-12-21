use super::{error::BeaconError, utils::must_be_ssz};
use crate::eth::EthApi;
use alloy_eips::BlockId;
use alloy_primitives::{B256, aliases::B32};
use alloy_rpc_types_beacon::{
    genesis::{GenesisData, GenesisResponse},
    sidecar::GetBlobsResponse,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use ssz::Encode;
use std::{collections::HashMap, str::FromStr as _};

/// Handles incoming Beacon API requests for blob sidecars
///
/// This endpoint is deprecated. Use `GET /eth/v1/beacon/blobs/{block_id}` instead.
///
/// GET /eth/v1/beacon/blob_sidecars/{block_id}
pub async fn handle_get_blob_sidecars(
    State(_api): State<EthApi>,
    Path(_block_id): Path<String>,
    Query(_params): Query<HashMap<String, String>>,
) -> Response {
    BeaconError::deprecated_endpoint_with_hint("Use `GET /eth/v1/beacon/blobs/{block_id}` instead.")
        .into_response()
}

/// Handles incoming Beacon API requests for blobs
///
/// GET /eth/v1/beacon/blobs/{block_id}
pub async fn handle_get_blobs(
    headers: HeaderMap,
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
        Ok(Some(blobs)) => {
            if must_be_ssz(&headers) {
                blobs.as_ssz_bytes().into_response()
            } else {
                Json(GetBlobsResponse {
                    execution_optimistic: false,
                    finalized: false,
                    data: blobs,
                })
                .into_response()
            }
        }
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
        Ok(genesis_time) => Json(GenesisResponse {
            data: GenesisData {
                genesis_time,
                genesis_validators_root: B256::ZERO,
                genesis_fork_version: B32::ZERO,
            },
        })
        .into_response(),
        Err(_) => BeaconError::internal_error().into_response(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn header_map_with_accept(accept: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(axum::http::header::ACCEPT, HeaderValue::from_str(accept).unwrap());
        headers
    }

    #[test]
    fn test_must_be_ssz() {
        let test_cases = vec![
            (None, false, "no Accept header"),
            (Some("application/json"), false, "JSON only"),
            (Some("application/octet-stream"), true, "octet-stream only"),
            (Some("application/octet-stream;q=1.0,application/json;q=0.9"), true, "SSZ preferred"),
            (
                Some("application/json;q=1.0,application/octet-stream;q=0.9"),
                false,
                "JSON preferred",
            ),
            (Some("application/octet-stream;q=0.5,application/json;q=0.5"), false, "equal quality"),
            (
                Some("text/html;q=0.9, application/octet-stream;q=1.0, application/json;q=0.8"),
                true,
                "multiple types",
            ),
            (
                Some("application/octet-stream ; q=1.0 , application/json ; q=0.9"),
                true,
                "whitespace handling",
            ),
            (Some("application/octet-stream, application/json;q=0.9"), true, "default quality"),
        ];

        for (accept_header, expected, description) in test_cases {
            let headers = match accept_header {
                None => HeaderMap::new(),
                Some(header) => header_map_with_accept(header),
            };
            assert_eq!(
                must_be_ssz(&headers),
                expected,
                "Test case '{}' failed: expected {}, got {}",
                description,
                expected,
                !expected
            );
        }
    }
}
