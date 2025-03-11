use std::{
    io,
    path::{Path, PathBuf},
};

#[allow(unused_imports)]
use std::fs::{self, File};

/// Various error variants for `fs` operations that serve as an addition to the io::Error which
/// does not provide any information about the path.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum FsPathError {
    /// Provides additional path context for [`fs::write`].
    #[error("failed to write to {path:?}: {source}")]
    Write { source: io::Error, path: PathBuf },
    /// Provides additional path context for [`fs::read`].
    #[error("failed to read from {path:?}: {source}")]
    Read { source: io::Error, path: PathBuf },
    /// Provides additional path context for [`fs::copy`].
    #[error("failed to copy from {from:?} to {to:?}: {source}")]
    Copy { source: io::Error, from: PathBuf, to: PathBuf },
    /// Provides additional path context for [`fs::read_link`].
    #[error("failed to read from {path:?}: {source}")]
    ReadLink { source: io::Error, path: PathBuf },
    /// Provides additional path context for [`File::create`].
    #[error("failed to create file {path:?}: {source}")]
    CreateFile { source: io::Error, path: PathBuf },
    /// Provides additional path context for [`fs::remove_file`].
    #[error("failed to remove file {path:?}: {source}")]
    RemoveFile { source: io::Error, path: PathBuf },
    /// Provides additional path context for [`fs::create_dir`].
    #[error("failed to create dir {path:?}: {source}")]
    CreateDir { source: io::Error, path: PathBuf },
    /// Provides additional path context for [`fs::remove_dir`].
    #[error("failed to remove dir {path:?}: {source}")]
    RemoveDir { source: io::Error, path: PathBuf },
    /// Provides additional path context for [`File::open`].
    #[error("failed to open file {path:?}: {source}")]
    Open { source: io::Error, path: PathBuf },
    /// Provides additional path context for the file whose contents should be parsed as JSON.
    #[error("failed to parse json file: {path:?}: {source}")]
    ReadJson { source: serde_json::Error, path: PathBuf },
    /// Provides additional path context for the new JSON file.
    #[error("failed to write to json file: {path:?}: {source}")]
    WriteJson { source: serde_json::Error, path: PathBuf },
}

impl FsPathError {
    /// Returns the complementary error variant for [`fs::write`].
    pub fn write(source: io::Error, path: impl Into<PathBuf>) -> Self {
        Self::Write { source, path: path.into() }
    }

    /// Returns the complementary error variant for [`fs::read`].
    pub fn read(source: io::Error, path: impl Into<PathBuf>) -> Self {
        Self::Read { source, path: path.into() }
    }

    /// Returns the complementary error variant for [`fs::copy`].
    pub fn copy(source: io::Error, from: impl Into<PathBuf>, to: impl Into<PathBuf>) -> Self {
        Self::Copy { source, from: from.into(), to: to.into() }
    }

    /// Returns the complementary error variant for [`fs::read_link`].
    pub fn read_link(source: io::Error, path: impl Into<PathBuf>) -> Self {
        Self::ReadLink { source, path: path.into() }
    }

    /// Returns the complementary error variant for [`File::create`].
    pub fn create_file(source: io::Error, path: impl Into<PathBuf>) -> Self {
        Self::CreateFile { source, path: path.into() }
    }

    /// Returns the complementary error variant for [`fs::remove_file`].
    pub fn remove_file(source: io::Error, path: impl Into<PathBuf>) -> Self {
        Self::RemoveFile { source, path: path.into() }
    }

    /// Returns the complementary error variant for [`fs::create_dir`].
    pub fn create_dir(source: io::Error, path: impl Into<PathBuf>) -> Self {
        Self::CreateDir { source, path: path.into() }
    }

    /// Returns the complementary error variant for [`fs::remove_dir`].
    pub fn remove_dir(source: io::Error, path: impl Into<PathBuf>) -> Self {
        Self::RemoveDir { source, path: path.into() }
    }

    /// Returns the complementary error variant for [`File::open`].
    pub fn open(source: io::Error, path: impl Into<PathBuf>) -> Self {
        Self::Open { source, path: path.into() }
    }
}

impl AsRef<Path> for FsPathError {
    fn as_ref(&self) -> &Path {
        match self {
            Self::Write { path, .. } |
            Self::Read { path, .. } |
            Self::ReadLink { path, .. } |
            Self::Copy { from: path, .. } |
            Self::CreateDir { path, .. } |
            Self::RemoveDir { path, .. } |
            Self::CreateFile { path, .. } |
            Self::RemoveFile { path, .. } |
            Self::Open { path, .. } |
            Self::ReadJson { path, .. } |
            Self::WriteJson { path, .. } => path,
        }
    }
}

impl From<FsPathError> for io::Error {
    fn from(value: FsPathError) -> Self {
        match value {
            FsPathError::Write { source, .. } |
            FsPathError::Read { source, .. } |
            FsPathError::ReadLink { source, .. } |
            FsPathError::Copy { source, .. } |
            FsPathError::CreateDir { source, .. } |
            FsPathError::RemoveDir { source, .. } |
            FsPathError::CreateFile { source, .. } |
            FsPathError::RemoveFile { source, .. } |
            FsPathError::Open { source, .. } => source,

            FsPathError::ReadJson { source, .. } | FsPathError::WriteJson { source, .. } => {
                source.into()
            }
        }
    }
}
