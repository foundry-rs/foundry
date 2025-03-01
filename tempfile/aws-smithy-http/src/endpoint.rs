/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Code for resolving an endpoint (URI) that a request should be sent to

#![allow(deprecated)]

use crate::endpoint::error::InvalidEndpointError;
use http_02x::uri::{Authority, Uri};
use std::borrow::Cow;
use std::result::Result as StdResult;
use std::str::FromStr;

pub mod error;
pub use error::ResolveEndpointError;

/// An endpoint-resolution-specific Result. Contains either an [`Endpoint`](aws_smithy_types::endpoint::Endpoint) or a [`ResolveEndpointError`].
#[deprecated(since = "0.60.1", note = "Was never used.")]
pub type Result = std::result::Result<aws_smithy_types::endpoint::Endpoint, ResolveEndpointError>;

/// A special type that adds support for services that have special URL-prefixing rules.
#[deprecated(
    since = "0.60.1",
    note = "Use aws_smithy_runtime_api::client::endpoint::EndpointPrefix instead."
)]
pub type EndpointPrefix = aws_smithy_runtime_api::client::endpoint::EndpointPrefix;

/// Apply `endpoint` to `uri`
///
/// This method mutates `uri` by setting the `endpoint` on it
#[deprecated(
    since = "0.60.1",
    note = "Use aws_smithy_runtime::client::endpoint::apply_endpoint instead."
)]
pub fn apply_endpoint(
    uri: &mut Uri,
    endpoint: &Uri,
    prefix: Option<&EndpointPrefix>,
) -> StdResult<(), InvalidEndpointError> {
    let prefix = prefix.map(EndpointPrefix::as_str).unwrap_or("");
    let authority = endpoint
        .authority()
        .as_ref()
        .map(|auth| auth.as_str())
        .unwrap_or("");
    let authority = if !prefix.is_empty() {
        Cow::Owned(format!("{}{}", prefix, authority))
    } else {
        Cow::Borrowed(authority)
    };
    let authority = Authority::from_str(&authority).map_err(|err| {
        InvalidEndpointError::failed_to_construct_authority(authority.into_owned(), err)
    })?;
    let scheme = *endpoint
        .scheme()
        .as_ref()
        .ok_or_else(InvalidEndpointError::endpoint_must_have_scheme)?;
    let new_uri = Uri::builder()
        .authority(authority)
        .scheme(scheme.clone())
        .path_and_query(merge_paths(endpoint, uri).as_ref())
        .build()
        .map_err(InvalidEndpointError::failed_to_construct_uri)?;
    *uri = new_uri;
    Ok(())
}

fn merge_paths<'a>(endpoint: &'a Uri, uri: &'a Uri) -> Cow<'a, str> {
    if let Some(query) = endpoint.path_and_query().and_then(|pq| pq.query()) {
        tracing::warn!(query = %query, "query specified in endpoint will be ignored during endpoint resolution");
    }
    let endpoint_path = endpoint.path();
    let uri_path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("");
    if endpoint_path.is_empty() {
        Cow::Borrowed(uri_path_and_query)
    } else {
        let ep_no_slash = endpoint_path.strip_suffix('/').unwrap_or(endpoint_path);
        let uri_path_no_slash = uri_path_and_query
            .strip_prefix('/')
            .unwrap_or(uri_path_and_query);
        Cow::Owned(format!("{}/{}", ep_no_slash, uri_path_no_slash))
    }
}
