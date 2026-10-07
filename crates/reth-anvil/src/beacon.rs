//! The Beacon API routes anvil serves next to the JSON-RPC methods: the blobs of a block and
//! the genesis time, under `/eth/v1/beacon/`.

use crate::{eth_api::decode_blob, server::SharedModule};
use alloy_consensus::{Blob, Transaction as _};
use alloy_eips::BlockId;
use alloy_primitives::{B256, aliases::B32};
use alloy_rpc_types_beacon::{
    genesis::{GenesisData, GenesisResponse},
    sidecar::GetBlobsResponse,
};
use alloy_rpc_types_eth::Block;
use hyper::{
    Method, StatusCode,
    header::{ACCEPT, CONTENT_TYPE},
};
use jsonrpsee::{
    RpcModule,
    server::{HttpBody, HttpRequest, HttpResponse},
};
use serde::Serialize;
use ssz::Encode;
use std::{
    pin::Pin,
    str::FromStr,
    task::{Context, Poll},
};
use tower::{Layer, Service};

/// The path prefix of the Beacon API routes.
const BEACON_PREFIX: &str = "/eth/v1/beacon/";

/// Serves the Beacon API routes in front of the JSON-RPC server.
#[derive(Clone)]
pub struct BeaconLayer {
    module: SharedModule,
}

impl BeaconLayer {
    /// Creates the layer over the node's RPC module.
    pub const fn new(module: SharedModule) -> Self {
        Self { module }
    }
}

impl<S> Layer<S> for BeaconLayer {
    type Service = BeaconService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        BeaconService { inner, module: self.module.clone() }
    }
}

/// The service [`BeaconLayer`] wraps the JSON-RPC server with.
#[derive(Clone)]
pub struct BeaconService<S> {
    inner: S,
    module: SharedModule,
}

impl<S, B> Service<HttpRequest<B>> for BeaconService<S>
where
    S: Service<HttpRequest<B>, Response = HttpResponse>,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    B: Send + 'static,
{
    type Response = HttpResponse;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: HttpRequest<B>) -> Self::Future {
        if request.method() == Method::GET
            && let Some(route) = request.uri().path().strip_prefix(BEACON_PREFIX)
        {
            let module = self.module.read().clone();
            let route = route.to_string();
            let query = request.uri().query().map(str::to_string);
            let accept = request
                .headers()
                .get(ACCEPT)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            return Box::pin(async move { Ok(respond(&module, &route, query, accept).await) });
        }
        Box::pin(self.inner.call(request))
    }
}

/// Answers a Beacon API route.
async fn respond(
    module: &RpcModule<()>,
    route: &str,
    query: Option<String>,
    accept: Option<String>,
) -> HttpResponse {
    if route.starts_with("blob_sidecars/") {
        return error(
            StatusCode::GONE,
            "This endpoint is deprecated. Use `GET /eth/v1/beacon/blobs/{block_id}` instead.",
        );
    }
    if let Some(block_id) = route.strip_prefix("blobs/") {
        return blobs(module, block_id, query.as_deref(), accept.as_deref()).await;
    }
    if route == "genesis" {
        return genesis(module).await;
    }
    error(StatusCode::NOT_FOUND, "Not found")
}

/// `GET /eth/v1/beacon/blobs/{block_id}`: the blobs of the block, optionally filtered by
/// `versioned_hashes`.
async fn blobs(
    module: &RpcModule<()>,
    block_id: &str,
    query: Option<&str>,
    accept: Option<&str>,
) -> HttpResponse {
    let Ok(block_id) = BlockId::from_str(block_id) else {
        return error(StatusCode::BAD_REQUEST, &format!("Invalid block ID: {block_id}"));
    };
    let mut wanted = Vec::new();
    for (key, value) in
        query.into_iter().flat_map(|query| query.split('&')).filter_map(|pair| pair.split_once('='))
    {
        if key != "versioned_hashes" {
            continue;
        }
        for hash in value.split(',').map(str::trim).filter(|hash| !hash.is_empty()) {
            match B256::from_str(hash) {
                Ok(hash) => wanted.push(hash),
                Err(_) => {
                    return error(
                        StatusCode::BAD_REQUEST,
                        &format!("Invalid versioned hash: {hash}"),
                    );
                }
            }
        }
    }
    // Anvil serves mined blocks only.
    let block = if block_id.is_pending() {
        Ok(None)
    } else {
        match block_id {
            BlockId::Hash(hash) => {
                module.call::<_, Option<Block>>("eth_getBlockByHash", (hash.block_hash, true)).await
            }
            BlockId::Number(number) => {
                module.call::<_, Option<Block>>("eth_getBlockByNumber", (number, true)).await
            }
        }
    };
    let block = match block {
        Ok(Some(block)) => block,
        Ok(None) => return error(StatusCode::NOT_FOUND, "Block not found"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
    };
    let mut hashes: Vec<B256> = Vec::new();
    for tx in block.transactions.txns() {
        if let Some(tx_hashes) = tx.blob_versioned_hashes() {
            hashes
                .extend(tx_hashes.iter().filter(|hash| wanted.is_empty() || wanted.contains(hash)));
        }
    }
    let mut blobs: Vec<Blob> = Vec::new();
    for hash in hashes {
        match module.call::<_, Option<String>>("anvil_getBlobByHash", (hash,)).await {
            Ok(Some(blob)) => match decode_blob(&blob) {
                Ok(blob) => blobs.push(*blob),
                Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
            },
            Ok(None) => {}
            Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
        }
    }
    if prefers_ssz(accept) {
        return response(StatusCode::OK, "application/octet-stream", blobs.as_ssz_bytes());
    }
    json(
        StatusCode::OK,
        &GetBlobsResponse { execution_optimistic: false, finalized: false, data: blobs },
    )
}

/// `GET /eth/v1/beacon/genesis`: the genesis time; the other fields are zero.
async fn genesis(module: &RpcModule<()>) -> HttpResponse {
    match module.call::<_, u64>("anvil_getGenesisTime", [(); 0]).await {
        Ok(genesis_time) => json(
            StatusCode::OK,
            &GenesisResponse {
                data: GenesisData {
                    genesis_time,
                    genesis_validators_root: B256::ZERO,
                    genesis_fork_version: B32::ZERO,
                },
            },
        ),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
    }
}

/// Returns whether the `Accept` header prefers SSZ (`application/octet-stream`) over JSON.
fn prefers_ssz(accept: Option<&str>) -> bool {
    let Some(accept) = accept else { return false };
    let (mut octet_stream, mut json) = (0.0f32, 0.0f32);
    for media_type in accept.split(',').map(str::trim) {
        let quality = media_type
            .split(';')
            .map(str::trim)
            .find_map(|param| param.strip_prefix("q=")?.parse::<f32>().ok())
            .unwrap_or(1.0);
        if media_type.starts_with("application/octet-stream") {
            octet_stream = quality;
        } else if media_type.starts_with("application/json") {
            json = quality;
        }
    }
    octet_stream > json
}

/// A JSON response.
fn json<T: Serialize>(status: StatusCode, value: &T) -> HttpResponse {
    match serde_json::to_vec(value) {
        Ok(body) => response(status, "application/json", body),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
    }
}

/// An error response in the Beacon API's `{"code", "message"}` form.
fn error(status: StatusCode, message: &str) -> HttpResponse {
    let body = serde_json::json!({ "code": status.as_u16(), "message": message });
    response(status, "application/json", body.to_string().into_bytes())
}

/// A response with the given status, content type, and body.
fn response(status: StatusCode, content_type: &str, body: Vec<u8>) -> HttpResponse {
    HttpResponse::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .body(HttpBody::from(body))
        .expect("a valid response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssz_preference() {
        assert!(!prefers_ssz(None));
        assert!(!prefers_ssz(Some("application/json")));
        assert!(prefers_ssz(Some("application/octet-stream")));
        assert!(prefers_ssz(Some("application/octet-stream;q=1.0,application/json;q=0.9")));
        assert!(!prefers_ssz(Some("application/json;q=1.0,application/octet-stream;q=0.9")));
        assert!(!prefers_ssz(Some("application/octet-stream;q=0.5,application/json;q=0.5")));
        assert!(prefers_ssz(Some(
            "text/html;q=0.9, application/octet-stream;q=1.0, application/json;q=0.8"
        )));
        assert!(prefers_ssz(Some("application/octet-stream, application/json;q=0.9")));
    }
}
