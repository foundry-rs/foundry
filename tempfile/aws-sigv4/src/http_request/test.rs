/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Functions shared between the tests of several modules.

use crate::http_request::{SignableBody, SignableRequest};
use http0::{Method, Uri};
use std::error::Error as StdError;

pub(crate) mod v4 {
    use super::*;

    fn path(name: &str, ext: &str) -> String {
        format!("aws-sig-v4-test-suite/{}/{}.{}", name, name, ext)
    }

    pub(crate) fn test_canonical_request(name: &str) -> String {
        // Tests fail if there's a trailing newline in the file, and pre-commit requires trailing newlines
        read(&path(name, "creq")).trim().to_string()
    }

    pub(crate) fn test_sts(name: &str) -> String {
        read(&path(name, "sts"))
    }

    pub(crate) fn test_request(name: &str) -> TestRequest {
        test_parsed_request(name, "req")
    }

    pub(crate) fn test_signed_request(name: &str) -> TestRequest {
        test_parsed_request(name, "sreq")
    }

    pub(crate) fn test_signed_request_query_params(name: &str) -> TestRequest {
        test_parsed_request(name, "qpsreq")
    }

    fn test_parsed_request(name: &str, ext: &str) -> TestRequest {
        let path = path(name, ext);
        match parse_request(read(&path).as_bytes()) {
            Ok(parsed) => parsed,
            Err(err) => panic!("Failed to parse {}: {}", path, err),
        }
    }

    #[test]
    fn test_parse() {
        test_request("post-header-key-case");
    }

    #[test]
    fn test_read_query_params() {
        test_request("get-vanilla-query-order-key-case");
    }
}

#[cfg(feature = "sigv4a")]
pub(crate) mod v4a {
    use super::*;
    use crate::http_request::{
        PayloadChecksumKind, SessionTokenMode, SignatureLocation, SigningSettings,
    };
    use aws_credential_types::Credentials;
    use aws_smithy_runtime_api::client::identity::Identity;
    use serde_derive::Deserialize;
    use std::time::{Duration, SystemTime};
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;

    fn path(test_name: &str, definition_name: &str) -> String {
        format!("aws-sig-v4a-test-suite/{test_name}/{definition_name}.txt")
    }

    pub(crate) fn test_request(name: &str) -> TestRequest {
        test_parsed_request(&path(name, "request"))
    }

    pub(crate) fn test_canonical_request(
        name: &str,
        signature_location: SignatureLocation,
    ) -> String {
        match signature_location {
            SignatureLocation::QueryParams => read(&path(name, "query-canonical-request")),
            SignatureLocation::Headers => read(&path(name, "header-canonical-request")),
        }
    }

    pub(crate) fn test_string_to_sign(name: &str, signature_location: SignatureLocation) -> String {
        match signature_location {
            SignatureLocation::QueryParams => read(&path(name, "query-string-to-sign")),
            SignatureLocation::Headers => read(&path(name, "header-string-to-sign")),
        }
    }

    fn test_parsed_request(path: &str) -> TestRequest {
        match parse_request(read(path).as_bytes()) {
            Ok(parsed) => parsed,
            Err(err) => panic!("Failed to parse {}: {}", path, err),
        }
    }

    pub(crate) fn test_context(test_name: &str) -> TestContext {
        let path = format!("aws-sig-v4a-test-suite/{test_name}/context.json");
        let context = read(&path);
        let tc_builder: TestContextBuilder = serde_json::from_str(&context).unwrap();
        tc_builder.build()
    }

    pub(crate) struct TestContext {
        pub(crate) identity: Identity,
        pub(crate) expiration_in_seconds: u64,
        pub(crate) normalize: bool,
        pub(crate) region: String,
        pub(crate) service: String,
        pub(crate) timestamp: String,
        pub(crate) omit_session_token: bool,
        pub(crate) sign_body: bool,
    }

    impl<'a> From<&'a TestContext> for crate::sign::v4a::SigningParams<'a, SigningSettings> {
        fn from(tc: &'a TestContext) -> Self {
            crate::sign::v4a::SigningParams {
                identity: &tc.identity,
                region_set: &tc.region,
                name: &tc.service,
                time: OffsetDateTime::parse(&tc.timestamp, &Rfc3339)
                    .unwrap()
                    .into(),
                settings: SigningSettings {
                    // payload_checksum_kind: PayloadChecksumKind::XAmzSha256,
                    expires_in: Some(Duration::from_secs(tc.expiration_in_seconds)),
                    uri_path_normalization_mode: tc.normalize.into(),
                    session_token_mode: if tc.omit_session_token {
                        SessionTokenMode::Exclude
                    } else {
                        SessionTokenMode::Include
                    },
                    payload_checksum_kind: if tc.sign_body {
                        PayloadChecksumKind::XAmzSha256
                    } else {
                        PayloadChecksumKind::NoHeader
                    },
                    ..Default::default()
                },
            }
        }
    }

    // Serde has limitations requiring this odd workaround.
    // See https://github.com/serde-rs/serde/issues/368 for more info.
    fn return_true() -> bool {
        true
    }

    #[derive(Deserialize)]
    pub(crate) struct TestContextBuilder {
        credentials: TestContextCreds,
        expiration_in_seconds: u64,
        normalize: bool,
        region: String,
        service: String,
        timestamp: String,
        #[serde(default)]
        omit_session_token: bool,
        #[serde(default = "return_true")]
        sign_body: bool,
    }

    impl TestContextBuilder {
        pub(crate) fn build(self) -> TestContext {
            let identity = Identity::new(
                Credentials::from_keys(
                    &self.credentials.access_key_id,
                    &self.credentials.secret_access_key,
                    self.credentials.token.clone(),
                ),
                Some(SystemTime::UNIX_EPOCH + Duration::from_secs(self.expiration_in_seconds)),
            );

            TestContext {
                identity,
                expiration_in_seconds: self.expiration_in_seconds,
                normalize: self.normalize,
                region: self.region,
                service: self.service,
                timestamp: self.timestamp,
                omit_session_token: self.omit_session_token,
                sign_body: self.sign_body,
            }
        }
    }

    #[derive(Deserialize)]
    pub(crate) struct TestContextCreds {
        access_key_id: String,
        secret_access_key: String,
        token: Option<String>,
    }

    #[test]
    fn test_parse() {
        let req = test_request("post-header-key-case");
        assert_eq!(req.method, "POST");
        assert_eq!(req.uri, "https://example.amazonaws.com/");
        assert!(req.headers.is_empty());
    }

    #[test]
    fn test_read_query_params() {
        let req = test_request("get-header-value-trim");
        assert_eq!(req.method, "GET");
        assert_eq!(req.uri, "https://example.amazonaws.com/");
        assert!(!req.headers.is_empty());
    }
}

fn read(path: &str) -> String {
    println!("Loading `{}` for test case...", path);
    let v = {
        match std::fs::read_to_string(path) {
            // This replacement is necessary for tests to pass on Windows, as reading the
            // test snapshots from the file system results in CRLF line endings being inserted.
            Ok(value) => value.replace("\r\n", "\n"),
            Err(err) => {
                panic!("failed to load test case `{}`: {}", path, err);
            }
        }
    };

    v.trim().to_string()
}

pub(crate) struct TestRequest {
    pub(crate) uri: String,
    pub(crate) method: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: TestSignedBody,
}

pub(crate) enum TestSignedBody {
    Signable(SignableBody<'static>),
    Bytes(Vec<u8>),
}

impl TestSignedBody {
    fn as_signable_body(&self) -> SignableBody<'_> {
        match self {
            TestSignedBody::Signable(data) => data.clone(),
            TestSignedBody::Bytes(data) => SignableBody::Bytes(data.as_slice()),
        }
    }
}

impl TestRequest {
    pub(crate) fn set_body(&mut self, body: SignableBody<'static>) {
        self.body = TestSignedBody::Signable(body);
    }

    pub(crate) fn as_http_request(&self) -> http0::Request<&'static str> {
        let mut builder = http0::Request::builder()
            .uri(&self.uri)
            .method(Method::from_bytes(self.method.as_bytes()).unwrap());
        for (k, v) in &self.headers {
            builder = builder.header(k, v);
        }
        builder.body("body").unwrap()
    }
}

impl<B: AsRef<[u8]>> From<http0::Request<B>> for TestRequest {
    fn from(value: http0::Request<B>) -> Self {
        let invalid = value
            .headers()
            .values()
            .find(|h| std::str::from_utf8(h.as_bytes()).is_err());
        if let Some(invalid) = invalid {
            panic!("invalid header: {:?}", invalid);
        }
        Self {
            uri: value.uri().to_string(),
            method: value.method().to_string(),
            headers: value
                .headers()
                .iter()
                .map(|(k, v)| {
                    (
                        k.to_string(),
                        String::from_utf8(v.as_bytes().to_vec()).unwrap(),
                    )
                })
                .collect::<Vec<_>>(),
            body: TestSignedBody::Bytes(value.body().as_ref().to_vec()),
        }
    }
}

impl<'a> From<&'a TestRequest> for SignableRequest<'a> {
    fn from(request: &'a TestRequest) -> SignableRequest<'a> {
        SignableRequest::new(
            &request.method,
            &request.uri,
            request
                .headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
            request.body.as_signable_body(),
        )
        .expect("URI MUST be valid")
    }
}

fn parse_request(s: &[u8]) -> Result<TestRequest, Box<dyn StdError + Send + Sync + 'static>> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    // httparse 1.5 requires two trailing newlines to head the header section.
    let mut with_newline = Vec::from(s);
    with_newline.push(b'\n');
    let mut req = httparse::Request::new(&mut headers);
    let _ = req.parse(&with_newline).unwrap();

    let mut uri_builder = Uri::builder().scheme("https");
    if let Some(path) = req.path {
        uri_builder = uri_builder.path_and_query(path);
    }

    let mut headers = vec![];
    for header in req.headers {
        let name = header.name.to_lowercase();
        if name == "host" {
            uri_builder = uri_builder.authority(header.value);
        } else if !name.is_empty() {
            headers.push((
                header.name.to_string(),
                std::str::from_utf8(header.value)?.to_string(),
            ));
        }
    }

    Ok(TestRequest {
        uri: uri_builder.build()?.to_string(),
        method: req.method.unwrap().to_string(),
        headers,
        body: TestSignedBody::Bytes(vec![]),
    })
}

#[test]
fn test_parse_headers() {
    let buf = b"Host:example.amazonaws.com\nX-Amz-Date:20150830T123600Z\n\nblah blah";
    let mut headers = [httparse::EMPTY_HEADER; 4];
    assert_eq!(
        httparse::parse_headers(buf, &mut headers),
        Ok(httparse::Status::Complete((
            56,
            &[
                httparse::Header {
                    name: "Host",
                    value: b"example.amazonaws.com",
                },
                httparse::Header {
                    name: "X-Amz-Date",
                    value: b"20150830T123600Z",
                }
            ][..]
        )))
    );
}
