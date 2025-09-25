use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};

use crate::{BeaconApiHandler, BeaconRequest, BeaconResponse};

pub fn beacon_router<Beacon: BeaconApiHandler>(beacon: Beacon) -> Router {
    Router::new()
        .route("/v1/beacon/blob_sidecars/{block_id}", get(get_blob_sidecars_by_block_id::<Beacon>))
        .with_state(beacon)
}

async fn get_blob_sidecars_by_block_id<Beacon: BeaconApiHandler>(
    Path(block_id): Path<String>,
    State(beacon): State<Beacon>,
) -> Json<BeaconResponse> {
    Json(beacon.call(BeaconRequest::GetBlobSidecarsByBlockId(block_id)))
}
