use core::fmt;

/// Structural document errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatError {
    DocumentTooLarge,
    InvalidCbor,
    InvalidCoseTag,
    InvalidCoseStructure,
    InvalidProtectedHeader,
    InvalidPayload,
    InvalidField(&'static str),
}

/// Certificate profile and chain errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CertificateError {
    RootMismatch,
    InvalidDer { index: usize },
    InvalidVersion { index: usize },
    InvalidSignatureAlgorithm { index: usize },
    InvalidPublicKey { index: usize },
    InvalidValidity { index: usize },
    BrokenIssuerLink { index: usize },
    DuplicateExtension { index: usize },
    UnknownCriticalExtension { index: usize },
    InvalidBasicConstraints { index: usize },
    InvalidKeyUsage { index: usize },
    InvalidPathLength { index: usize },
}

/// Certificate or document signature failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureError {
    /// Signature on a non-root certificate. Index zero is the root, so this is always nonzero.
    Certificate { index: usize },
    /// COSE document signature.
    Document,
}

/// Categorized Nitro attestation validation error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidFormat(FormatError),
    InvalidCertificate(CertificateError),
    InvalidSignature(SignatureError),
}

impl From<FormatError> for Error {
    fn from(value: FormatError) -> Self {
        Self::InvalidFormat(value)
    }
}

impl From<CertificateError> for Error {
    fn from(value: CertificateError) -> Self {
        Self::InvalidCertificate(value)
    }
}

impl From<SignatureError> for Error {
    fn from(value: SignatureError) -> Self {
        Self::InvalidSignature(value)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat(reason) => write!(f, "invalid attestation format: {reason:?}"),
            Self::InvalidCertificate(reason) => write!(f, "invalid certificate: {reason:?}"),
            Self::InvalidSignature(reason) => write!(f, "invalid signature: {reason:?}"),
        }
    }
}
