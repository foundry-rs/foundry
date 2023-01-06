use std::fmt::{Display, Formatter};

use thiserror::Error;

/// The parser error
#[derive(Error, Debug)]
pub struct NoopError;

impl Display for NoopError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("noop;")
    }
}

/// A result with default noop error
pub type ParserResult<T, E = NoopError> = std::result::Result<T, E>;
