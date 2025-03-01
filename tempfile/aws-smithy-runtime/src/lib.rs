/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

/* Automatically managed default lints */
#![cfg_attr(docsrs, feature(doc_auto_cfg))]
/* End of automatically managed default lints */
//! Runtime support logic and types for smithy-rs generated code.
//!
//! # Crate Features
//!
//! - `http-auth`: Enables auth scheme and identity resolver implementations for HTTP API Key,
//!   Basic Auth, Bearer Token, and Digest Auth.
//! - `test-util`: Enables utilities for unit tests. DO NOT ENABLE IN PRODUCTION.

#![warn(
    missing_docs,
    rustdoc::missing_crate_level_docs,
    unreachable_pub,
    rust_2018_idioms
)]

/// Runtime support logic for generated clients.
#[cfg(feature = "client")]
pub mod client;

/// Cache for entries that have an expiration time.
pub mod expiring_cache;

/// A data structure for persisting and sharing state between multiple clients.
pub mod static_partition_map;

/// General testing utilities.
#[cfg(feature = "test-util")]
pub mod test_util;
