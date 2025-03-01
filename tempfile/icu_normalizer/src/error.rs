// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! Normalizer-specific error

use displaydoc::Display;
use icu_properties::PropertiesError;
use icu_provider::DataError;

/// A list of error outcomes for various operations in this module.
///
/// Re-exported as [`Error`](crate::Error).
#[derive(Display, Debug)]
#[non_exhaustive]
pub enum NormalizerError {
    /// Error coming from the data provider
    #[displaydoc("{0}")]
    Data(DataError),
    /// The data uses a planned but unsupported feature.
    FutureExtension,
    /// Data failed manual validation
    ValidationError,
}

#[cfg(feature = "std")]
impl std::error::Error for NormalizerError {}

impl From<DataError> for NormalizerError {
    fn from(e: DataError) -> Self {
        NormalizerError::Data(e)
    }
}

impl From<PropertiesError> for NormalizerError {
    fn from(e: PropertiesError) -> Self {
        match e {
            PropertiesError::PropDataLoad(d) => NormalizerError::Data(d),
            _ => unreachable!("Shouldn't have non-Data PropertiesError"),
        }
    }
}
