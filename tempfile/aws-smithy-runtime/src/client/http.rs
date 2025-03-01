/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

/// Interceptor for connection poisoning.
pub mod connection_poisoning;

#[cfg(feature = "test-util")]
pub mod test_util;

/// Default HTTP and TLS connectors that use hyper 0.14.x and rustls.
///
/// This module is named after the hyper version number since we anticipate
/// needing to provide equivalent functionality for hyper 1.x in the future.
#[cfg(feature = "connector-hyper-0-14-x")]
pub mod hyper_014;

/// HTTP body and body-wrapper types
pub mod body;
