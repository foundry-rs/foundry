//! Modified from `hex::error`.

use core::fmt;

/// The error type for decoding a hex string into `Vec<u8>` or `[u8; N]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)]
pub enum FromHexError {
    /// An invalid character was found. Valid ones are: `0...9`, `a...f`
    /// or `A...F`.
    #[allow(missing_docs)]
    InvalidHexCharacter { c: char, index: usize },

    /// A hex string's length needs to be even, as two digits correspond to
    /// one byte.
    OddLength,

    /// If the hex string is decoded into a fixed sized container, such as an
    /// array, the hex string's length * 2 has to match the container's
    /// length.
    InvalidStringLength,
}

#[cfg(feature = "core-error")]
impl core::error::Error for FromHexError {}
#[cfg(all(feature = "std", not(feature = "core-error")))]
impl std::error::Error for FromHexError {}

impl fmt::Display for FromHexError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::InvalidHexCharacter { c, index } => {
                write!(f, "invalid character {c:?} at position {index}")
            }
            Self::OddLength => f.write_str("odd number of digits"),
            Self::InvalidStringLength => f.write_str("invalid string length"),
        }
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn test_display() {
        assert_eq!(
            FromHexError::InvalidHexCharacter { c: '\n', index: 5 }.to_string(),
            "invalid character '\\n' at position 5"
        );

        assert_eq!(FromHexError::OddLength.to_string(), "odd number of digits");
        assert_eq!(
            FromHexError::InvalidStringLength.to_string(),
            "invalid string length"
        );
    }
}
