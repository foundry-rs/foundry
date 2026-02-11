//! Shared utilities for brutalizer mutators.
//!
//! Provides deterministic hashing and helper functions used by the value,
//! memory, and FMP brutalizer mutators.

use solar::ast::{FunctionKind, Span, Visibility};

/// Applies the splitmix64 finalizer to produce a well-distributed 64-bit hash.
///
/// The constants are from splitmix64 (part of the SplitMix PRNG family):
/// - `0xbf58476d1ce4e5b9`: first finalizer multiplier
/// - `0x94d049bb133111eb`: second finalizer multiplier
/// - `>> 30` / `>> 27` / `>> 31`: avalanche shifts to propagate bits
pub fn splitmix64(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58476d1ce4e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d049bb133111eb);
    x ^= x >> 31;
    x
}

/// Derives a deterministic seed from a span's byte offsets.
///
/// Mixes lo and hi positions using the golden ratio constant (`0x9e3779b97f4a7c15`)
/// to produce a unique seed per source location.
pub fn span_seed(span: Span) -> u64 {
    let lo = span.lo().0 as u64;
    let hi = span.hi().0 as u64;
    splitmix64(lo.wrapping_mul(0x9e3779b97f4a7c15) ^ hi.wrapping_mul(0xff51afd7ed558ccd))
}

/// Returns true if the function is eligible for entry-point mutations
/// (memory brutalization and FMP misalignment).
///
/// Only regular `external` functions qualify. Public functions are excluded
/// because they can be called internally (JUMP), sharing the caller's memory.
/// Constructors, fallbacks, receives, and modifiers are also excluded.
pub fn is_eligible_function(visibility: Option<Visibility>, kind: Option<FunctionKind>) -> bool {
    if let Some(kind) = kind
        && !matches!(kind, FunctionKind::Function)
    {
        return false;
    }

    matches!(visibility, Some(Visibility::External))
}

pub fn extract_span_text(source: &str, span: Span) -> String {
    let lo = span.lo().0 as usize;
    let hi = span.hi().0 as usize;
    source.get(lo..hi).map(|s| s.to_string()).unwrap_or_default()
}
