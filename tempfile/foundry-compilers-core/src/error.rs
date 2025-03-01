use semver::Version;
use std::{
    io,
    path::{Path, PathBuf},
};
use thiserror::Error;

pub type Result<T, E = SolcError> = std::result::Result<T, E>;

#[allow(unused_macros)]
#[macro_export]
macro_rules! format_err {
    ($($tt:tt)*) => {
        $crate::error::SolcError::msg(format!($($tt)*))
    };
}

#[allow(unused_macros)]
#[macro_export]
macro_rules! bail {
    ($($tt:tt)*) => { return Err(format_err!($($tt)*)) };
}

/// Various error types
#[derive(Debug, Error)]
pub enum SolcError {
    /// Errors related to the Solc executable itself.
    #[error("solc exited with {0}\n{1}")]
    SolcError(std::process::ExitStatus, String),
    #[error("failed to parse a file: {0}")]
    ParseError(String),
    #[error("invalid UTF-8 in Solc output")]
    InvalidUtf8,
    #[error("missing pragma from Solidity file")]
    PragmaNotFound,
    #[error("could not find Solc version locally or upstream")]
    VersionNotFound,
    #[error("checksum mismatch for {file}: expected {expected} found {detected} for {version}")]
    ChecksumMismatch { version: Version, expected: String, detected: String, file: PathBuf },
    #[error("checksum not found for {version}")]
    ChecksumNotFound { version: Version },
    #[error(transparent)]
    SemverError(#[from] semver::Error),
    /// Deserialization error
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
    /// Filesystem IO error
    #[error(transparent)]
    Io(#[from] SolcIoError),
    #[error("file could not be resolved due to broken symlink: {0}")]
    ResolveBadSymlink(SolcIoError),
    /// Failed to resolve a file
    #[error("failed to resolve file: {0}; check configured remappings")]
    Resolve(SolcIoError),
    #[error("file cannot be resolved due to mismatch of file name case: {error}.\nFound existing file: {existing_file:?}\nPlease check the case of the import.")]
    ResolveCaseSensitiveFileName { error: SolcIoError, existing_file: PathBuf },
    #[error(
        "{0}\n\t\
         --> {1}\n\t\
         {2}"
    )]
    FailedResolveImport(Box<SolcError>, PathBuf, PathBuf),
    #[cfg(feature = "svm-solc")]
    #[error(transparent)]
    SvmError(#[from] svm::SvmError),
    #[error("no contracts found at \"{0}\"")]
    NoContracts(String),
    /// General purpose message.
    #[error("{0}")]
    Message(String),

    #[error("no artifact found for `{}:{}`", .0.display(), .1)]
    ArtifactNotFound(PathBuf, String),

    #[cfg(feature = "project-util")]
    #[error(transparent)]
    FsExtra(#[from] fs_extra::error::Error),
}

impl SolcError {
    pub fn io(err: io::Error, path: impl Into<PathBuf>) -> Self {
        SolcIoError::new(err, path).into()
    }

    /// Create an error from the Solc executable's output.
    pub fn solc_output(output: &std::process::Output) -> Self {
        let mut msg = String::from_utf8_lossy(&output.stderr);
        let mut trimmed = msg.trim();
        if trimmed.is_empty() {
            msg = String::from_utf8_lossy(&output.stdout);
            trimmed = msg.trim();
            if trimmed.is_empty() {
                trimmed = "<empty output>";
            }
        }
        Self::SolcError(output.status, trimmed.into())
    }

    /// General purpose message.
    pub fn msg(msg: impl std::fmt::Display) -> Self {
        Self::Message(msg.to_string())
    }
}

#[derive(Debug, Error)]
#[error("\"{}\": {io}", self.path.display())]
pub struct SolcIoError {
    io: io::Error,
    path: PathBuf,
}

impl SolcIoError {
    pub fn new(io: io::Error, path: impl Into<PathBuf>) -> Self {
        Self { io, path: path.into() }
    }

    /// The path at which the error occurred
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The underlying `io::Error`
    pub fn source(&self) -> &io::Error {
        &self.io
    }
}

impl From<SolcIoError> for io::Error {
    fn from(err: SolcIoError) -> Self {
        err.io
    }
}
