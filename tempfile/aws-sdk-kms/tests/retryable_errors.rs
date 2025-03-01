/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use aws_credential_types::Credentials;
use aws_runtime::retries::classifiers::AwsErrorCodeClassifier;
use aws_sdk_kms as kms;
use aws_smithy_runtime::client::http::test_util::infallible_client_fn;
use aws_smithy_runtime_api::client::interceptors::context::{Error, Input, InterceptorContext};
use aws_smithy_runtime_api::client::orchestrator::{HttpResponse, OrchestratorError};
use aws_smithy_runtime_api::client::result::SdkError;
use aws_smithy_runtime_api::client::retries::classifiers::{ClassifyRetry, RetryAction};
use bytes::Bytes;
use kms::operation::create_alias::CreateAliasError;

async fn make_err(
    response: impl Fn() -> http::Response<Bytes> + Send + Sync + 'static,
) -> SdkError<CreateAliasError, HttpResponse> {
    let http_client = infallible_client_fn(move |_| response());
    let conf = kms::Config::builder()
        .http_client(http_client)
        .credentials_provider(Credentials::for_tests())
        .region(kms::config::Region::from_static("us-east-1"))
        .build();
    let client = kms::Client::from_conf(conf);
    client
        .create_alias()
        .send()
        .await
        .expect_err("response was a failure")
}

/// Parse a semi-real response body and assert that the correct retry status is returned
#[tokio::test]
async fn errors_are_retryable() {
    let err = make_err(|| {
        http::Response::builder()
            .status(400)
            .body(Bytes::from_static(
                br#"{ "code": "LimitExceededException" }"#,
            ))
            .unwrap()
    })
    .await;

    dbg!(&err);
    let classifier = AwsErrorCodeClassifier::<CreateAliasError>::new();
    let mut ctx = InterceptorContext::new(Input::doesnt_matter());
    let err = err.into_service_error();
    ctx.set_output_or_error(Err(OrchestratorError::operation(Error::erase(err))));
    let retry_action = classifier.classify_retry(&ctx);
    assert_eq!(RetryAction::throttling_error(), retry_action);
}

#[tokio::test]
async fn unmodeled_errors_are_retryable() {
    let err = make_err(|| {
        http::Response::builder()
            .status(400)
            .body(Bytes::from_static(br#"{ "code": "ThrottlingException" }"#))
            .unwrap()
    })
    .await;

    dbg!(&err);
    let classifier = AwsErrorCodeClassifier::<CreateAliasError>::new();
    let mut ctx = InterceptorContext::new(Input::doesnt_matter());
    let err = err.into_service_error();
    ctx.set_output_or_error(Err(OrchestratorError::operation(Error::erase(err))));
    let retry_action = classifier.classify_retry(&ctx);
    assert_eq!(RetryAction::throttling_error(), retry_action);
}
