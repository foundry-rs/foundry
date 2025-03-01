/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use aws_sdk_kms as kms;
use kms::operation::generate_random::GenerateRandomOutput;
use kms::primitives::Blob;

#[test]
fn validate_sensitive_trait() {
    let builder = GenerateRandomOutput::builder().plaintext(Blob::new("some output"));
    assert_eq!(
        format!("{:?}", builder),
        "GenerateRandomOutputBuilder { plaintext: \"*** Sensitive Data Redacted ***\", ciphertext_for_recipient: None, _request_id: None }"
    );
    let output = GenerateRandomOutput::builder()
        .plaintext(Blob::new("some output"))
        .build();
    assert_eq!(
        format!("{:?}", output),
        "GenerateRandomOutput { plaintext: \"*** Sensitive Data Redacted ***\", ciphertext_for_recipient: None, _request_id: None }"
    );
}
