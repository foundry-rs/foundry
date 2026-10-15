//! AWS-LC backend for Nitro attestation verification.

use aws_lc_rs::{
    digest::{Digest as AwsLcDigest, SHA384 as AWS_LC_SHA384, digest as aws_lc_digest},
    signature::{
        ECDSA_P384_SHA384_ASN1, ECDSA_P384_SHA384_FIXED, ParsedPublicKey as AwsLcParsedPublicKey,
    },
};

use crate::{
    P384_FIXED_SIGNATURE_SIZE, P384_PUBLIC_KEY_SIZE, P384Verifier, SHA384_SIZE, Sha384Hasher,
};

/// AWS-LC backend for SHA-384 hashing and P-384 Nitro attestation verification.
pub struct AwsLcP384;

impl Sha384Hasher for AwsLcP384 {
    fn sha384(&self, input: &[u8]) -> [u8; SHA384_SIZE] {
        aws_lc_digest(&AWS_LC_SHA384, input)
            .as_ref()
            .try_into()
            .expect("SHA-384 has a fixed 48-byte output")
    }
}

impl P384Verifier for AwsLcP384 {
    fn validate_public_key(&self, public_key: &[u8; P384_PUBLIC_KEY_SIZE]) -> bool {
        AwsLcParsedPublicKey::new(&ECDSA_P384_SHA384_ASN1, public_key).is_ok()
    }

    fn verify_der(
        &self,
        public_key: &[u8; P384_PUBLIC_KEY_SIZE],
        digest: &[u8; SHA384_SIZE],
        signature_der: &[u8],
    ) -> bool {
        verify_digest(&ECDSA_P384_SHA384_ASN1, public_key, digest, signature_der)
    }

    fn verify_fixed(
        &self,
        public_key: &[u8; P384_PUBLIC_KEY_SIZE],
        digest: &[u8; SHA384_SIZE],
        signature: &[u8; P384_FIXED_SIGNATURE_SIZE],
    ) -> bool {
        verify_digest(&ECDSA_P384_SHA384_FIXED, public_key, digest, signature)
    }
}

fn verify_digest(
    algorithm: &'static aws_lc_rs::signature::EcdsaVerificationAlgorithm,
    public_key: &[u8; P384_PUBLIC_KEY_SIZE],
    digest: &[u8; SHA384_SIZE],
    signature: &[u8],
) -> bool {
    let Ok(public_key) = AwsLcParsedPublicKey::new(algorithm, public_key) else {
        return false;
    };
    let Ok(digest) = AwsLcDigest::import_less_safe(digest, &AWS_LC_SHA384) else {
        return false;
    };
    public_key.verify_digest_sig(&digest, signature).is_ok()
}
