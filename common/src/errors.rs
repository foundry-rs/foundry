//! Commonly used errors

use std::{
    io,
    path::{Path, PathBuf},
};

/// Various error variants for `std::fs` operations that serve as an addition to the io::Error which
/// does not provide any information about the path.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum FsPathError {
    /// Provides additional path context for `std::fs::write`.
    #[error("failed to write to {path:?}: {source}")]
    Write { source: io::Error, path: PathBuf },
    /// Provides additional path context for `std::fs::read`.
    #[error("failed to read from {path:?}: {source}")]
    Read { source: io::Error, path: PathBuf },
    /// Provides additional path context for `std::fs::File::create`.
    #[error("failed to create file {path:?}: {source}")]
    CreateFile { source: io::Error, path: PathBuf },
    /// Provides additional path context for `std::fs::remove_file`.
    #[error("failed to remove file {path:?}: {source}")]
    RemoveFile { source: io::Error, path: PathBuf },
    /// Provides additional path context for `std::fs::create_dir`.
    #[error("failed to create dir {path:?}: {source}")]
    CreateDir { source: io::Error, path: PathBuf },
    /// Provides additional path context for `std::fs::write_dir`.
    #[error("failed to remove dir {path:?}: {source}")]
    RemoveDir { source: io::Error, path: PathBuf },
    /// Provides additional path context for `std::fs::open`.
    #[error("failed to open file {path:?}: {source}")]
    Open { source: io::Error, path: PathBuf },
}

impl FsPathError {
    /// Returns the complementary error variant for `std::fs::write`.
    pub fn write(source: io::Error, path: impl Into<PathBuf>) -> Self {
        FsPathError::Write { source, path: path.into() }
    }

    /// Returns the complementary error variant for `std::fs::read`.
    pub fn read(source: io::Error, path: impl Into<PathBuf>) -> Self {
        FsPathError::Read { source, path: path.into() }
    }

    /// Returns the complementary error variant for `std::fs::File::create`.
    pub fn create_file(source: io::Error, path: impl Into<PathBuf>) -> Self {
        FsPathError::CreateFile { source, path: path.into() }
    }

    /// Returns the complementary error variant for `std::fs::remove_file`.
    pub fn remove_file(source: io::Error, path: impl Into<PathBuf>) -> Self {
        FsPathError::RemoveFile { source, path: path.into() }
    }

    /// Returns the complementary error variant for `std::fs::create_dir`.
    pub fn create_dir(source: io::Error, path: impl Into<PathBuf>) -> Self {
        FsPathError::CreateDir { source, path: path.into() }
    }

    /// Returns the complementary error variant for `std::fs::remove_dir`.
    pub fn remove_dir(source: io::Error, path: impl Into<PathBuf>) -> Self {
        FsPathError::RemoveDir { source, path: path.into() }
    }

    /// Returns the complementary error variant for `std::fs::File::open`.
    pub fn open(source: io::Error, path: impl Into<PathBuf>) -> Self {
        FsPathError::Open { source, path: path.into() }
    }
}

impl AsRef<Path> for FsPathError {
    fn as_ref(&self) -> &Path {
        match self {
            FsPathError::Write { path, .. } => path,
            FsPathError::Read { path, .. } => path,
            FsPathError::CreateDir { path, .. } => path,
            FsPathError::RemoveDir { path, .. } => path,
            FsPathError::CreateFile { path, .. } => path,
            FsPathError::RemoveFile { path, .. } => path,
            FsPathError::Open { path, .. } => path,
        }
    }
}

impl From<FsPathError> for io::Error {
    fn from(err: FsPathError) -> Self {
        match err {
            FsPathError::Write { source, .. } => source,
            FsPathError::Read { source, .. } => source,
            FsPathError::CreateDir { source, .. } => source,
            FsPathError::RemoveDir { source, .. } => source,
            FsPathError::CreateFile { source, .. } => source,
            FsPathError::RemoveFile { source, .. } => source,
            FsPathError::Open { source, .. } => source,
        }
    }
}
