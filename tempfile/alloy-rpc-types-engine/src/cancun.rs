//! Contains types related to the Cancun hardfork that will be used by RPC to communicate with the
//! beacon consensus engine.

use alloc::vec::Vec;
use alloy_primitives::B256;

/// Fields introduced in `engine_newPayloadV3` that are not present in the `ExecutionPayload` RPC
/// object.
///
/// See also:
/// <https://github.com/ethereum/execution-apis/blob/fe8e13c288c592ec154ce25c534e26cb7ce0530d/src/engine/cancun.md#request>
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CancunPayloadFields {
    /// The parent beacon block root.
    pub parent_beacon_block_root: B256,

    /// The expected blob versioned hashes.
    pub versioned_hashes: Vec<B256>,
}

impl CancunPayloadFields {
    /// Returns a new [`CancunPayloadFields`] instance.
    pub const fn new(parent_beacon_block_root: B256, versioned_hashes: Vec<B256>) -> Self {
        Self { parent_beacon_block_root, versioned_hashes }
    }
}

/// A container type for [CancunPayloadFields] that may or may not be present.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MaybeCancunPayloadFields {
    fields: Option<CancunPayloadFields>,
}

impl MaybeCancunPayloadFields {
    /// Returns a new `MaybeCancunPayloadFields` with no cancun fields.
    pub const fn none() -> Self {
        Self { fields: None }
    }

    /// Returns a new `MaybeCancunPayloadFields` with the given cancun fields.
    pub fn into_inner(self) -> Option<CancunPayloadFields> {
        self.fields
    }

    /// Returns the parent beacon block root, if any.
    pub fn parent_beacon_block_root(&self) -> Option<B256> {
        self.fields.as_ref().map(|fields| fields.parent_beacon_block_root)
    }

    /// Returns the blob versioned hashes, if any.
    pub fn versioned_hashes(&self) -> Option<&Vec<B256>> {
        self.fields.as_ref().map(|fields| &fields.versioned_hashes)
    }

    /// Returns a reference to the inner fields.
    pub const fn as_ref(&self) -> Option<&CancunPayloadFields> {
        self.fields.as_ref()
    }
}

impl From<CancunPayloadFields> for MaybeCancunPayloadFields {
    #[inline]
    fn from(fields: CancunPayloadFields) -> Self {
        Self { fields: Some(fields) }
    }
}

impl From<Option<CancunPayloadFields>> for MaybeCancunPayloadFields {
    #[inline]
    fn from(fields: Option<CancunPayloadFields>) -> Self {
        Self { fields }
    }
}
