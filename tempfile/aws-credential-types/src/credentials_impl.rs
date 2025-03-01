/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use aws_smithy_types::date_time::Format;
use std::fmt;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

use aws_smithy_runtime_api::client::identity::Identity;

/// AWS SDK Credentials
///
/// An opaque struct representing credentials that may be used in an AWS SDK, modeled on
/// the [CRT credentials implementation](https://github.com/awslabs/aws-c-auth/blob/main/source/credentials.c).
///
/// When `Credentials` is dropped, its contents are zeroed in memory. Credentials uses an interior Arc to ensure
/// that even when cloned, credentials don't exist in multiple memory locations.
#[derive(Clone, Eq, PartialEq)]
pub struct Credentials(Arc<Inner>);

#[derive(Clone, Eq, PartialEq)]
struct Inner {
    access_key_id: Zeroizing<String>,
    secret_access_key: Zeroizing<String>,
    session_token: Zeroizing<Option<String>>,

    /// Credential Expiry
    ///
    /// A SystemTime at which the credentials should no longer be used because they have expired.
    /// The primary purpose of this value is to allow credentials to communicate to the caching
    /// provider when they need to be refreshed.
    ///
    /// If these credentials never expire, this value will be set to `None`
    expires_after: Option<SystemTime>,

    provider_name: &'static str,
}

impl Debug for Credentials {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut creds = f.debug_struct("Credentials");
        creds
            .field("provider_name", &self.0.provider_name)
            .field("access_key_id", &self.0.access_key_id.as_str())
            .field("secret_access_key", &"** redacted **");
        if let Some(expiry) = self.expiry() {
            if let Some(formatted) = expiry.duration_since(UNIX_EPOCH).ok().and_then(|dur| {
                aws_smithy_types::DateTime::from_secs(dur.as_secs() as _)
                    .fmt(Format::DateTime)
                    .ok()
            }) {
                creds.field("expires_after", &formatted);
            } else {
                creds.field("expires_after", &expiry);
            }
        } else {
            creds.field("expires_after", &"never");
        }
        creds.finish()
    }
}

#[cfg(feature = "hardcoded-credentials")]
const STATIC_CREDENTIALS: &str = "Static";

impl Credentials {
    /// Creates `Credentials`.
    ///
    /// This is intended to be used from a custom credentials provider implementation.
    /// It is __NOT__ secure to hardcode credentials into your application.
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        session_token: Option<String>,
        expires_after: Option<SystemTime>,
        provider_name: &'static str,
    ) -> Self {
        Credentials(Arc::new(Inner {
            access_key_id: Zeroizing::new(access_key_id.into()),
            secret_access_key: Zeroizing::new(secret_access_key.into()),
            session_token: Zeroizing::new(session_token),
            expires_after,
            provider_name,
        }))
    }

    /// Creates `Credentials` from hardcoded access key, secret key, and session token.
    ///
    /// _Note: In general, you should prefer to use the credential providers that come
    /// with the AWS SDK to get credentials. It is __NOT__ secure to hardcode credentials
    /// into your application. If you're writing a custom credentials provider, then
    /// use [`Credentials::new`] instead of this._
    ///
    /// This function requires the `hardcoded-credentials` feature to be enabled.
    ///
    /// [`Credentials`] implement
    /// [`ProvideCredentials`](crate::provider::ProvideCredentials) directly, so no custom provider
    /// implementation is required when wiring these up to a client:
    /// ```rust
    /// use aws_credential_types::Credentials;
    /// # mod service {
    /// #     use aws_credential_types::provider::ProvideCredentials;
    /// #     pub struct Config;
    /// #     impl Config {
    /// #        pub fn builder() -> Self {
    /// #            Config
    /// #        }
    /// #        pub fn credentials_provider(self, provider: impl ProvideCredentials + 'static) -> Self {
    /// #            self
    /// #        }
    /// #        pub fn build(self) -> Config { Config }
    /// #     }
    /// #     pub struct Client;
    /// #     impl Client {
    /// #        pub fn from_conf(config: Config) -> Self {
    /// #            Client
    /// #        }
    /// #     }
    /// # }
    /// # use service::{Config, Client};
    ///
    /// let creds = Credentials::from_keys("akid", "secret_key", None);
    /// let config = Config::builder()
    ///     .credentials_provider(creds)
    ///     .build();
    /// let client = Client::from_conf(config);
    /// ```
    #[cfg(feature = "hardcoded-credentials")]
    pub fn from_keys(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        session_token: Option<String>,
    ) -> Self {
        Self::new(
            access_key_id,
            secret_access_key,
            session_token,
            None,
            STATIC_CREDENTIALS,
        )
    }

    /// Returns the access key ID.
    pub fn access_key_id(&self) -> &str {
        &self.0.access_key_id
    }

    /// Returns the secret access key.
    pub fn secret_access_key(&self) -> &str {
        &self.0.secret_access_key
    }

    /// Returns the time when the credentials will expire.
    pub fn expiry(&self) -> Option<SystemTime> {
        self.0.expires_after
    }

    /// Returns a mutable reference to the time when the credentials will expire.
    pub fn expiry_mut(&mut self) -> &mut Option<SystemTime> {
        &mut Arc::make_mut(&mut self.0).expires_after
    }

    /// Returns the session token.
    pub fn session_token(&self) -> Option<&str> {
        self.0.session_token.as_deref()
    }
}

#[cfg(feature = "test-util")]
impl Credentials {
    /// Creates a test `Credentials` with no session token.
    pub fn for_tests() -> Self {
        Self::new(
            "ANOTREAL",
            "notrealrnrELgWzOk3IfjzDKtFBhDby",
            None,
            None,
            "test",
        )
    }

    /// Creates a test `Credentials` that include a session token.
    pub fn for_tests_with_session_token() -> Self {
        Self::new(
            "ANOTREAL",
            "notrealrnrELgWzOk3IfjzDKtFBhDby",
            Some("notarealsessiontoken".to_string()),
            None,
            "test",
        )
    }
}

impl From<Credentials> for Identity {
    fn from(val: Credentials) -> Self {
        let expiry = val.expiry();
        Identity::new(val, expiry)
    }
}

#[cfg(test)]
mod test {
    use crate::Credentials;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn debug_impl() {
        let creds = Credentials::new(
            "akid",
            "secret",
            Some("token".into()),
            Some(UNIX_EPOCH + Duration::from_secs(1234567890)),
            "debug tester",
        );
        assert_eq!(
            format!("{:?}", creds),
            r#"Credentials { provider_name: "debug tester", access_key_id: "akid", secret_access_key: "** redacted **", expires_after: "2009-02-13T23:31:30Z" }"#
        );
    }
}
