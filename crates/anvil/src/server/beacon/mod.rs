//! Beacon Node REST API implementation for Anvil.

use axum::{Router, routing::get};

use crate::eth::EthApi;

mod error;
mod handlers;
mod utils;

/// Configures an [`axum::Router`] that handles Beacon REST API calls.
pub fn router(api: EthApi) -> Router {
    Router::new()
        .route("/eth/v1/beacon/blob_sidecars/{block_id}", get(handlers::handle_get_blob_sidecars))
        .route("/eth/v1/beacon/blobs/{block_id}", get(handlers::handle_get_blobs))
        .route("/eth/v1/beacon/genesis", get(handlers::handle_get_genesis))
        .with_state(api)
}
