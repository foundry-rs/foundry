use crate::fmt::FormatterError;
use solar_interface::diagnostics::EmittedDiagnostics;
use thiserror::Error;

/// The parser error.
#[derive(Debug, Error)]
#[error(transparent)]
pub enum ParserError {
    /// Formatter error.
    #[error(transparent)]
    Formatter2(EmittedDiagnostics),
    /// Formatter error.
    #[error(transparent)]
    Formatter(#[from] FormatterError),
    /// Internal parser error.
    #[error(transparent)]
    Internal(#[from] eyre::Error),
}

/// The parser result.
pub type ParserResult<T, E = ParserError> = std::result::Result<T, E>;
