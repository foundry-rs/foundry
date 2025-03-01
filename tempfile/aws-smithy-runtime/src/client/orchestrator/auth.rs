/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::client::auth::no_auth::NO_AUTH_SCHEME_ID;
use crate::client::identity::IdentityCache;
use aws_smithy_runtime_api::box_error::BoxError;
use aws_smithy_runtime_api::client::auth::{
    AuthScheme, AuthSchemeEndpointConfig, AuthSchemeId, AuthSchemeOptionResolverParams,
    ResolveAuthSchemeOptions,
};
use aws_smithy_runtime_api::client::identity::ResolveIdentity;
use aws_smithy_runtime_api::client::identity::{IdentityCacheLocation, ResolveCachedIdentity};
use aws_smithy_runtime_api::client::interceptors::context::InterceptorContext;
use aws_smithy_runtime_api::client::runtime_components::RuntimeComponents;
use aws_smithy_types::config_bag::ConfigBag;
use aws_smithy_types::endpoint::Endpoint;
use aws_smithy_types::Document;
use std::borrow::Cow;
use std::error::Error as StdError;
use std::fmt;
use tracing::trace;

#[derive(Debug)]
struct NoMatchingAuthSchemeError(ExploredList);

impl fmt::Display for NoMatchingAuthSchemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let explored = &self.0;

        // Use the information we have about the auth options that were explored to construct
        // as helpful of an error message as possible.
        if explored.items().count() == 0 {
            return f.write_str(
                "no auth options are available. This can happen if there's \
                    a problem with the service model, or if there is a codegen bug.",
            );
        }
        if explored
            .items()
            .all(|explored| matches!(explored.result, ExploreResult::NoAuthScheme))
        {
            return f.write_str(
                "no auth schemes are registered. This can happen if there's \
                    a problem with the service model, or if there is a codegen bug.",
            );
        }

        let mut try_add_identity = false;
        let mut likely_bug = false;
        f.write_str("failed to select an auth scheme to sign the request with.")?;
        for item in explored.items() {
            write!(
                f,
                " \"{}\" wasn't a valid option because ",
                item.scheme_id.as_str()
            )?;
            f.write_str(match item.result {
                ExploreResult::NoAuthScheme => {
                    likely_bug = true;
                    "no auth scheme was registered for it."
                }
                ExploreResult::NoIdentityResolver => {
                    try_add_identity = true;
                    "there was no identity resolver for it."
                }
                ExploreResult::MissingEndpointConfig => {
                    likely_bug = true;
                    "there is auth config in the endpoint config, but this scheme wasn't listed in it \
                    (see https://github.com/smithy-lang/smithy-rs/discussions/3281 for more details)."
                }
                ExploreResult::NotExplored => {
                    debug_assert!(false, "this should be unreachable");
                    "<unknown>"
                }
            })?;
        }
        if try_add_identity {
            f.write_str(" Be sure to set an identity, such as credentials, auth token, or other identity type that is required for this service.")?;
        } else if likely_bug {
            f.write_str(" This is likely a bug.")?;
        }
        if explored.truncated {
            f.write_str(" Note: there were other auth schemes that were evaluated that weren't listed here.")?;
        }

        Ok(())
    }
}

impl StdError for NoMatchingAuthSchemeError {}

#[derive(Debug)]
enum AuthOrchestrationError {
    MissingEndpointConfig,
    BadAuthSchemeEndpointConfig(Cow<'static, str>),
}

impl fmt::Display for AuthOrchestrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // This error is never bubbled up
            Self::MissingEndpointConfig => f.write_str("missing endpoint config"),
            Self::BadAuthSchemeEndpointConfig(message) => f.write_str(message),
        }
    }
}

impl StdError for AuthOrchestrationError {}

pub(super) async fn orchestrate_auth(
    ctx: &mut InterceptorContext,
    runtime_components: &RuntimeComponents,
    cfg: &ConfigBag,
) -> Result<(), BoxError> {
    let params = cfg
        .load::<AuthSchemeOptionResolverParams>()
        .expect("auth scheme option resolver params must be set");
    let option_resolver = runtime_components.auth_scheme_option_resolver();
    let options = option_resolver.resolve_auth_scheme_options(params)?;
    let endpoint = cfg
        .load::<Endpoint>()
        .expect("endpoint added to config bag by endpoint orchestrator");

    trace!(
        auth_scheme_option_resolver_params = ?params,
        auth_scheme_options = ?options,
        "orchestrating auth",
    );

    let mut explored = ExploredList::default();

    // Iterate over IDs of possibly-supported auth schemes
    for &scheme_id in options.as_ref() {
        // For each ID, try to resolve the corresponding auth scheme.
        if let Some(auth_scheme) = runtime_components.auth_scheme(scheme_id) {
            // Use the resolved auth scheme to resolve an identity
            if let Some(identity_resolver) = auth_scheme.identity_resolver(runtime_components) {
                let identity_cache = if identity_resolver.cache_location()
                    == IdentityCacheLocation::RuntimeComponents
                {
                    runtime_components.identity_cache()
                } else {
                    IdentityCache::no_cache()
                };
                let signer = auth_scheme.signer();
                trace!(
                    auth_scheme = ?auth_scheme,
                    identity_cache = ?identity_cache,
                    identity_resolver = ?identity_resolver,
                    signer = ?signer,
                    "resolved auth scheme, identity cache, identity resolver, and signing implementation"
                );

                match extract_endpoint_auth_scheme_config(endpoint, scheme_id) {
                    Ok(auth_scheme_endpoint_config) => {
                        trace!(auth_scheme_endpoint_config = ?auth_scheme_endpoint_config, "extracted auth scheme endpoint config");

                        let identity = identity_cache
                            .resolve_cached_identity(identity_resolver, runtime_components, cfg)
                            .await?;
                        trace!(identity = ?identity, "resolved identity");

                        trace!("signing request");
                        let request = ctx.request_mut().expect("set during serialization");
                        signer.sign_http_request(
                            request,
                            &identity,
                            auth_scheme_endpoint_config,
                            runtime_components,
                            cfg,
                        )?;
                        return Ok(());
                    }
                    Err(AuthOrchestrationError::MissingEndpointConfig) => {
                        explored.push(scheme_id, ExploreResult::MissingEndpointConfig);
                        continue;
                    }
                    Err(other_err) => return Err(other_err.into()),
                }
            } else {
                explored.push(scheme_id, ExploreResult::NoIdentityResolver);
            }
        } else {
            explored.push(scheme_id, ExploreResult::NoAuthScheme);
        }
    }

    Err(NoMatchingAuthSchemeError(explored).into())
}

fn extract_endpoint_auth_scheme_config(
    endpoint: &Endpoint,
    scheme_id: AuthSchemeId,
) -> Result<AuthSchemeEndpointConfig<'_>, AuthOrchestrationError> {
    // TODO(P96049742): Endpoint config doesn't currently have a concept of optional auth or "no auth", so
    // we are short-circuiting lookup of endpoint auth scheme config if that is the selected scheme.
    if scheme_id == NO_AUTH_SCHEME_ID {
        return Ok(AuthSchemeEndpointConfig::empty());
    }
    let auth_schemes = match endpoint.properties().get("authSchemes") {
        Some(Document::Array(schemes)) => schemes,
        // no auth schemes:
        None => return Ok(AuthSchemeEndpointConfig::empty()),
        _other => {
            return Err(AuthOrchestrationError::BadAuthSchemeEndpointConfig(
                "expected an array for `authSchemes` in endpoint config".into(),
            ))
        }
    };
    let auth_scheme_config = auth_schemes
        .iter()
        .find(|doc| {
            let config_scheme_id = doc
                .as_object()
                .and_then(|object| object.get("name"))
                .and_then(Document::as_string);
            config_scheme_id == Some(scheme_id.as_str())
        })
        .ok_or(AuthOrchestrationError::MissingEndpointConfig)?;
    Ok(AuthSchemeEndpointConfig::from(Some(auth_scheme_config)))
}

#[derive(Debug)]
enum ExploreResult {
    NotExplored,
    NoAuthScheme,
    NoIdentityResolver,
    MissingEndpointConfig,
}

/// Information about an evaluated auth option.
/// This should be kept small so it can fit in an array on the stack.
#[derive(Debug)]
struct ExploredAuthOption {
    scheme_id: AuthSchemeId,
    result: ExploreResult,
}
impl Default for ExploredAuthOption {
    fn default() -> Self {
        Self {
            scheme_id: AuthSchemeId::new(""),
            result: ExploreResult::NotExplored,
        }
    }
}

const MAX_EXPLORED_LIST_LEN: usize = 8;

/// Stack allocated list of explored auth options for error messaging
#[derive(Default)]
struct ExploredList {
    items: [ExploredAuthOption; MAX_EXPLORED_LIST_LEN],
    len: usize,
    truncated: bool,
}
impl ExploredList {
    fn items(&self) -> impl Iterator<Item = &ExploredAuthOption> {
        self.items.iter().take(self.len)
    }

    fn push(&mut self, scheme_id: AuthSchemeId, result: ExploreResult) {
        if self.len + 1 >= self.items.len() {
            self.truncated = true;
        } else {
            self.items[self.len] = ExploredAuthOption { scheme_id, result };
            self.len += 1;
        }
    }
}
impl fmt::Debug for ExploredList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExploredList")
            .field("items", &&self.items[0..self.len])
            .field("truncated", &self.truncated)
            .finish()
    }
}

#[cfg(all(test, feature = "test-util"))]
mod tests {
    use super::*;
    use aws_smithy_runtime_api::client::auth::static_resolver::StaticAuthSchemeOptionResolver;
    use aws_smithy_runtime_api::client::auth::{
        AuthScheme, AuthSchemeId, AuthSchemeOptionResolverParams, SharedAuthScheme,
        SharedAuthSchemeOptionResolver, Sign,
    };
    use aws_smithy_runtime_api::client::identity::{
        Identity, IdentityFuture, ResolveIdentity, SharedIdentityResolver,
    };
    use aws_smithy_runtime_api::client::interceptors::context::{Input, InterceptorContext};
    use aws_smithy_runtime_api::client::orchestrator::HttpRequest;
    use aws_smithy_runtime_api::client::runtime_components::{
        GetIdentityResolver, RuntimeComponents, RuntimeComponentsBuilder,
    };
    use aws_smithy_types::config_bag::Layer;
    use std::collections::HashMap;

    #[tokio::test]
    async fn basic_case() {
        #[derive(Debug)]
        struct TestIdentityResolver;
        impl ResolveIdentity for TestIdentityResolver {
            fn resolve_identity<'a>(
                &'a self,
                _runtime_components: &'a RuntimeComponents,
                _config_bag: &'a ConfigBag,
            ) -> IdentityFuture<'a> {
                IdentityFuture::ready(Ok(Identity::new("doesntmatter", None)))
            }
        }

        #[derive(Debug)]
        struct TestSigner;

        impl Sign for TestSigner {
            fn sign_http_request(
                &self,
                request: &mut HttpRequest,
                _identity: &Identity,
                _auth_scheme_endpoint_config: AuthSchemeEndpointConfig<'_>,
                _runtime_components: &RuntimeComponents,
                _config_bag: &ConfigBag,
            ) -> Result<(), BoxError> {
                request
                    .headers_mut()
                    .insert(http_02x::header::AUTHORIZATION, "success!");
                Ok(())
            }
        }

        const TEST_SCHEME_ID: AuthSchemeId = AuthSchemeId::new("test-scheme");

        #[derive(Debug)]
        struct TestAuthScheme {
            signer: TestSigner,
        }
        impl AuthScheme for TestAuthScheme {
            fn scheme_id(&self) -> AuthSchemeId {
                TEST_SCHEME_ID
            }

            fn identity_resolver(
                &self,
                identity_resolvers: &dyn GetIdentityResolver,
            ) -> Option<SharedIdentityResolver> {
                identity_resolvers.identity_resolver(self.scheme_id())
            }

            fn signer(&self) -> &dyn Sign {
                &self.signer
            }
        }

        let mut ctx = InterceptorContext::new(Input::doesnt_matter());
        ctx.enter_serialization_phase();
        ctx.set_request(HttpRequest::empty());
        let _ = ctx.take_input();
        ctx.enter_before_transmit_phase();

        let runtime_components = RuntimeComponentsBuilder::for_tests()
            .with_auth_scheme(SharedAuthScheme::new(TestAuthScheme { signer: TestSigner }))
            .with_auth_scheme_option_resolver(Some(SharedAuthSchemeOptionResolver::new(
                StaticAuthSchemeOptionResolver::new(vec![TEST_SCHEME_ID]),
            )))
            .with_identity_resolver(
                TEST_SCHEME_ID,
                SharedIdentityResolver::new(TestIdentityResolver),
            )
            .build()
            .unwrap();

        let mut layer: Layer = Layer::new("test");
        layer.store_put(AuthSchemeOptionResolverParams::new("doesntmatter"));
        layer.store_put(Endpoint::builder().url("dontcare").build());
        let cfg = ConfigBag::of_layers(vec![layer]);

        orchestrate_auth(&mut ctx, &runtime_components, &cfg)
            .await
            .expect("success");

        assert_eq!(
            "success!",
            ctx.request()
                .expect("request is set")
                .headers()
                .get("Authorization")
                .unwrap()
        );
    }

    #[cfg(feature = "http-auth")]
    #[tokio::test]
    async fn select_best_scheme_for_available_identity_resolvers() {
        use crate::client::auth::http::{BasicAuthScheme, BearerAuthScheme};
        use aws_smithy_runtime_api::client::auth::http::{
            HTTP_BASIC_AUTH_SCHEME_ID, HTTP_BEARER_AUTH_SCHEME_ID,
        };
        use aws_smithy_runtime_api::client::identity::http::{Login, Token};

        let mut ctx = InterceptorContext::new(Input::doesnt_matter());
        ctx.enter_serialization_phase();
        ctx.set_request(HttpRequest::empty());
        let _ = ctx.take_input();
        ctx.enter_before_transmit_phase();

        fn config_with_identity(
            scheme_id: AuthSchemeId,
            identity: impl ResolveIdentity + 'static,
        ) -> (RuntimeComponents, ConfigBag) {
            let runtime_components = RuntimeComponentsBuilder::for_tests()
                .with_auth_scheme(SharedAuthScheme::new(BasicAuthScheme::new()))
                .with_auth_scheme(SharedAuthScheme::new(BearerAuthScheme::new()))
                .with_auth_scheme_option_resolver(Some(SharedAuthSchemeOptionResolver::new(
                    StaticAuthSchemeOptionResolver::new(vec![
                        HTTP_BASIC_AUTH_SCHEME_ID,
                        HTTP_BEARER_AUTH_SCHEME_ID,
                    ]),
                )))
                .with_identity_resolver(scheme_id, SharedIdentityResolver::new(identity))
                .build()
                .unwrap();

            let mut layer = Layer::new("test");
            layer.store_put(Endpoint::builder().url("dontcare").build());
            layer.store_put(AuthSchemeOptionResolverParams::new("doesntmatter"));

            (runtime_components, ConfigBag::of_layers(vec![layer]))
        }

        // First, test the presence of a basic auth login and absence of a bearer token
        let (runtime_components, cfg) =
            config_with_identity(HTTP_BASIC_AUTH_SCHEME_ID, Login::new("a", "b", None));
        orchestrate_auth(&mut ctx, &runtime_components, &cfg)
            .await
            .expect("success");
        assert_eq!(
            // "YTpi" == "a:b" in base64
            "Basic YTpi",
            ctx.request()
                .expect("request is set")
                .headers()
                .get("Authorization")
                .unwrap()
        );

        // Next, test the presence of a bearer token and absence of basic auth
        let (runtime_components, cfg) =
            config_with_identity(HTTP_BEARER_AUTH_SCHEME_ID, Token::new("t", None));
        let mut ctx = InterceptorContext::new(Input::erase("doesnt-matter"));
        ctx.enter_serialization_phase();
        ctx.set_request(HttpRequest::empty());
        let _ = ctx.take_input();
        ctx.enter_before_transmit_phase();
        orchestrate_auth(&mut ctx, &runtime_components, &cfg)
            .await
            .expect("success");
        assert_eq!(
            "Bearer t",
            ctx.request()
                .expect("request is set")
                .headers()
                .get("Authorization")
                .unwrap()
        );
    }

    #[test]
    fn extract_endpoint_auth_scheme_config_no_config() {
        let endpoint = Endpoint::builder()
            .url("dontcare")
            .property("something-unrelated", Document::Null)
            .build();
        let config = extract_endpoint_auth_scheme_config(&endpoint, "test-scheme-id".into())
            .expect("success");
        assert!(config.as_document().is_none());
    }

    #[test]
    fn extract_endpoint_auth_scheme_config_wrong_type() {
        let endpoint = Endpoint::builder()
            .url("dontcare")
            .property("authSchemes", Document::String("bad".into()))
            .build();
        extract_endpoint_auth_scheme_config(&endpoint, "test-scheme-id".into())
            .expect_err("should fail because authSchemes is the wrong type");
    }

    #[test]
    fn extract_endpoint_auth_scheme_config_no_matching_scheme() {
        let endpoint = Endpoint::builder()
            .url("dontcare")
            .property(
                "authSchemes",
                vec![
                    Document::Object({
                        let mut out = HashMap::new();
                        out.insert("name".to_string(), "wrong-scheme-id".to_string().into());
                        out
                    }),
                    Document::Object({
                        let mut out = HashMap::new();
                        out.insert(
                            "name".to_string(),
                            "another-wrong-scheme-id".to_string().into(),
                        );
                        out
                    }),
                ],
            )
            .build();
        extract_endpoint_auth_scheme_config(&endpoint, "test-scheme-id".into())
            .expect_err("should fail because authSchemes doesn't include the desired scheme");
    }

    #[test]
    fn extract_endpoint_auth_scheme_config_successfully() {
        let endpoint = Endpoint::builder()
            .url("dontcare")
            .property(
                "authSchemes",
                vec![
                    Document::Object({
                        let mut out = HashMap::new();
                        out.insert("name".to_string(), "wrong-scheme-id".to_string().into());
                        out
                    }),
                    Document::Object({
                        let mut out = HashMap::new();
                        out.insert("name".to_string(), "test-scheme-id".to_string().into());
                        out.insert(
                            "magicString".to_string(),
                            "magic string value".to_string().into(),
                        );
                        out
                    }),
                ],
            )
            .build();
        let config = extract_endpoint_auth_scheme_config(&endpoint, "test-scheme-id".into())
            .expect("should find test-scheme-id");
        assert_eq!(
            "magic string value",
            config
                .as_document()
                .expect("config is set")
                .as_object()
                .expect("it's an object")
                .get("magicString")
                .expect("magicString is set")
                .as_string()
                .expect("gimme the string, dammit!")
        );
    }

    #[cfg(feature = "http-auth")]
    #[tokio::test]
    async fn use_identity_cache() {
        use crate::client::auth::http::{ApiKeyAuthScheme, ApiKeyLocation};
        use aws_smithy_runtime_api::client::auth::http::HTTP_API_KEY_AUTH_SCHEME_ID;
        use aws_smithy_runtime_api::client::identity::http::Token;
        use aws_smithy_types::body::SdkBody;

        let mut ctx = InterceptorContext::new(Input::doesnt_matter());
        ctx.enter_serialization_phase();
        ctx.set_request(
            http_02x::Request::builder()
                .body(SdkBody::empty())
                .unwrap()
                .try_into()
                .unwrap(),
        );
        let _ = ctx.take_input();
        ctx.enter_before_transmit_phase();

        #[derive(Debug)]
        struct Cache;
        impl ResolveCachedIdentity for Cache {
            fn resolve_cached_identity<'a>(
                &'a self,
                _resolver: SharedIdentityResolver,
                _: &'a RuntimeComponents,
                _config_bag: &'a ConfigBag,
            ) -> IdentityFuture<'a> {
                IdentityFuture::ready(Ok(Identity::new(Token::new("cached (pass)", None), None)))
            }
        }

        let runtime_components = RuntimeComponentsBuilder::for_tests()
            .with_auth_scheme(SharedAuthScheme::new(ApiKeyAuthScheme::new(
                "result:",
                ApiKeyLocation::Header,
                "Authorization",
            )))
            .with_auth_scheme_option_resolver(Some(SharedAuthSchemeOptionResolver::new(
                StaticAuthSchemeOptionResolver::new(vec![HTTP_API_KEY_AUTH_SCHEME_ID]),
            )))
            .with_identity_cache(Some(Cache))
            .with_identity_resolver(
                HTTP_API_KEY_AUTH_SCHEME_ID,
                SharedIdentityResolver::new(Token::new("uncached (fail)", None)),
            )
            .build()
            .unwrap();
        let mut layer = Layer::new("test");
        layer.store_put(Endpoint::builder().url("dontcare").build());
        layer.store_put(AuthSchemeOptionResolverParams::new("doesntmatter"));
        let config_bag = ConfigBag::of_layers(vec![layer]);

        orchestrate_auth(&mut ctx, &runtime_components, &config_bag)
            .await
            .expect("success");
        assert_eq!(
            "result: cached (pass)",
            ctx.request()
                .expect("request is set")
                .headers()
                .get("Authorization")
                .unwrap()
        );
    }

    #[test]
    fn friendly_error_messages() {
        let err = NoMatchingAuthSchemeError(ExploredList::default());
        assert_eq!(
            "no auth options are available. This can happen if there's a problem with \
            the service model, or if there is a codegen bug.",
            err.to_string()
        );

        let mut list = ExploredList::default();
        list.push(
            AuthSchemeId::new("SigV4"),
            ExploreResult::NoIdentityResolver,
        );
        list.push(
            AuthSchemeId::new("SigV4a"),
            ExploreResult::NoIdentityResolver,
        );
        let err = NoMatchingAuthSchemeError(list);
        assert_eq!(
            "failed to select an auth scheme to sign the request with. \
            \"SigV4\" wasn't a valid option because there was no identity resolver for it. \
            \"SigV4a\" wasn't a valid option because there was no identity resolver for it. \
            Be sure to set an identity, such as credentials, auth token, or other identity \
            type that is required for this service.",
            err.to_string()
        );

        // It should prioritize the suggestion to try an identity before saying it's a bug
        let mut list = ExploredList::default();
        list.push(
            AuthSchemeId::new("SigV4"),
            ExploreResult::NoIdentityResolver,
        );
        list.push(
            AuthSchemeId::new("SigV4a"),
            ExploreResult::MissingEndpointConfig,
        );
        let err = NoMatchingAuthSchemeError(list);
        assert_eq!(
            "failed to select an auth scheme to sign the request with. \
            \"SigV4\" wasn't a valid option because there was no identity resolver for it. \
            \"SigV4a\" wasn't a valid option because there is auth config in the endpoint \
            config, but this scheme wasn't listed in it (see \
            https://github.com/smithy-lang/smithy-rs/discussions/3281 for more details). \
            Be sure to set an identity, such as credentials, auth token, or other identity \
            type that is required for this service.",
            err.to_string()
        );

        // Otherwise, it should suggest it's a bug
        let mut list = ExploredList::default();
        list.push(
            AuthSchemeId::new("SigV4a"),
            ExploreResult::MissingEndpointConfig,
        );
        let err = NoMatchingAuthSchemeError(list);
        assert_eq!(
            "failed to select an auth scheme to sign the request with. \
            \"SigV4a\" wasn't a valid option because there is auth config in the endpoint \
            config, but this scheme wasn't listed in it (see \
            https://github.com/smithy-lang/smithy-rs/discussions/3281 for more details). \
            This is likely a bug.",
            err.to_string()
        );

        // Truncation should be indicated
        let mut list = ExploredList::default();
        for _ in 0..=MAX_EXPLORED_LIST_LEN {
            list.push(
                AuthSchemeId::new("dontcare"),
                ExploreResult::MissingEndpointConfig,
            );
        }
        let err = NoMatchingAuthSchemeError(list).to_string();
        if !err.contains(
            "Note: there were other auth schemes that were evaluated that weren't listed here",
        ) {
            panic!("The error should indicate that the explored list was truncated.");
        }
    }
}
