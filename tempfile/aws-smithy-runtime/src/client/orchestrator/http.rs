/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use aws_smithy_runtime_api::client::orchestrator::{HttpResponse, SensitiveOutput};
use aws_smithy_types::body::SdkBody;
use aws_smithy_types::config_bag::ConfigBag;
use bytes::{Buf, Bytes};
use http_body_04x::Body;
use pin_utils::pin_mut;
use tracing::trace;

const LOG_SENSITIVE_BODIES: &str = "LOG_SENSITIVE_BODIES";

async fn body_to_bytes(body: SdkBody) -> Result<Bytes, <SdkBody as Body>::Error> {
    let mut output = Vec::new();
    pin_mut!(body);
    while let Some(buf) = body.data().await {
        let mut buf = buf?;
        while buf.has_remaining() {
            output.extend_from_slice(buf.chunk());
            buf.advance(buf.chunk().len())
        }
    }

    Ok(Bytes::from(output))
}

pub(crate) async fn read_body(response: &mut HttpResponse) -> Result<(), <SdkBody as Body>::Error> {
    let mut body = SdkBody::taken();
    std::mem::swap(&mut body, response.body_mut());

    let bytes = body_to_bytes(body).await?;
    let mut body = SdkBody::from(bytes);
    std::mem::swap(&mut body, response.body_mut());

    Ok(())
}

pub(crate) fn log_response_body(response: &HttpResponse, cfg: &ConfigBag) {
    if cfg.load::<SensitiveOutput>().is_none()
        || std::env::var(LOG_SENSITIVE_BODIES)
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or_default()
    {
        trace!(response = ?response, "read HTTP response body");
    } else {
        trace!(
            response = "** REDACTED **. To print, set LOG_SENSITIVE_BODIES=true",
            "read HTTP response body"
        )
    }
}
