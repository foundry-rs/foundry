/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! APIs needed to configure endpoint resolution for clients.

use crate::box_error::BoxError;
use crate::client::runtime_components::sealed::ValidateConfig;
use crate::impl_shared_conversions;
use aws_smithy_types::config_bag::{Storable, StoreReplace};
use aws_smithy_types::endpoint::Endpoint;
use aws_smithy_types::type_erasure::TypeErasedBox;
use error::InvalidEndpointError;
use http_02x::uri::Authority;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

new_type_future! {
    #[doc = "Future for [`EndpointResolver::resolve_endpoint`]."]
    pub struct EndpointFuture<'a, Endpoint, BoxError>;
}

/// Parameters originating from the Smithy endpoint ruleset required for endpoint resolution.
///
/// The actual endpoint parameters are code generated from the Smithy model, and thus,
/// are not known to the runtime crates. Hence, this struct is really a new-type around
/// a [`TypeErasedBox`] that holds the actual concrete parameters in it.
#[derive(Debug)]
pub struct EndpointResolverParams(TypeErasedBox);

impl EndpointResolverParams {
    /// Creates a new [`EndpointResolverParams`] from a concrete parameters instance.
    pub fn new<T: fmt::Debug + Send + Sync + 'static>(params: T) -> Self {
        Self(TypeErasedBox::new(params))
    }

    /// Attempts to downcast the underlying concrete parameters to `T` and return it as a reference.
    pub fn get<T: fmt::Debug + Send + Sync + 'static>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }
}

impl Storable for EndpointResolverParams {
    type Storer = StoreReplace<Self>;
}

/// Configurable endpoint resolver implementation.
pub trait ResolveEndpoint: Send + Sync + fmt::Debug {
    /// Asynchronously resolves an endpoint to use from the given endpoint parameters.
    fn resolve_endpoint<'a>(&'a self, params: &'a EndpointResolverParams) -> EndpointFuture<'a>;
}

/// Shared endpoint resolver.
///
/// This is a simple shared ownership wrapper type for the [`ResolveEndpoint`] trait.
#[derive(Clone, Debug)]
pub struct SharedEndpointResolver(Arc<dyn ResolveEndpoint>);

impl SharedEndpointResolver {
    /// Creates a new [`SharedEndpointResolver`].
    pub fn new(endpoint_resolver: impl ResolveEndpoint + 'static) -> Self {
        Self(Arc::new(endpoint_resolver))
    }
}

impl ResolveEndpoint for SharedEndpointResolver {
    fn resolve_endpoint<'a>(&'a self, params: &'a EndpointResolverParams) -> EndpointFuture<'a> {
        self.0.resolve_endpoint(params)
    }
}

impl ValidateConfig for SharedEndpointResolver {}

impl_shared_conversions!(convert SharedEndpointResolver from ResolveEndpoint using SharedEndpointResolver::new);

/// A special type that adds support for services that have special URL-prefixing rules.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EndpointPrefix(String);
impl EndpointPrefix {
    /// Create a new endpoint prefix from an `impl Into<String>`. If the prefix argument is invalid,
    /// a [`InvalidEndpointError`] will be returned.
    pub fn new(prefix: impl Into<String>) -> Result<Self, InvalidEndpointError> {
        let prefix = prefix.into();
        match Authority::from_str(&prefix) {
            Ok(_) => Ok(EndpointPrefix(prefix)),
            Err(err) => Err(InvalidEndpointError::failed_to_construct_authority(
                prefix, err,
            )),
        }
    }

    /// Get the `str` representation of this `EndpointPrefix`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Storable for EndpointPrefix {
    type Storer = StoreReplace<Self>;
}

/// Errors related to endpoint resolution and validation
pub mod error {
    use crate::box_error::BoxError;
    use std::error::Error as StdError;
    use std::fmt;

    /// Endpoint resolution failed
    #[derive(Debug)]
    pub struct ResolveEndpointError {
        message: String,
        source: Option<BoxError>,
    }

    impl ResolveEndpointError {
        /// Create an [`ResolveEndpointError`] with a message
        pub fn message(message: impl Into<String>) -> Self {
            Self {
                message: message.into(),
                source: None,
            }
        }

        /// Add a source to the error
        pub fn with_source(self, source: Option<BoxError>) -> Self {
            Self { source, ..self }
        }

        /// Create a [`ResolveEndpointError`] from a message and a source
        pub fn from_source(message: impl Into<String>, source: impl Into<BoxError>) -> Self {
            Self::message(message).with_source(Some(source.into()))
        }
    }

    impl fmt::Display for ResolveEndpointError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.message)
        }
    }

    impl StdError for ResolveEndpointError {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            self.source.as_ref().map(|err| err.as_ref() as _)
        }
    }

    #[derive(Debug)]
    pub(super) enum InvalidEndpointErrorKind {
        EndpointMustHaveScheme,
        FailedToConstructAuthority { authority: String, source: BoxError },
        FailedToConstructUri { source: BoxError },
    }

    /// An error that occurs when an endpoint is found to be invalid. This usually occurs due to an
    /// incomplete URI.
    #[derive(Debug)]
    pub struct InvalidEndpointError {
        pub(super) kind: InvalidEndpointErrorKind,
    }

    impl InvalidEndpointError {
        /// Construct a build error for a missing scheme
        pub fn endpoint_must_have_scheme() -> Self {
            Self {
                kind: InvalidEndpointErrorKind::EndpointMustHaveScheme,
            }
        }

        /// Construct a build error for an invalid authority
        pub fn failed_to_construct_authority(
            authority: impl Into<String>,
            source: impl Into<Box<dyn StdError + Send + Sync + 'static>>,
        ) -> Self {
            Self {
                kind: InvalidEndpointErrorKind::FailedToConstructAuthority {
                    authority: authority.into(),
                    source: source.into(),
                },
            }
        }

        /// Construct a build error for an invalid URI
        pub fn failed_to_construct_uri(
            source: impl Into<Box<dyn StdError + Send + Sync + 'static>>,
        ) -> Self {
            Self {
                kind: InvalidEndpointErrorKind::FailedToConstructUri {
                    source: source.into(),
                },
            }
        }
    }

    impl From<InvalidEndpointErrorKind> for InvalidEndpointError {
        fn from(kind: InvalidEndpointErrorKind) -> Self {
            Self { kind }
        }
    }

    impl fmt::Display for InvalidEndpointError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            use InvalidEndpointErrorKind as ErrorKind;
            match &self.kind {
            ErrorKind::EndpointMustHaveScheme => write!(f, "endpoint must contain a valid scheme"),
            ErrorKind::FailedToConstructAuthority { authority, source: _ } => write!(
                f,
                "endpoint must contain a valid authority when combined with endpoint prefix: {authority}"
            ),
            ErrorKind::FailedToConstructUri { .. } => write!(f, "failed to construct URI"),
        }
        }
    }

    impl StdError for InvalidEndpointError {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            use InvalidEndpointErrorKind as ErrorKind;
            match &self.kind {
                ErrorKind::FailedToConstructUri { source } => Some(source.as_ref()),
                ErrorKind::FailedToConstructAuthority {
                    authority: _,
                    source,
                } => Some(source.as_ref()),
                ErrorKind::EndpointMustHaveScheme => None,
            }
        }
    }
}
