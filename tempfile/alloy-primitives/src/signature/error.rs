use core::convert::Infallible;

/// Errors in signature parsing or verification.
#[derive(Debug)]
#[cfg_attr(not(feature = "k256"), derive(Copy, Clone))]
pub enum SignatureError {
    /// Error converting from bytes.
    FromBytes(&'static str),

    /// Error converting hex to bytes.
    FromHex(hex::FromHexError),

    /// Invalid parity.
    InvalidParity(u64),

    /// k256 error
    #[cfg(feature = "k256")]
    K256(k256::ecdsa::Error),
}

#[cfg(feature = "k256")]
impl From<k256::ecdsa::Error> for SignatureError {
    fn from(err: k256::ecdsa::Error) -> Self {
        Self::K256(err)
    }
}

impl From<hex::FromHexError> for SignatureError {
    fn from(err: hex::FromHexError) -> Self {
        Self::FromHex(err)
    }
}

impl core::error::Error for SignatureError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            #[cfg(all(feature = "k256", feature = "std"))]
            Self::K256(e) => Some(e),
            #[cfg(any(feature = "std", not(feature = "hex-compat")))]
            Self::FromHex(e) => Some(e),
            _ => None,
        }
    }
}

impl core::fmt::Display for SignatureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            #[cfg(feature = "k256")]
            Self::K256(e) => e.fmt(f),
            Self::FromBytes(e) => f.write_str(e),
            Self::FromHex(e) => e.fmt(f),
            Self::InvalidParity(v) => write!(f, "invalid parity: {v}"),
        }
    }
}

impl From<Infallible> for SignatureError {
    fn from(_: Infallible) -> Self {
        unreachable!()
    }
}
