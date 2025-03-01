//! Versions for the engine api.

/// The version of the engine api.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u32)]
pub enum ForkchoiceUpdateVersion {
    /// Version 1 of the engine api.
    V1 = 1,
    /// Version 2 of the engine api.
    V2 = 2,
    /// Version 3 of the engine api.
    V3 = 3,
    /// Version 4 of the engine api.
    V4 = 4,
}
