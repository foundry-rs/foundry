use core::fmt;
use ruint::BaseConvertError;

/// The error type that is returned when parsing a signed integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseSignedError {
    /// Error that occurs when an invalid digit is encountered while parsing.
    Ruint(ruint::ParseError),

    /// Error that occurs when the number is too large or too small (negative)
    /// and does not fit in the target signed integer.
    IntegerOverflow,
}

impl From<ruint::ParseError> for ParseSignedError {
    fn from(err: ruint::ParseError) -> Self {
        // these errors are redundant, so we coerce the more complex one to the
        // simpler one
        match err {
            ruint::ParseError::BaseConvertError(BaseConvertError::Overflow) => {
                Self::IntegerOverflow
            }
            _ => Self::Ruint(err),
        }
    }
}

impl core::error::Error for ParseSignedError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            #[cfg(feature = "std")]
            Self::Ruint(err) => Some(err),
            _ => None,
        }
    }
}

impl fmt::Display for ParseSignedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ruint(e) => e.fmt(f),
            Self::IntegerOverflow => f.write_str("number does not fit in the integer size"),
        }
    }
}

/// The error type that is returned when conversion to or from a integer fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BigIntConversionError;

impl core::error::Error for BigIntConversionError {}

impl fmt::Display for BigIntConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("output of range integer conversion attempted")
    }
}
