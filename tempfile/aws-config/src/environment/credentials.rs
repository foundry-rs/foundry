/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use std::env::VarError;

use aws_credential_types::provider::{self, error::CredentialsError, future, ProvideCredentials};
use aws_credential_types::Credentials;
use aws_types::os_shim_internal::Env;

/// Load Credentials from Environment Variables
///
/// `EnvironmentVariableCredentialsProvider` uses the following variables:
/// - `AWS_ACCESS_KEY_ID`
/// - `AWS_SECRET_ACCESS_KEY` with fallback to `SECRET_ACCESS_KEY`
/// - `AWS_SESSION_TOKEN`
#[derive(Debug, Clone)]
pub struct EnvironmentVariableCredentialsProvider {
    env: Env,
}

impl EnvironmentVariableCredentialsProvider {
    fn credentials(&self) -> provider::Result {
        let access_key = self
            .env
            .get("AWS_ACCESS_KEY_ID")
            .and_then(err_if_blank)
            .map_err(to_cred_error)?;
        let secret_key = self
            .env
            .get("AWS_SECRET_ACCESS_KEY")
            .and_then(err_if_blank)
            .or_else(|_| self.env.get("SECRET_ACCESS_KEY"))
            .and_then(err_if_blank)
            .map_err(to_cred_error)?;
        let session_token =
            self.env
                .get("AWS_SESSION_TOKEN")
                .ok()
                .and_then(|token| match token.trim() {
                    "" => None,
                    s => Some(s.to_string()),
                });
        Ok(Credentials::new(
            access_key,
            secret_key,
            session_token,
            None,
            ENV_PROVIDER,
        ))
    }
}

impl EnvironmentVariableCredentialsProvider {
    /// Create a `EnvironmentVariableCredentialsProvider`
    pub fn new() -> Self {
        Self::new_with_env(Env::real())
    }

    /// Create a new `EnvironmentVariableCredentialsProvider` with `Env` overridden
    ///
    /// This function is intended for tests that mock out the process environment.
    pub(crate) fn new_with_env(env: Env) -> Self {
        Self { env }
    }
}

impl Default for EnvironmentVariableCredentialsProvider {
    fn default() -> Self {
        Self::new()
    }
}

const ENV_PROVIDER: &str = "EnvironmentVariable";

impl ProvideCredentials for EnvironmentVariableCredentialsProvider {
    fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        future::ProvideCredentials::ready(self.credentials())
    }
}

fn to_cred_error(err: VarError) -> CredentialsError {
    match err {
        VarError::NotPresent => CredentialsError::not_loaded("environment variable not set"),
        e @ VarError::NotUnicode(_) => CredentialsError::unhandled(e),
    }
}

fn err_if_blank(value: String) -> Result<String, VarError> {
    if value.trim().is_empty() {
        Err(VarError::NotPresent)
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod test {
    use aws_credential_types::provider::{error::CredentialsError, ProvideCredentials};
    use aws_types::os_shim_internal::Env;
    use futures_util::FutureExt;

    use super::EnvironmentVariableCredentialsProvider;

    fn make_provider(vars: &[(&str, &str)]) -> EnvironmentVariableCredentialsProvider {
        EnvironmentVariableCredentialsProvider {
            env: Env::from_slice(vars),
        }
    }

    #[test]
    fn valid_no_token() {
        let provider = make_provider(&[
            ("AWS_ACCESS_KEY_ID", "access"),
            ("AWS_SECRET_ACCESS_KEY", "secret"),
        ]);
        let creds = provider
            .provide_credentials()
            .now_or_never()
            .unwrap()
            .expect("valid credentials");
        assert_eq!(creds.session_token(), None);
        assert_eq!(creds.access_key_id(), "access");
        assert_eq!(creds.secret_access_key(), "secret");
    }

    #[test]
    fn valid_with_token() {
        let provider = make_provider(&[
            ("AWS_ACCESS_KEY_ID", "access"),
            ("AWS_SECRET_ACCESS_KEY", "secret"),
            ("AWS_SESSION_TOKEN", "token"),
        ]);

        let creds = provider
            .provide_credentials()
            .now_or_never()
            .unwrap()
            .expect("valid credentials");
        assert_eq!(creds.session_token().unwrap(), "token");
        assert_eq!(creds.access_key_id(), "access");
        assert_eq!(creds.secret_access_key(), "secret");
    }

    #[test]
    fn empty_token_env_var() {
        for token_value in &["", " "] {
            let provider = make_provider(&[
                ("AWS_ACCESS_KEY_ID", "access"),
                ("AWS_SECRET_ACCESS_KEY", "secret"),
                ("AWS_SESSION_TOKEN", token_value),
            ]);

            let creds = provider
                .provide_credentials()
                .now_or_never()
                .unwrap()
                .expect("valid credentials");
            assert_eq!(creds.access_key_id(), "access");
            assert_eq!(creds.secret_access_key(), "secret");
            assert_eq!(creds.session_token(), None);
        }
    }

    #[test]
    fn secret_key_fallback() {
        let provider = make_provider(&[
            ("AWS_ACCESS_KEY_ID", "access"),
            ("SECRET_ACCESS_KEY", "secret"),
            ("AWS_SESSION_TOKEN", "token"),
        ]);

        let creds = provider
            .provide_credentials()
            .now_or_never()
            .unwrap()
            .expect("valid credentials");
        assert_eq!(creds.session_token().unwrap(), "token");
        assert_eq!(creds.access_key_id(), "access");
        assert_eq!(creds.secret_access_key(), "secret");
    }

    #[test]
    fn secret_key_fallback_empty() {
        let provider = make_provider(&[
            ("AWS_ACCESS_KEY_ID", "access"),
            ("AWS_SECRET_ACCESS_KEY", " "),
            ("SECRET_ACCESS_KEY", "secret"),
            ("AWS_SESSION_TOKEN", "token"),
        ]);

        let creds = provider
            .provide_credentials()
            .now_or_never()
            .unwrap()
            .expect("valid credentials");
        assert_eq!(creds.session_token().unwrap(), "token");
        assert_eq!(creds.access_key_id(), "access");
        assert_eq!(creds.secret_access_key(), "secret");
    }

    #[test]
    fn missing() {
        let provider = make_provider(&[]);
        let err = provider
            .provide_credentials()
            .now_or_never()
            .unwrap()
            .expect_err("no credentials defined");
        assert!(matches!(err, CredentialsError::CredentialsNotLoaded { .. }));
    }

    #[test]
    fn empty_keys_env_vars() {
        for [access_key_value, secret_key_value] in &[
            &["", ""],
            &[" ", ""],
            &["access", ""],
            &["", " "],
            &[" ", " "],
            &["access", " "],
            &["", "secret"],
            &[" ", "secret"],
        ] {
            let provider = make_provider(&[
                ("AWS_ACCESS_KEY_ID", access_key_value),
                ("AWS_SECRET_ACCESS_KEY", secret_key_value),
            ]);

            let err = provider
                .provide_credentials()
                .now_or_never()
                .unwrap()
                .expect_err("no credentials defined");
            assert!(matches!(err, CredentialsError::CredentialsNotLoaded { .. }));
        }
    }

    #[test]
    fn real_environment() {
        let provider = EnvironmentVariableCredentialsProvider::new();
        // we don't know what's in the env, just make sure it doesn't crash.
        let _fut = provider.provide_credentials();
    }
}
