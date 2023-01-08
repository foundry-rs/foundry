use thiserror::Error;

/// The parser error.
#[derive(Error, Debug)]
#[error(transparent)]
pub struct ParserError(#[from] eyre::Error);

/// The parser result.
pub type ParserResult<T, E = ParserError> = std::result::Result<T, E>;
