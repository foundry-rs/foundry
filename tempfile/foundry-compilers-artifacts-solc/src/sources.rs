use foundry_compilers_core::error::SolcIoError;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(feature = "walkdir")]
use foundry_compilers_core::utils;

type SourcesInner = BTreeMap<PathBuf, Source>;

/// An ordered list of files and their source.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sources(pub SourcesInner);

impl Sources {
    /// Returns a new instance of [Sources].
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if no sources should have optimized output selection.
    pub fn all_dirty(&self) -> bool {
        self.0.values().all(|s| s.is_dirty())
    }

    /// Returns all entries that should not be optimized.
    pub fn dirty(&self) -> impl Iterator<Item = (&PathBuf, &Source)> + '_ {
        self.0.iter().filter(|(_, s)| s.is_dirty())
    }

    /// Returns all entries that should be optimized.
    pub fn clean(&self) -> impl Iterator<Item = (&PathBuf, &Source)> + '_ {
        self.0.iter().filter(|(_, s)| !s.is_dirty())
    }

    /// Returns all files that should not be optimized.
    pub fn dirty_files(&self) -> impl Iterator<Item = &PathBuf> + '_ {
        self.dirty().map(|(k, _)| k)
    }
}

impl std::ops::Deref for Sources {
    type Target = SourcesInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Sources {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<I> From<I> for Sources
where
    SourcesInner: From<I>,
{
    fn from(value: I) -> Self {
        Self(From::from(value))
    }
}

impl<I> FromIterator<I> for Sources
where
    SourcesInner: FromIterator<I>,
{
    fn from_iter<T: IntoIterator<Item = I>>(iter: T) -> Self {
        Self(FromIterator::from_iter(iter))
    }
}

impl IntoIterator for Sources {
    type Item = <SourcesInner as IntoIterator>::Item;
    type IntoIter = <SourcesInner as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Sources {
    type Item = <&'a SourcesInner as IntoIterator>::Item;
    type IntoIter = <&'a SourcesInner as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'a> IntoIterator for &'a mut Sources {
    type Item = <&'a mut SourcesInner as IntoIterator>::Item;
    type IntoIter = <&'a mut SourcesInner as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}

/// Content of a solidity file
///
/// This contains the actual source code of a file
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// Content of the file
    ///
    /// This is an `Arc` because it may be cloned. If the graph of the project contains multiple
    /// conflicting versions then the same [Source] may be required by conflicting versions and
    /// needs to be duplicated.
    pub content: Arc<String>,
    #[serde(skip, default)]
    pub kind: SourceCompilationKind,
}

impl Source {
    /// Creates a new instance of [Source] with the given content.
    pub fn new(content: impl Into<String>) -> Self {
        Self { content: Arc::new(content.into()), kind: SourceCompilationKind::Complete }
    }

    /// Reads the file's content
    #[instrument(name = "read_source", level = "debug", skip_all, err)]
    pub fn read(file: &Path) -> Result<Self, SolcIoError> {
        trace!(file=%file.display());
        let mut content = fs::read_to_string(file).map_err(|err| SolcIoError::new(err, file))?;

        // Normalize line endings to ensure deterministic metadata.
        if content.contains('\r') {
            content = content.replace("\r\n", "\n");
        }

        Ok(Self::new(content))
    }

    /// Returns `true` if the source should be compiled with full output selection.
    pub fn is_dirty(&self) -> bool {
        self.kind.is_dirty()
    }

    /// Recursively finds all source files under the given dir path and reads them all
    #[cfg(feature = "walkdir")]
    pub fn read_all_from(dir: &Path, extensions: &[&str]) -> Result<Sources, SolcIoError> {
        Self::read_all_files(utils::source_files(dir, extensions))
    }

    /// Recursively finds all solidity and yul files under the given dir path and reads them all
    #[cfg(feature = "walkdir")]
    pub fn read_sol_yul_from(dir: &Path) -> Result<Sources, SolcIoError> {
        Self::read_all_from(dir, utils::SOLC_EXTENSIONS)
    }

    /// Reads all source files of the given vec
    ///
    /// Depending on the len of the vec it will try to read the files in parallel
    pub fn read_all_files(files: Vec<PathBuf>) -> Result<Sources, SolcIoError> {
        Self::read_all(files)
    }

    /// Reads all files
    pub fn read_all<T, I>(files: I) -> Result<Sources, SolcIoError>
    where
        I: IntoIterator<Item = T>,
        T: Into<PathBuf>,
    {
        files
            .into_iter()
            .map(Into::into)
            .map(|file| Self::read(&file).map(|source| (file, source)))
            .collect()
    }

    /// Parallelized version of `Self::read_all` that reads all files using a parallel iterator
    ///
    /// NOTE: this is only expected to be faster than `Self::read_all` if the given iterator
    /// contains at least several paths or the files are rather large.
    #[cfg(feature = "rayon")]
    pub fn par_read_all<T, I>(files: I) -> Result<Sources, SolcIoError>
    where
        I: IntoIterator<Item = T>,
        <I as IntoIterator>::IntoIter: Send,
        T: Into<PathBuf> + Send,
    {
        use rayon::{iter::ParallelBridge, prelude::ParallelIterator};
        files
            .into_iter()
            .par_bridge()
            .map(Into::into)
            .map(|file| Self::read(&file).map(|source| (file, source)))
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map(Sources)
    }

    /// Generate a non-cryptographically secure checksum of the file's content.
    #[cfg(feature = "checksum")]
    pub fn content_hash(&self) -> String {
        alloy_primitives::hex::encode(<md5::Md5 as md5::Digest>::digest(self.content.as_bytes()))
    }
}

#[cfg(feature = "async")]
impl Source {
    /// async version of `Self::read`
    #[instrument(name = "async_read_source", level = "debug", skip_all, err)]
    pub async fn async_read(file: &Path) -> Result<Self, SolcIoError> {
        let mut content =
            tokio::fs::read_to_string(file).await.map_err(|err| SolcIoError::new(err, file))?;

        // Normalize line endings to ensure deterministic metadata.
        if content.contains('\r') {
            content = content.replace("\r\n", "\n");
        }

        Ok(Self::new(content))
    }

    /// Finds all source files under the given dir path and reads them all
    #[cfg(feature = "walkdir")]
    pub async fn async_read_all_from(
        dir: &Path,
        extensions: &[&str],
    ) -> Result<Sources, SolcIoError> {
        Self::async_read_all(utils::source_files(dir, extensions)).await
    }

    /// async version of `Self::read_all`
    pub async fn async_read_all<T, I>(files: I) -> Result<Sources, SolcIoError>
    where
        I: IntoIterator<Item = T>,
        T: Into<PathBuf>,
    {
        futures_util::future::join_all(
            files
                .into_iter()
                .map(Into::into)
                .map(|file| async { Self::async_read(&file).await.map(|source| (file, source)) }),
        )
        .await
        .into_iter()
        .collect()
    }
}

impl AsRef<str> for Source {
    fn as_ref(&self) -> &str {
        &self.content
    }
}

impl AsRef<[u8]> for Source {
    fn as_ref(&self) -> &[u8] {
        self.content.as_bytes()
    }
}

/// Represents the state of a filtered [`Source`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SourceCompilationKind {
    /// We need a complete compilation output for the source.
    #[default]
    Complete,
    /// A source for which we don't need a complete output and want to optimize its compilation by
    /// reducing output selection.
    Optimized,
}

impl SourceCompilationKind {
    /// Whether this file should be compiled with full output selection
    pub fn is_dirty(&self) -> bool {
        matches!(self, Self::Complete)
    }
}
