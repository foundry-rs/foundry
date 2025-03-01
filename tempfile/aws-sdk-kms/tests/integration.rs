/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use aws_sdk_kms as kms;
use aws_sdk_kms::operation::RequestId;
use aws_smithy_runtime::client::http::test_util::{ReplayEvent, StaticReplayClient};
use aws_smithy_runtime_api::client::result::SdkError;
use aws_smithy_types::body::SdkBody;
use http::Uri;
use kms::config::{Config, Credentials, Region};

// TODO(DVR): having the full HTTP requests right in the code is a bit gross, consider something
// like https://github.com/davidbarsky/sigv4/blob/master/aws-sigv4/src/lib.rs#L283-L315 to store
// the requests/responses externally

/// Validate that for CN regions we set the URI correctly
#[tokio::test]
async fn generate_random_cn() {
    let http_client= StaticReplayClient::new(vec![ReplayEvent::new(
        http::Request::builder()
            .uri(Uri::from_static("https://kms.cn-north-1.amazonaws.com.cn/"))
            .body(SdkBody::from(r#"{"NumberOfBytes":64}"#)).unwrap(),
        http::Response::builder()
            .status(http::StatusCode::from_u16(200).unwrap())
            .body(SdkBody::from(r#"{"Plaintext":"6CG0fbzzhg5G2VcFCPmJMJ8Njv3voYCgrGlp3+BZe7eDweCXgiyDH9BnkKvLmS7gQhnYDUlyES3fZVGwv5+CxA=="}"#)).unwrap())
    ]);
    let conf = Config::builder()
        .http_client(http_client.clone())
        .region(Region::new("cn-north-1"))
        .credentials_provider(Credentials::for_tests())
        .build();
    let client = kms::Client::from_conf(conf);
    let _ = client
        .generate_random()
        .number_of_bytes(64)
        .send()
        .await
        .expect("success");

    assert_eq!(http_client.actual_requests().count(), 1);
    http_client.assert_requests_match(&[]);
}

#[cfg(feature = "test-util")]
#[tokio::test]
async fn generate_random() {
    let http_client = StaticReplayClient::new(vec![ReplayEvent::new(
        http::Request::builder()
            .header("content-type", "application/x-amz-json-1.1")
            .header("x-amz-target", "TrentService.GenerateRandom")
            .header("content-length", "20")
            .header("authorization", "AWS4-HMAC-SHA256 Credential=ANOTREAL/20090213/us-east-1/kms/aws4_request, SignedHeaders=content-length;content-type;host;x-amz-date;x-amz-target;x-amz-user-agent, Signature=53dcf70f6f852cb576185dcabef5aaa3d068704cf1b7ea7dc644efeaa46674d7")
            .header("x-amz-date", "20090213T233130Z")
            .header("user-agent", "aws-sdk-rust/0.123.test os/windows/XPSP3 lang/rust/1.50.0")
            .header("x-amz-user-agent", "aws-sdk-rust/0.123.test api/test-service/0.123 os/windows/XPSP3 lang/rust/1.50.0")
            .uri(Uri::from_static("https://kms.us-east-1.amazonaws.com/"))
            .body(SdkBody::from(r#"{"NumberOfBytes":64}"#)).unwrap(),
        http::Response::builder()
            .status(http::StatusCode::from_u16(200).unwrap())
            .body(SdkBody::from(r#"{"Plaintext":"6CG0fbzzhg5G2VcFCPmJMJ8Njv3voYCgrGlp3+BZe7eDweCXgiyDH9BnkKvLmS7gQhnYDUlyES3fZVGwv5+CxA=="}"#)).unwrap())
    ]);
    let conf = Config::builder()
        .http_client(http_client.clone())
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::for_tests_with_session_token())
        .with_test_defaults()
        .build();
    let client = kms::Client::from_conf(conf);
    let resp = client
        .generate_random()
        .number_of_bytes(64)
        .customize()
        .mutate_request(|req| {
            // Remove the invocation ID since the signed request above doesn't have it
            req.headers_mut().remove("amz-sdk-invocation-id");
        })
        .send()
        .await
        .expect("request should succeed");
    // primitive checksum
    assert_eq!(
        resp.plaintext
            .expect("blob should exist")
            .as_ref()
            .iter()
            .map(|i| *i as u32)
            .sum::<u32>(),
        8562
    );
    http_client.relaxed_requests_match();
}

#[tokio::test]
async fn generate_random_malformed_response() {
    let http_client = StaticReplayClient::new(vec![ReplayEvent::new(
        http::Request::builder().body(SdkBody::from(r#"{"NumberOfBytes":64}"#)).unwrap(),
        http::Response::builder()
            .status(http::StatusCode::from_u16(200).unwrap())
            // last `}` replaced with a space, invalid JSON
            .body(SdkBody::from(r#"{"Plaintext":"6CG0fbzzhg5G2VcFCPmJMJ8Njv3voYCgrGlp3+BZe7eDweCXgiyDH9BnkKvLmS7gQhnYDUlyES3fZVGwv5+CxA==" "#)).unwrap())
    ]);
    let conf = Config::builder()
        .http_client(http_client.clone())
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::for_tests())
        .build();
    let client = kms::Client::from_conf(conf);
    client
        .generate_random()
        .number_of_bytes(64)
        .send()
        .await
        .expect_err("response was malformed");
}

#[cfg(feature = "test-util")]
#[tokio::test]
async fn generate_random_keystore_not_found() {
    let http_client = StaticReplayClient::new(vec![ReplayEvent::new(
        http::Request::builder()
            .header("content-type", "application/x-amz-json-1.1")
            .header("x-amz-target", "TrentService.GenerateRandom")
            .header("content-length", "56")
            .header("authorization", "AWS4-HMAC-SHA256 Credential=ANOTREAL/20090213/us-east-1/kms/aws4_request, SignedHeaders=content-length;content-type;host;x-amz-target, Signature=ffef92c6b75d66cc511daa896eb4a085ec053a2592e17d1f22ecaf167f2fa4bb")
            .header("x-amz-date", "20090213T233130Z")
            .header("user-agent", "aws-sdk-rust/0.123.test os/windows/XPSP3 lang/rust/1.50.0")
            .header("x-amz-user-agent", "aws-sdk-rust/0.123.test api/test-service/0.123 os/windows/XPSP3 lang/rust/1.50.0")
            .uri(Uri::from_static("https://kms.us-east-1.amazonaws.com/"))
            .body(SdkBody::from(r#"{"NumberOfBytes":64,"CustomKeyStoreId":"does not exist"}"#)).unwrap(),
        http::Response::builder()
            .status(http::StatusCode::from_u16(400).unwrap())
            .header("x-amzn-requestid", "bfe81a0a-9a08-4e71-9910-cdb5ab6ea3b6")
            .header("cache-control", "no-cache, no-store, must-revalidate, private")
            .header("expires", "0")
            .header("pragma", "no-cache")
            .header("date", "Fri, 05 Mar 2021 15:01:40 GMT")
            .header("content-type", "application/x-amz-json-1.1")
            .header("content-length", "44")
            .body(SdkBody::from(r#"{"__type":"CustomKeyStoreNotFoundException"}"#)).unwrap())
    ]);
    let conf = Config::builder()
        .http_client(http_client.clone())
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::for_tests_with_session_token())
        .with_test_defaults()
        .build();
    let client = kms::Client::from_conf(conf);

    let err = client
        .generate_random()
        .number_of_bytes(64)
        .custom_key_store_id("does not exist")
        .send()
        .await
        .expect_err("key store doesn't exist");

    let inner = match err {
        SdkError::ServiceError(context) => context.into_err(),
        other => panic!("Incorrect error received: {:}", other),
    };
    assert!(inner.is_custom_key_store_not_found_exception());
    assert_eq!(
        inner.request_id(),
        Some("bfe81a0a-9a08-4e71-9910-cdb5ab6ea3b6")
    );
    http_client.relaxed_requests_match();
}
