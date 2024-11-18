use forge_fmt::FormatterError;
use foundry_common::errors::FsPathError;

/// Possible errors when scanning a solidity file
#[derive(Debug, thiserror::Error)]
pub enum ScanFileError {
    #[error(transparent)]
    Io(#[from] FsPathError),
    #[error(transparent)]
    ParseSol(#[from] FormatterError),
}
