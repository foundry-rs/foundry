// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! 🚧 \[Unstable\] Property names-related data for this component
//!
//! <div class="stab unstable">
//! 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
//! including in SemVer minor releases. While the serde representation of data structs is guaranteed
//! to be stable, their Rust representation might not be. Use with caution.
//! </div>
//!
//! Read more about data providers: [`icu_provider`]

use alloc::boxed::Box;
use core::cmp::Ordering;

use core::str;

use icu_provider::prelude::*;

use tinystr::TinyStr4;
use zerovec::ule::{UnvalidatedStr, VarULE};
use zerovec::{maps::ZeroMapKV, VarZeroSlice, VarZeroVec, ZeroMap, ZeroVec};

/// This is a property name that can be "loose matched" as according to
/// [PropertyValueAliases.txt](https://www.unicode.org/Public/UCD/latest/ucd/PropertyValueAliases.txt)
///
/// (matched case-insensitively in ASCII, ignoring underscores, whitespace, and hyphens)
///
/// This is expected to be ASCII, but we do not rely on this invariant anywhere except during
/// datagen.
///
/// The Ord impl will sort things using strict equality, but in such a way that all loose-equal items
/// will sort into the same area, such that a map can be searched for both strict and loose equality.
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
///
/// # Examples
///
/// Using a [`NormalizedPropertyNameStr`] as the key of a [`ZeroMap`]:
///
/// ```
/// use icu::properties::provider::names::NormalizedPropertyNameStr;
/// use zerovec::ZeroMap;
///
/// let map: ZeroMap<NormalizedPropertyNameStr, usize> = [
///     (NormalizedPropertyNameStr::from_str("A_BC"), 11),
///     (NormalizedPropertyNameStr::from_str("dEf"), 22),
///     (NormalizedPropertyNameStr::from_str("G_H-I"), 33),
/// ]
/// .into_iter()
/// .collect();
///
/// let key_approx = NormalizedPropertyNameStr::from_str("AB-C");
/// let key_exact = NormalizedPropertyNameStr::from_str("A_BC");
///
/// // Strict lookup:
/// assert_eq!(None, map.get_copied(key_approx));
/// assert_eq!(Some(11), map.get_copied(key_exact));
///
/// // Loose lookup:
/// assert_eq!(Some(11), map.get_copied_by(|u| u.cmp_loose(key_approx)));
/// assert_eq!(Some(11), map.get_copied_by(|u| u.cmp_loose(key_exact)));
/// ```
#[derive(PartialEq, Eq)] // VarULE wants these to be byte equality
#[derive(Debug, VarULE)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(transparent)]
pub struct NormalizedPropertyNameStr(UnvalidatedStr);

/// This impl requires enabling the optional `serde` Cargo feature of the `icu::properties` crate
#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Box<NormalizedPropertyNameStr> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        <Box<UnvalidatedStr>>::deserialize(deserializer).map(NormalizedPropertyNameStr::cast_box)
    }
}

/// This impl requires enabling the optional `serde` Cargo feature of the `icu::properties` crate
#[cfg(feature = "serde")]
impl<'de, 'a> serde::Deserialize<'de> for &'a NormalizedPropertyNameStr
where
    'de: 'a,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        <&UnvalidatedStr>::deserialize(deserializer).map(NormalizedPropertyNameStr::cast_ref)
    }
}

impl<'a> ZeroMapKV<'a> for NormalizedPropertyNameStr {
    type Container = VarZeroVec<'a, NormalizedPropertyNameStr>;
    type Slice = VarZeroSlice<NormalizedPropertyNameStr>;
    type GetType = NormalizedPropertyNameStr;
    type OwnedType = Box<NormalizedPropertyNameStr>;
}

/// The Ord/PartialOrd impl will sort things using strict equality, but in such a way that all loose-equal items
/// will sort into the same area, such that a map can be searched for both strict and loose equality.
impl PartialOrd for NormalizedPropertyNameStr {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Normalize a character based on the "loose matching" described in PropertyValueAliases.txt,
/// returning `None` for skippable characters
///
/// ICU has [code for this][1] (and [during property lookup][2]) which we emulate.
/// In particular, ICU only does normalization within ASCII, which makes sense since character names
/// seem to be only ASCII.
///
/// [1]: https://github.com/unicode-org/icu/blob/288c4c7555915ce7b1fb675d94ddd495058fc039/icu4c/source/common/propname.cpp#L35
/// [2]: https://github.com/unicode-org/icu/blob/288c4c7555915ce7b1fb675d94ddd495058fc039/icu4c/source/common/propname.cpp#L226-L230
fn normalize_char(ch: u8) -> Option<u8> {
    match ch {
        // all ascii whitespace
        ch if ch.is_ascii_whitespace() => None,
        // underscores, hyphens, and the vertical tab character
        // not covered by is_ascii_whitespace()
        b'_' | b'-' | 0x0B => None,
        // ignore case by lowercasing
        ch => Some(ch.to_ascii_lowercase()),
    }
}

/// The Ord impl will sort things using strict equality, but in such a way that all loose-equal items
/// will sort into the same area, such that a map can be searched for both strict and loose equality.
impl Ord for NormalizedPropertyNameStr {
    fn cmp(&self, other: &Self) -> Ordering {
        let cmp = self.cmp_loose(other);
        // When loose equality holds, fall back to strict equality
        if cmp == Ordering::Equal {
            self.0.cmp(&other.0)
        } else {
            cmp
        }
    }
}

impl NormalizedPropertyNameStr {
    /// Perform the loose comparison as defined in [`NormalizedPropertyNameStr`].
    pub fn cmp_loose(&self, other: &Self) -> Ordering {
        let self_iter = self.0.iter().copied().filter_map(normalize_char);
        let other_iter = other.0.iter().copied().filter_map(normalize_char);
        self_iter.cmp(other_iter)
    }

    /// Convert a string reference to a [`NormalizedPropertyNameStr`].
    pub const fn from_str(s: &str) -> &Self {
        Self::cast_ref(UnvalidatedStr::from_str(s))
    }

    /// Convert a [`UnvalidatedStr`] reference to a [`NormalizedPropertyNameStr`] reference.
    pub const fn cast_ref(value: &UnvalidatedStr) -> &Self {
        // Safety: repr(transparent)
        unsafe { core::mem::transmute(value) }
    }

    /// Convert a [`UnvalidatedStr`] box to a [`NormalizedPropertyNameStr`] box.
    pub const fn cast_box(value: Box<UnvalidatedStr>) -> Box<Self> {
        // Safety: repr(transparent)
        unsafe { core::mem::transmute(value) }
    }

    /// Get a [`NormalizedPropertyNameStr`] box from a byte slice.
    pub fn boxed_from_bytes(b: &[u8]) -> Box<Self> {
        Self::cast_box(UnvalidatedStr::from_boxed_bytes(b.into()))
    }
}

/// A set of characters and strings which share a particular property value.
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
#[derive(Debug, Clone, PartialEq)]
#[icu_provider::data_struct(marker(
    GeneralCategoryMaskNameToValueV1Marker,
    "propnames/from/gcm@1",
    singleton,
))]
#[cfg_attr(
    feature = "datagen", 
    derive(serde::Serialize, databake::Bake),
    databake(path = icu_properties::provider::names),
)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[yoke(prove_covariance_manually)]
pub struct PropertyValueNameToEnumMapV1<'data> {
    /// A map from names to their value discriminant
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub map: ZeroMap<'data, NormalizedPropertyNameStr, u16>,
}

/// A mapping of property values to their names. A single instance of this map will only cover
/// either long or short names, determined whilst loading data.
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
#[derive(Debug, Clone, PartialEq)]
#[icu_provider::data_struct]
#[cfg_attr(
    feature = "datagen", 
    derive(serde::Serialize, databake::Bake),
    databake(path = icu_properties::provider::names),
)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[yoke(prove_covariance_manually)]
pub struct PropertyEnumToValueNameSparseMapV1<'data> {
    /// A map from the value discriminant to the names
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub map: ZeroMap<'data, u16, str>,
}

/// A mapping of property values to their names. A single instance of this map will only cover
/// either long or short names, determined whilst loading data.
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
#[derive(Debug, Clone, PartialEq)]
#[icu_provider::data_struct]
#[cfg_attr(
    feature = "datagen", 
    derive(serde::Serialize, databake::Bake),
    databake(path = icu_properties::provider::names),
)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[yoke(prove_covariance_manually)]
pub struct PropertyEnumToValueNameLinearMapV1<'data> {
    /// A map from the value discriminant (the index) to the names, for mostly
    /// contiguous data. Empty strings count as missing.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub map: VarZeroVec<'data, str>,
}

/// A mapping of property values to their names. A single instance of this map will only cover
/// either long or short names, determined whilst loading data.
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
#[derive(Debug, Clone, PartialEq)]
#[icu_provider::data_struct]
#[cfg_attr(
    feature = "datagen", 
    derive(serde::Serialize, databake::Bake),
    databake(path = icu_properties::provider::names),
)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[yoke(prove_covariance_manually)]
pub struct PropertyEnumToValueNameLinearTiny4MapV1<'data> {
    /// A map from the value discriminant (the index) to the names, for mostly
    /// contiguous data. Empty strings count as missing.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub map: ZeroVec<'data, TinyStr4>,
}
