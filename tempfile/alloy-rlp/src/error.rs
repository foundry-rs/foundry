use core::fmt;

/// RLP result type.
pub type Result<T, E = Error> = core::result::Result<T, E>;

/// RLP error type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Numeric Overflow.
    Overflow,
    /// Leading zero disallowed.
    LeadingZero,
    /// Overran input while decoding.
    InputTooShort,
    /// Expected single byte, but got invalid value.
    NonCanonicalSingleByte,
    /// Expected size, but got invalid value.
    NonCanonicalSize,
    /// Expected a payload of a specific size, got an unexpected size.
    UnexpectedLength,
    /// Expected another type, got a string instead.
    UnexpectedString,
    /// Expected another type, got a list instead.
    UnexpectedList,
    /// Got an unexpected number of items in a list.
    ListLengthMismatch {
        /// Expected length.
        expected: usize,
        /// Actual length.
        got: usize,
    },
    /// Custom error.
    Custom(&'static str),
}

#[cfg(all(feature = "core-net", not(feature = "std")))]
impl core::error::Error for Error {}
#[cfg(feature = "std")]
impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Overflow => f.write_str("overflow"),
            Self::LeadingZero => f.write_str("leading zero"),
            Self::InputTooShort => f.write_str("input too short"),
            Self::NonCanonicalSingleByte => f.write_str("non-canonical single byte"),
            Self::NonCanonicalSize => f.write_str("non-canonical size"),
            Self::UnexpectedLength => f.write_str("unexpected length"),
            Self::UnexpectedString => f.write_str("unexpected string"),
            Self::UnexpectedList => f.write_str("unexpected list"),
            Self::ListLengthMismatch { got, expected } => {
                write!(f, "unexpected list length (got {got}, expected {expected})")
            }
            Self::Custom(err) => f.write_str(err),
        }
    }
}
