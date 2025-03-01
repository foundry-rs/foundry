// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use super::*;
use icu_provider::prelude::*;
use zerovec::ZeroVec;

#[icu_provider::data_struct(marker(
    ScriptDirectionV1Marker,
    "locid_transform/script_dir@1",
    singleton
))]
#[derive(Debug, PartialEq, Clone)]
#[cfg_attr(
    feature = "datagen",
    derive(serde::Serialize, databake::Bake),
    databake(path = icu_locid_transform::provider),
)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
/// This directionality data is used to determine the script directionality of a locale.
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
#[yoke(prove_covariance_manually)]
pub struct ScriptDirectionV1<'data> {
    /// Scripts in right-to-left direction.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub rtl: ZeroVec<'data, UnvalidatedScript>,
    /// Scripts in left-to-right direction.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub ltr: ZeroVec<'data, UnvalidatedScript>,
}
