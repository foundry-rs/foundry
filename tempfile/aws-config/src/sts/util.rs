/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use aws_credential_types::provider::{self, error::CredentialsError};
use aws_credential_types::Credentials as AwsCredentials;
use aws_sdk_sts::types::Credentials as StsCredentials;

use std::time::{SystemTime, UNIX_EPOCH};

/// Convert STS credentials to aws_auth::Credentials
pub(crate) fn into_credentials(
    sts_credentials: Option<StsCredentials>,
    provider_name: &'static str,
) -> provider::Result {
    let sts_credentials = sts_credentials
        .ok_or_else(|| CredentialsError::unhandled("STS credentials must be defined"))?;
    let expiration = SystemTime::try_from(sts_credentials.expiration).map_err(|_| {
        CredentialsError::unhandled(
            "credential expiration time cannot be represented by a SystemTime",
        )
    })?;
    Ok(AwsCredentials::new(
        sts_credentials.access_key_id,
        sts_credentials.secret_access_key,
        Some(sts_credentials.session_token),
        Some(expiration),
        provider_name,
    ))
}

/// Create a default STS session name
///
/// STS Assume Role providers MUST assign a name to their generated session. When a user does not
/// provide a name for the session, the provider will choose a name composed of a base + a timestamp,
/// e.g. `profile-file-provider-123456789`
pub(crate) fn default_session_name(base: &str, ts: SystemTime) -> String {
    let now = ts.duration_since(UNIX_EPOCH).expect("post epoch");
    format!("{}-{}", base, now.as_millis())
}
