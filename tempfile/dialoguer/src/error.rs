use std::{io::Error as IoError, result::Result as StdResult};

use thiserror::Error;

/// Possible errors returned by prompts.
#[derive(Error, Debug)]
pub enum Error {
    /// Error while executing IO operations.
    #[error("IO error: {0}")]
    IO(#[from] IoError),
}

/// Result type where errors are of type [Error](crate::error::Error)
pub type Result<T = ()> = StdResult<T, Error>;
