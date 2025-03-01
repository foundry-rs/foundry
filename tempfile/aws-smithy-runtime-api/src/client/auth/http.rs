/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::client::auth::AuthSchemeId;

/// Auth scheme ID for HTTP API key based authentication.
pub const HTTP_API_KEY_AUTH_SCHEME_ID: AuthSchemeId = AuthSchemeId::new("http-api-key-auth");

/// Auth scheme ID for HTTP Basic Auth.
pub const HTTP_BASIC_AUTH_SCHEME_ID: AuthSchemeId = AuthSchemeId::new("http-basic-auth");

/// Auth scheme ID for HTTP Bearer Auth.
pub const HTTP_BEARER_AUTH_SCHEME_ID: AuthSchemeId = AuthSchemeId::new("http-bearer-auth");

/// Auth scheme ID for HTTP Digest Auth.
pub const HTTP_DIGEST_AUTH_SCHEME_ID: AuthSchemeId = AuthSchemeId::new("http-digest-auth");
