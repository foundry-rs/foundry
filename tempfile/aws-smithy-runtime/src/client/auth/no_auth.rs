/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! The [`NoAuthRuntimePlugin`] and supporting code.

use crate::client::identity::no_auth::NoAuthIdentityResolver;
use aws_smithy_runtime_api::box_error::BoxError;
use aws_smithy_runtime_api::client::auth::{
    AuthScheme, AuthSchemeEndpointConfig, AuthSchemeId, SharedAuthScheme, Sign,
};
use aws_smithy_runtime_api::client::identity::{Identity, SharedIdentityResolver};
use aws_smithy_runtime_api::client::orchestrator::HttpRequest;
use aws_smithy_runtime_api::client::runtime_components::{
    GetIdentityResolver, RuntimeComponents, RuntimeComponentsBuilder,
};
use aws_smithy_runtime_api::client::runtime_plugin::RuntimePlugin;
use aws_smithy_types::config_bag::ConfigBag;
use std::borrow::Cow;

/// Auth scheme ID for "no auth".
pub const NO_AUTH_SCHEME_ID: AuthSchemeId = AuthSchemeId::new("no_auth");

/// A [`RuntimePlugin`] that registers a "no auth" identity resolver and auth scheme.
///
/// This plugin can be used to disable authentication in certain cases, such as when there is
/// a Smithy `@optionalAuth` trait.
#[non_exhaustive]
#[derive(Debug)]
pub struct NoAuthRuntimePlugin(RuntimeComponentsBuilder);

impl Default for NoAuthRuntimePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl NoAuthRuntimePlugin {
    /// Creates a new `NoAuthRuntimePlugin`.
    pub fn new() -> Self {
        Self(
            RuntimeComponentsBuilder::new("NoAuthRuntimePlugin")
                .with_identity_resolver(
                    NO_AUTH_SCHEME_ID,
                    SharedIdentityResolver::new(NoAuthIdentityResolver::new()),
                )
                .with_auth_scheme(SharedAuthScheme::new(NoAuthScheme::new())),
        )
    }
}

impl RuntimePlugin for NoAuthRuntimePlugin {
    fn runtime_components(
        &self,
        _: &RuntimeComponentsBuilder,
    ) -> Cow<'_, RuntimeComponentsBuilder> {
        Cow::Borrowed(&self.0)
    }
}

/// The "no auth" auth scheme.
///
/// The orchestrator requires an auth scheme, so Smithy's `@optionalAuth` trait is implemented
/// by placing a "no auth" auth scheme at the end of the auth scheme options list so that it is
/// used if there's no identity resolver available for the other auth schemes. It's also used
/// for models that don't have auth at all.
#[derive(Debug, Default)]
pub struct NoAuthScheme {
    signer: NoAuthSigner,
}

impl NoAuthScheme {
    /// Creates a new `NoAuthScheme`.
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Default)]
struct NoAuthSigner;

impl Sign for NoAuthSigner {
    fn sign_http_request(
        &self,
        _request: &mut HttpRequest,
        _identity: &Identity,
        _auth_scheme_endpoint_config: AuthSchemeEndpointConfig<'_>,
        _runtime_components: &RuntimeComponents,
        _config_bag: &ConfigBag,
    ) -> Result<(), BoxError> {
        Ok(())
    }
}

impl AuthScheme for NoAuthScheme {
    fn scheme_id(&self) -> AuthSchemeId {
        NO_AUTH_SCHEME_ID
    }

    fn identity_resolver(
        &self,
        identity_resolvers: &dyn GetIdentityResolver,
    ) -> Option<SharedIdentityResolver> {
        identity_resolvers.identity_resolver(NO_AUTH_SCHEME_ID)
    }

    fn signer(&self) -> &dyn Sign {
        &self.signer
    }
}
