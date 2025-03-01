Where did the files in this directory come from?
================================================

These test files were taken from the [aws-c-auth](https://github.com/awslabs/aws-c-auth/tree/main/tests/aws-signing-test-suite/v4a) project.

Signature Version 4A Test Suite
------------------------------

To assist you in the development of an AWS client that supports Signature Version 4A, you can use the
files in the test suite to ensure your code is performing each step of the signing process correctly.

Each test group contains several files that you can use to validate most of the tasks described in
Signature Version 4A Signing Process. The following list describes the contents of each file.

- context.json - Credentials and signer options to use when signing test requests
- request.txt - The web request to be signed.
- header-canonical-request.txt - The resulting canonical request in header-signature mode.
- header-string-to-sign.txt - The resulting string to sign in header-signature mode.
- query-canonical-request.txt - The resulting canonical request in query-signature mode.
- query-string-to-sign.txt - The resulting string to sign in header-query mode.

Sigv4A signature generation isn't deterministic, so generated signatures can't be tested against known good ones.
Instead, tests generate a signature, derive a verification key from the signing key, and verify the signature and
the string to sign. This mirrors what AWS services do when verifying Sigv4A-signed requests.
