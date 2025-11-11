//! Beacon API response types

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

/// Generic Beacon API response wrapper
///
/// This follows the beacon chain API specification where responses include
/// the actual data plus metadata about execution optimism and finalization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeaconResponse<T> {
    /// The response data
    pub data: T,
    /// Whether the response references an unverified execution payload
    ///
    /// For Anvil, this is always `false` since there's no real consensus layer
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_optimistic: Option<bool>,
    /// Whether the response references finalized history
    ///
    /// For Anvil, this is always `false` since there's no real consensus layer
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalized: Option<bool>,
}

impl<T> BeaconResponse<T> {
    /// Creates a new beacon response with the given data
    ///
    /// For Anvil context, `execution_optimistic` and `finalized` are always `false`
    pub fn new(data: T) -> Self {
        Self { data, execution_optimistic: None, finalized: None }
    }

    /// Creates a beacon response with custom execution_optimistic and finalized flags
    pub fn with_flags(data: T, execution_optimistic: bool, finalized: bool) -> Self {
        Self { data, execution_optimistic: Some(execution_optimistic), finalized: Some(finalized) }
    }
}

impl<T: Serialize> IntoResponse for BeaconResponse<T> {
    fn into_response(self) -> Response {
        (StatusCode::OK, Json(self)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_beacon_response_defaults() {
        let response = BeaconResponse::new("test data");
        assert_eq!(response.data, "test data");
        assert!(response.execution_optimistic.is_none());
        assert!(response.finalized.is_none());
    }

    #[test]
    fn test_beacon_response_serialization() {
        let response = BeaconResponse::with_flags(vec![1, 2, 3], false, false);
        let json = serde_json::to_value(&response).unwrap();

        assert_eq!(json["data"], serde_json::json!([1, 2, 3]));
        assert_eq!(json["execution_optimistic"], false);
        assert_eq!(json["finalized"], false);
    }
}
