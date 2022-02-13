use crate::cmd::Target;
use std::{io, path::PathBuf};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, SandError>;

/// Various error types
#[derive(Debug, Error)]
pub enum SandError {
    #[error("Could not find cairo-lang version for \"{0}\"")]
    VersionNotFound(Target),
    /// Filesystem IO error
    #[error(transparent)]
    Io(#[from] SandIoError),
    /// General purpose message
    #[error("{0}")]
    Message(String),
    #[error("{0}")]
    CompilerError(String),
    #[error(transparent)]
    SemverError(#[from] semver::Error),
    /// Deserialization error
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
}

impl SandError {
    pub(crate) fn io(err: io::Error, path: impl Into<PathBuf>) -> Self {
        SandIoError::new(err, path).into()
    }
    pub fn msg(msg: impl Into<String>) -> Self {
        SandError::Message(msg.into())
    }
}

#[derive(Debug, Error)]
#[error("\"{}\": {io}", self.path.display())]
pub struct SandIoError {
    io: io::Error,
    path: PathBuf,
}

impl SandIoError {
    pub fn new(io: io::Error, path: impl Into<PathBuf>) -> Self {
        Self { io, path: path.into() }
    }
}
