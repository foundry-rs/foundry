//! Contains types related to the Prague hardfork that will be used by RPC to communicate with the
//! beacon consensus engine.

use alloy_eips::eip7685::{Requests, RequestsOrHash};
use alloy_primitives::B256;

/// Fields introduced in `engine_newPayloadV4` that are not present in the `ExecutionPayload` RPC
/// object.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PraguePayloadFields {
    /// EIP-7685 requests.
    pub requests: RequestsOrHash,
}

impl PraguePayloadFields {
    /// Returns a new [`PraguePayloadFields`] instance.
    pub fn new(requests: impl Into<RequestsOrHash>) -> Self {
        Self { requests: requests.into() }
    }
}

/// A container type for [PraguePayloadFields] that may or may not be present.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MaybePraguePayloadFields {
    fields: Option<PraguePayloadFields>,
}

impl MaybePraguePayloadFields {
    /// Returns a new [`MaybePraguePayloadFields`] with no prague fields.
    pub const fn none() -> Self {
        Self { fields: None }
    }

    /// Returns a new [`MaybePraguePayloadFields`] with the given prague fields.
    pub fn into_inner(self) -> Option<PraguePayloadFields> {
        self.fields
    }

    /// Returns the requests, if any.
    pub fn requests(&self) -> Option<&Requests> {
        self.fields.as_ref().and_then(|fields| fields.requests.requests())
    }

    /// Calculates or retrieves the requests hash.
    ///
    /// - If the `prague` field contains a list of requests, it calculates the requests hash
    ///   dynamically.
    /// - If it contains a precomputed hash (used for testing), it returns that hash directly.
    pub fn requests_hash(&self) -> Option<B256> {
        self.fields.as_ref().map(|fields| fields.requests.requests_hash())
    }

    /// Returns a reference to the inner fields.
    pub const fn as_ref(&self) -> Option<&PraguePayloadFields> {
        self.fields.as_ref()
    }
}

impl From<PraguePayloadFields> for MaybePraguePayloadFields {
    #[inline]
    fn from(fields: PraguePayloadFields) -> Self {
        Self { fields: Some(fields) }
    }
}

impl From<Option<PraguePayloadFields>> for MaybePraguePayloadFields {
    #[inline]
    fn from(fields: Option<PraguePayloadFields>) -> Self {
        Self { fields }
    }
}
