use crate::{
    CertificateError, P384_PUBLIC_KEY_SIZE, P384Verifier, ParsedAttestation, Sha384Hasher,
    SignatureError,
};
use alloc::vec::Vec;
use x509_cert::{
    Certificate, Version,
    der::{
        Decode, Encode, Reader,
        asn1::{AnyRef, ObjectIdentifier, UintRef},
    },
    ext::pkix::{BasicConstraints, KeyUsage},
};

type Result<T> = core::result::Result<T, crate::Error>;

const ECDSA_WITH_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");
const ID_EC_PUBLIC_KEY: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const SECP384R1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.132.0.34");
const BASIC_CONSTRAINTS: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.19");
const KEY_USAGE: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.15");

struct CheckedCertificate {
    public_key: [u8; P384_PUBLIC_KEY_SIZE],
    tbs_der: Vec<u8>,
    signature_der: Vec<u8>,
    issuer_der: Vec<u8>,
    subject_der: Vec<u8>,
    path_len_constraint: Option<usize>,
}

pub(crate) fn validate_chain<V: P384Verifier + Sha384Hasher>(
    parsed: &ParsedAttestation,
    block_timestamp: u64,
    pinned_root_der: &[u8],
    verifier: &V,
) -> Result<[u8; P384_PUBLIC_KEY_SIZE]> {
    if parsed.cabundle.first().map(Vec::as_slice) != Some(pinned_root_der) {
        return Err(CertificateError::RootMismatch.into());
    }

    // Keep the verification path in parent-first order: root, intermediates, leaf.
    let bundle_len = parsed.cabundle.len();
    let mut path = Vec::with_capacity(bundle_len + 1);
    for (index, der) in parsed.cabundle.iter().enumerate() {
        path.push(check_certificate(
            der,
            index,
            CertificateRole::Ca,
            block_timestamp,
            verifier,
        )?);
    }
    path.push(check_certificate(
        &parsed.certificate,
        bundle_len,
        CertificateRole::Leaf,
        block_timestamp,
        verifier,
    )?);

    // Compare the exact DER encoding of child issuer and parent subject at each edge.
    for child_index in 1..path.len() {
        if path[child_index].issuer_der != path[child_index - 1].subject_der {
            return Err(CertificateError::BrokenIssuerLink { index: child_index }.into());
        }
    }

    // RFC 5280 counts only non-self-issued intermediate CA certificates beneath a CA for
    // pathLenConstraint. The leaf is excluded even when it happens to be self-issued.
    for ca_index in 0..bundle_len {
        if let Some(constraint) = path[ca_index].path_len_constraint {
            let ca_below = path[ca_index + 1..bundle_len]
                .iter()
                .filter(|certificate| certificate.subject_der != certificate.issuer_der)
                .count();
            if ca_below > constraint {
                return Err(CertificateError::InvalidPathLength { index: ca_index }.into());
            }
        }
    }

    // Root self-signature is intentionally not checked: trust comes from byte-identical pinning.
    for child_index in 1..path.len() {
        let digest = verifier.sha384(&path[child_index].tbs_der);
        if !verifier.verify_der(
            &path[child_index - 1].public_key,
            &digest,
            &path[child_index].signature_der,
        ) {
            return Err(SignatureError::Certificate { index: child_index }.into());
        }
    }

    path.last()
        .map(|certificate| certificate.public_key)
        .ok_or_else(|| CertificateError::InvalidDer { index: 0 }.into())
}

#[derive(Clone, Copy)]
enum CertificateRole {
    Ca,
    Leaf,
}

fn check_certificate<V: P384Verifier>(
    der: &[u8],
    index: usize,
    role: CertificateRole,
    block_timestamp: u64,
    verifier: &V,
) -> Result<CheckedCertificate> {
    let certificate = Certificate::from_der(der)
        .map_err(|_| crate::Error::from(CertificateError::InvalidDer { index }))?;
    // `der` normalizes SET OF values while decoding. Consensus validation must reject BER-like
    // or otherwise non-canonical input instead of verifying a normalized re-encoding.
    if certificate
        .to_der()
        .map_err(|_| CertificateError::InvalidDer { index })?
        != der
    {
        return Err(CertificateError::InvalidDer { index }.into());
    }

    if certificate.tbs_certificate.version != Version::V3 {
        return Err(CertificateError::InvalidVersion { index }.into());
    }
    validate_signature_algorithm(&certificate, index)?;
    validate_validity(&certificate, index, block_timestamp)?;
    let path_len_constraint = validate_extensions(&certificate, index, role)?;

    let public_key = extract_public_key(&certificate, index)?;
    if !verifier.validate_public_key(&public_key) {
        return Err(CertificateError::InvalidPublicKey { index }.into());
    }

    let signature_der = certificate
        .signature
        .as_bytes()
        .ok_or(CertificateError::InvalidDer { index })?
        .to_vec();
    validate_ecdsa_der_signature(&signature_der, index)?;

    Ok(CheckedCertificate {
        public_key,
        tbs_der: certificate
            .tbs_certificate
            .to_der()
            .map_err(|_| CertificateError::InvalidDer { index })?,
        signature_der,
        issuer_der: certificate
            .tbs_certificate
            .issuer
            .to_der()
            .map_err(|_| CertificateError::InvalidDer { index })?,
        subject_der: certificate
            .tbs_certificate
            .subject
            .to_der()
            .map_err(|_| CertificateError::InvalidDer { index })?,
        path_len_constraint,
    })
}

fn validate_signature_algorithm(certificate: &Certificate, index: usize) -> Result<()> {
    let outer = &certificate.signature_algorithm;
    let inner = &certificate.tbs_certificate.signature;
    if outer.oid != ECDSA_WITH_SHA384
        || inner.oid != ECDSA_WITH_SHA384
        || outer.parameters.is_some()
        || inner.parameters.is_some()
        || outer != inner
    {
        return Err(CertificateError::InvalidSignatureAlgorithm { index }.into());
    }
    Ok(())
}

fn validate_validity(certificate: &Certificate, index: usize, block_timestamp: u64) -> Result<()> {
    let validity = certificate.tbs_certificate.validity;
    let not_before = validity.not_before.to_unix_duration().as_secs();
    let not_after = validity.not_after.to_unix_duration().as_secs();
    if block_timestamp < not_before || block_timestamp > not_after {
        return Err(CertificateError::InvalidValidity { index }.into());
    }
    Ok(())
}

fn validate_extensions(
    certificate: &Certificate,
    index: usize,
    role: CertificateRole,
) -> Result<Option<usize>> {
    let extensions = certificate
        .tbs_certificate
        .extensions
        .as_deref()
        .unwrap_or(&[]);

    for (position, extension) in extensions.iter().enumerate() {
        if extensions[..position]
            .iter()
            .any(|prior| prior.extn_id == extension.extn_id)
        {
            return Err(CertificateError::DuplicateExtension { index }.into());
        }
        if extension.critical
            && extension.extn_id != BASIC_CONSTRAINTS
            && extension.extn_id != KEY_USAGE
        {
            return Err(CertificateError::UnknownCriticalExtension { index }.into());
        }
    }

    let basic = certificate
        .tbs_certificate
        .get::<BasicConstraints>()
        .map_err(|_| CertificateError::InvalidBasicConstraints { index })?;
    let key_usage = certificate
        .tbs_certificate
        .get::<KeyUsage>()
        .map_err(|_| CertificateError::InvalidKeyUsage { index })?;

    let path_len_constraint = match role {
        CertificateRole::Ca => {
            let (critical, basic) =
                basic.ok_or(CertificateError::InvalidBasicConstraints { index })?;
            if !critical || !basic.ca {
                return Err(CertificateError::InvalidBasicConstraints { index }.into());
            }

            let (_, usage) = key_usage.ok_or(CertificateError::InvalidKeyUsage { index })?;
            if !usage.key_cert_sign() {
                return Err(CertificateError::InvalidKeyUsage { index }.into());
            }
            basic.path_len_constraint.map(usize::from)
        }
        CertificateRole::Leaf => {
            if basic.is_some_and(|(_, constraints)| {
                constraints.ca || constraints.path_len_constraint.is_some()
            }) {
                return Err(CertificateError::InvalidBasicConstraints { index }.into());
            }

            let (_, usage) = key_usage.ok_or(CertificateError::InvalidKeyUsage { index })?;
            if !usage.digital_signature() {
                return Err(CertificateError::InvalidKeyUsage { index }.into());
            }
            None
        }
    };
    Ok(path_len_constraint)
}

fn extract_public_key(
    certificate: &Certificate,
    index: usize,
) -> Result<[u8; P384_PUBLIC_KEY_SIZE]> {
    let spki = &certificate.tbs_certificate.subject_public_key_info;
    if spki.algorithm.oid != ID_EC_PUBLIC_KEY {
        return Err(CertificateError::InvalidPublicKey { index }.into());
    }
    let curve = spki
        .algorithm
        .parameters
        .as_ref()
        .ok_or(CertificateError::InvalidPublicKey { index })?
        .decode_as::<ObjectIdentifier>()
        .map_err(|_| CertificateError::InvalidPublicKey { index })?;
    if curve != SECP384R1 {
        return Err(CertificateError::InvalidPublicKey { index }.into());
    }

    let bytes = spki
        .subject_public_key
        .as_bytes()
        .ok_or(CertificateError::InvalidPublicKey { index })?;
    let key: [u8; P384_PUBLIC_KEY_SIZE] = bytes
        .try_into()
        .map_err(|_| CertificateError::InvalidPublicKey { index })?;
    if key[0] != 0x04 {
        return Err(CertificateError::InvalidPublicKey { index }.into());
    }
    Ok(key)
}

fn validate_ecdsa_der_signature(signature: &[u8], index: usize) -> Result<()> {
    let signature =
        AnyRef::from_der(signature).map_err(|_| CertificateError::InvalidDer { index })?;
    signature
        .sequence(|reader| {
            let r: UintRef<'_> = reader.decode()?;
            let s: UintRef<'_> = reader.decode()?;
            if r.is_empty() || s.is_empty() || r.as_bytes().len() > 48 || s.as_bytes().len() > 48 {
                return Err(x509_cert::der::Tag::Integer.value_error());
            }
            Ok(())
        })
        .map_err(|_| CertificateError::InvalidDer { index })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn accepts_strict_der_ecdsa_signature_shape() {
        // SEQUENCE { INTEGER 1, INTEGER 2 }
        assert!(validate_ecdsa_der_signature(&[0x30, 6, 2, 1, 1, 2, 1, 2], 1).is_ok());
    }

    #[test]
    fn rejects_trailing_or_non_canonical_signature_der() {
        assert!(validate_ecdsa_der_signature(&[0x30, 6, 2, 1, 1, 2, 1, 2, 0], 1).is_err());
        // INTEGER 1 with a redundant leading zero.
        assert!(validate_ecdsa_der_signature(&[0x30, 7, 2, 2, 0, 1, 2, 1, 2], 1).is_err());
    }
}
