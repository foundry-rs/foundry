/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Behavior version of the client

/// Behavior version of the client
///
/// Over time, new best-practice behaviors are introduced. However, these behaviors might not be
/// backwards compatible. For example, a change which introduces new default timeouts or a new
/// retry-mode for all operations might be the ideal behavior but could break existing applications.
#[derive(Copy, Clone, PartialEq)]
pub struct BehaviorVersion {
    inner: Inner,
}

#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
enum Inner {
    // IMPORTANT: Order matters here for the `Ord` derive. Newer versions go to the bottom.
    V2023_11_09,
    V2024_03_28,
}

impl BehaviorVersion {
    /// This method will always return the latest major version.
    ///
    /// This is the recommend choice for customers who aren't reliant on extremely specific behavior
    /// characteristics. For example, if you are writing a CLI app, the latest behavior major
    /// version is probably the best setting for you.
    ///
    /// If, however, you're writing a service that is very latency sensitive, or that has written
    /// code to tune Rust SDK behaviors, consider pinning to a specific major version.
    ///
    /// The latest version is currently [`BehaviorVersion::v2024_03_28`]
    pub fn latest() -> Self {
        Self::v2024_03_28()
    }

    /// Behavior version for March 28th, 2024.
    ///
    /// This version enables stalled stream protection for uploads (request bodies) by default.
    ///
    /// When a new behavior major version is released, this method will be deprecated.
    pub fn v2024_03_28() -> Self {
        Self {
            inner: Inner::V2024_03_28,
        }
    }

    /// Behavior version for November 9th, 2023.
    #[deprecated(
        since = "1.4.0",
        note = "Superceded by v2024_03_28, which enabled stalled stream protection for uploads (request bodies) by default."
    )]
    pub fn v2023_11_09() -> Self {
        Self {
            inner: Inner::V2023_11_09,
        }
    }

    /// True if this version is newer or equal to the given `other` version.
    pub fn is_at_least(&self, other: BehaviorVersion) -> bool {
        self.inner >= other.inner
    }
}

impl std::fmt::Debug for BehaviorVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("BehaviorVersion").field(&self.inner).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(deprecated)]
    fn version_comparison() {
        assert!(BehaviorVersion::latest() == BehaviorVersion::latest());
        assert!(BehaviorVersion::v2023_11_09() == BehaviorVersion::v2023_11_09());
        assert!(BehaviorVersion::v2024_03_28() != BehaviorVersion::v2023_11_09());
        assert!(BehaviorVersion::latest().is_at_least(BehaviorVersion::latest()));
        assert!(BehaviorVersion::latest().is_at_least(BehaviorVersion::v2023_11_09()));
        assert!(BehaviorVersion::latest().is_at_least(BehaviorVersion::v2024_03_28()));
        assert!(!BehaviorVersion::v2023_11_09().is_at_least(BehaviorVersion::v2024_03_28()));
        assert!(Inner::V2024_03_28 > Inner::V2023_11_09);
        assert!(Inner::V2023_11_09 < Inner::V2024_03_28);
    }
}
