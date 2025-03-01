/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

/// Smithy retry classifiers.
pub mod classifiers;

/// Smithy retry strategies.
pub mod strategy;

mod client_rate_limiter;
mod token_bucket;

use aws_smithy_types::config_bag::{Storable, StoreReplace};
use std::fmt;

pub use client_rate_limiter::ClientRateLimiter;
pub use token_bucket::TokenBucket;

pub use client_rate_limiter::ClientRateLimiterPartition;
use std::borrow::Cow;

/// Represents the retry partition, e.g. an endpoint, a region
#[non_exhaustive]
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct RetryPartition {
    name: Cow<'static, str>,
}

impl RetryPartition {
    /// Creates a new `RetryPartition` from the given `name`.
    pub fn new(name: impl Into<Cow<'static, str>>) -> Self {
        Self { name: name.into() }
    }
}

impl fmt::Display for RetryPartition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

impl Storable for RetryPartition {
    type Storer = StoreReplace<RetryPartition>;
}
