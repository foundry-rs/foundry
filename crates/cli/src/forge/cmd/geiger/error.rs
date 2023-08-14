use foundry_common::errors::FsPathError;
use solang_parser::diagnostics::Diagnostic;
use std::path::PathBuf;

/// Possible errors when scanning a solidity file
#[derive(Debug, thiserror::Error)]
pub enum ScanFileError {
    #[error(transparent)]
    Io(#[from] FsPathError),
    #[error("Failed to parse {1:?}: {0:?}")]
    ParseSol(Vec<Diagnostic>, PathBuf),
}
