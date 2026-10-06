//! Beacon Node REST API implementation for Anvil.

use axum::{Router, routing::get};

use crate::eth::EthApi;
use alloy_network::Network;

mod error;
mod handlers;
mod utils;

/// Configures an [`axum::Router`] that handles Beacon REST API calls.
pub fn router<N: Network>(api: EthApi<N>) -> Router {
    Router::new()
        .route(
            "/eth/v1/beacon/blob_sidecars/{block_id}",
            get(handlers::handle_get_blob_sidecars::<N>),
        )
        .route("/eth/v1/beacon/blobs/{block_id}", get(handlers::handle_get_blobs::<N>))
        .route("/eth/v1/beacon/genesis", get(handlers::handle_get_genesis::<N>))
        .with_state(api)
}
