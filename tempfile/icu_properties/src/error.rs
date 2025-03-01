// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use displaydoc::Display;
use icu_provider::DataError;

#[cfg(feature = "std")]
impl std::error::Error for PropertiesError {}

/// A list of error outcomes for various operations in this module.
///
/// Re-exported as [`Error`](crate::Error).
#[derive(Display, Debug, Copy, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PropertiesError {
    /// An error occurred while loading data
    #[displaydoc("{0}")]
    PropDataLoad(DataError),
    /// An unknown value was used for the [`Script`](super::Script) property
    #[displaydoc("Unknown script id: {0}")]
    UnknownScriptId(u16),
    /// An unknown value was used for the [`GeneralCategoryGroup`](super::GeneralCategoryGroup) property
    #[displaydoc("Unknown general category group: {0}")]
    UnknownGeneralCategoryGroup(u32),
    /// An unknown or unexpected property name was used for an API dealing with properties specified as strings at runtime
    #[displaydoc("Unexpected or unknown property name")]
    UnexpectedPropertyName,
}

impl From<DataError> for PropertiesError {
    fn from(e: DataError) -> Self {
        PropertiesError::PropDataLoad(e)
    }
}
