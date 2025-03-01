use alloc::string::String;

/// Error variants when converting from [crate::Transaction] to [alloy_consensus::Signed]
/// transaction.
#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    /// A custom Conversion Error that doesn't fit other categories.
    #[error("conversion error: {0}")]
    Custom(String),
}
