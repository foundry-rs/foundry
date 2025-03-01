/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

#![cfg(feature = "credentials-process")]

//! Credentials Provider for external process

use crate::json_credentials::{json_parse_loop, InvalidJsonCredentials};
use crate::sensitive_command::CommandWithSensitiveArgs;
use aws_credential_types::provider::{self, error::CredentialsError, future, ProvideCredentials};
use aws_credential_types::Credentials;
use aws_smithy_json::deserialize::Token;
use std::process::Command;
use std::time::SystemTime;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// External process credentials provider
///
/// This credentials provider runs a configured external process and parses
/// its output to retrieve credentials.
///
/// The external process must exit with status 0 and output the following
/// JSON format to `stdout` to provide credentials:
///
/// ```json
/// {
///     "Version:" 1,
///     "AccessKeyId": "access key id",
///     "SecretAccessKey": "secret access key",
///     "SessionToken": "session token",
///     "Expiration": "time that the expiration will expire"
/// }
/// ```
///
/// The `Version` must be set to 1. `AccessKeyId` and `SecretAccessKey` are always required.
/// `SessionToken` must be set if a session token is associated with the `AccessKeyId`.
/// The `Expiration` is optional, and must be given in the RFC 3339 date time format (e.g.,
/// `2022-05-26T12:34:56.789Z`).
///
/// If the external process exits with a non-zero status, then the contents of `stderr`
/// will be output as part of the credentials provider error message.
///
/// This credentials provider is included in the profile credentials provider, and can be
/// configured using the `credential_process` attribute. For example:
///
/// ```plain
/// [profile example]
/// credential_process = /path/to/my/process --some --arguments
/// ```
#[derive(Debug)]
pub struct CredentialProcessProvider {
    command: CommandWithSensitiveArgs<String>,
}

impl ProvideCredentials for CredentialProcessProvider {
    fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        future::ProvideCredentials::new(self.credentials())
    }
}

impl CredentialProcessProvider {
    /// Create new [`CredentialProcessProvider`] with the `command` needed to execute the external process.
    pub fn new(command: String) -> Self {
        Self {
            command: CommandWithSensitiveArgs::new(command),
        }
    }

    pub(crate) fn from_command(command: &CommandWithSensitiveArgs<&str>) -> Self {
        Self {
            command: command.to_owned_string(),
        }
    }

    async fn credentials(&self) -> provider::Result {
        // Security: command arguments must be redacted at debug level
        tracing::debug!(command = %self.command, "loading credentials from external process");

        let command = if cfg!(windows) {
            let mut command = Command::new("cmd.exe");
            command.args(["/C", self.command.unredacted()]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", self.command.unredacted()]);
            command
        };
        let output = tokio::process::Command::from(command)
            .output()
            .await
            .map_err(|e| {
                CredentialsError::provider_error(format!(
                    "Error retrieving credentials from external process: {}",
                    e
                ))
            })?;

        // Security: command arguments can be logged at trace level
        tracing::trace!(command = ?self.command, status = ?output.status, "executed command (unredacted)");

        if !output.status.success() {
            let reason =
                std::str::from_utf8(&output.stderr).unwrap_or("could not decode stderr as UTF-8");
            return Err(CredentialsError::provider_error(format!(
                "Error retrieving credentials: external process exited with code {}. Stderr: {}",
                output.status, reason
            )));
        }

        let output = std::str::from_utf8(&output.stdout).map_err(|e| {
            CredentialsError::provider_error(format!(
                "Error retrieving credentials from external process: could not decode output as UTF-8: {}",
                e
            ))
        })?;

        parse_credential_process_json_credentials(output).map_err(|invalid| {
            CredentialsError::provider_error(format!(
                "Error retrieving credentials from external process, could not parse response: {}",
                invalid
            ))
        })
    }
}

/// Deserialize a credential_process response from a string
///
/// Returns an error if the response cannot be successfully parsed or is missing keys.
///
/// Keys are case insensitive.
pub(crate) fn parse_credential_process_json_credentials(
    credentials_response: &str,
) -> Result<Credentials, InvalidJsonCredentials> {
    let mut version = None;
    let mut access_key_id = None;
    let mut secret_access_key = None;
    let mut session_token = None;
    let mut expiration = None;
    json_parse_loop(credentials_response.as_bytes(), |key, value| {
        match (key, value) {
            /*
             "Version": 1,
             "AccessKeyId": "ASIARTESTID",
             "SecretAccessKey": "TESTSECRETKEY",
             "SessionToken": "TESTSESSIONTOKEN",
             "Expiration": "2022-05-02T18:36:00+00:00"
            */
            (key, Token::ValueNumber { value, .. }) if key.eq_ignore_ascii_case("Version") => {
                version = Some(i32::try_from(*value).map_err(|err| {
                    InvalidJsonCredentials::InvalidField {
                        field: "Version",
                        err: err.into(),
                    }
                })?);
            }
            (key, Token::ValueString { value, .. }) if key.eq_ignore_ascii_case("AccessKeyId") => {
                access_key_id = Some(value.to_unescaped()?)
            }
            (key, Token::ValueString { value, .. })
                if key.eq_ignore_ascii_case("SecretAccessKey") =>
            {
                secret_access_key = Some(value.to_unescaped()?)
            }
            (key, Token::ValueString { value, .. }) if key.eq_ignore_ascii_case("SessionToken") => {
                session_token = Some(value.to_unescaped()?)
            }
            (key, Token::ValueString { value, .. }) if key.eq_ignore_ascii_case("Expiration") => {
                expiration = Some(value.to_unescaped()?)
            }

            _ => {}
        };
        Ok(())
    })?;

    match version {
        Some(1) => { /* continue */ }
        None => return Err(InvalidJsonCredentials::MissingField("Version")),
        Some(version) => {
            return Err(InvalidJsonCredentials::InvalidField {
                field: "version",
                err: format!("unknown version number: {}", version).into(),
            })
        }
    }

    let access_key_id = access_key_id.ok_or(InvalidJsonCredentials::MissingField("AccessKeyId"))?;
    let secret_access_key =
        secret_access_key.ok_or(InvalidJsonCredentials::MissingField("SecretAccessKey"))?;
    let expiration = expiration.map(parse_expiration).transpose()?;
    if expiration.is_none() {
        tracing::debug!("no expiration provided for credentials provider credentials. these credentials will never be refreshed.")
    }
    Ok(Credentials::new(
        access_key_id,
        secret_access_key,
        session_token.map(|tok| tok.to_string()),
        expiration,
        "CredentialProcess",
    ))
}

fn parse_expiration(expiration: impl AsRef<str>) -> Result<SystemTime, InvalidJsonCredentials> {
    OffsetDateTime::parse(expiration.as_ref(), &Rfc3339)
        .map(SystemTime::from)
        .map_err(|err| InvalidJsonCredentials::InvalidField {
            field: "Expiration",
            err: err.into(),
        })
}

#[cfg(test)]
mod test {
    use crate::credential_process::CredentialProcessProvider;
    use aws_credential_types::provider::ProvideCredentials;
    use std::time::{Duration, SystemTime};
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;
    use tokio::time::timeout;

    // TODO(https://github.com/awslabs/aws-sdk-rust/issues/1117) This test is ignored on Windows because it uses Unix-style paths
    #[tokio::test]
    #[cfg_attr(windows, ignore)]
    async fn test_credential_process() {
        let provider = CredentialProcessProvider::new(String::from(
            r#"echo '{ "Version": 1, "AccessKeyId": "ASIARTESTID", "SecretAccessKey": "TESTSECRETKEY", "SessionToken": "TESTSESSIONTOKEN", "Expiration": "2022-05-02T18:36:00+00:00" }'"#,
        ));
        let creds = provider.provide_credentials().await.expect("valid creds");
        assert_eq!(creds.access_key_id(), "ASIARTESTID");
        assert_eq!(creds.secret_access_key(), "TESTSECRETKEY");
        assert_eq!(creds.session_token(), Some("TESTSESSIONTOKEN"));
        assert_eq!(
            creds.expiry(),
            Some(
                SystemTime::try_from(
                    OffsetDateTime::parse("2022-05-02T18:36:00+00:00", &Rfc3339)
                        .expect("static datetime")
                )
                .expect("static datetime")
            )
        );
    }

    // TODO(https://github.com/awslabs/aws-sdk-rust/issues/1117) This test is ignored on Windows because it uses Unix-style paths
    #[tokio::test]
    #[cfg_attr(windows, ignore)]
    async fn test_credential_process_no_expiry() {
        let provider = CredentialProcessProvider::new(String::from(
            r#"echo '{ "Version": 1, "AccessKeyId": "ASIARTESTID", "SecretAccessKey": "TESTSECRETKEY" }'"#,
        ));
        let creds = provider.provide_credentials().await.expect("valid creds");
        assert_eq!(creds.access_key_id(), "ASIARTESTID");
        assert_eq!(creds.secret_access_key(), "TESTSECRETKEY");
        assert_eq!(creds.session_token(), None);
        assert_eq!(creds.expiry(), None);
    }

    #[tokio::test]
    async fn credentials_process_timeouts() {
        let provider = CredentialProcessProvider::new(String::from("sleep 1000"));
        let _creds = timeout(Duration::from_millis(1), provider.provide_credentials())
            .await
            .expect_err("timeout forced");
    }
}
