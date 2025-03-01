use crate::SourceFile;
use foundry_compilers_core::utils::strip_prefix_owned;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// (source_file path  -> `SourceFile` + solc version)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VersionedSourceFiles(pub BTreeMap<PathBuf, Vec<VersionedSourceFile>>);

impl VersionedSourceFiles {
    /// Converts all `\\` separators in _all_ paths to `/`
    pub fn slash_paths(&mut self) {
        #[cfg(windows)]
        {
            use path_slash::PathExt;
            self.0 = std::mem::take(&mut self.0)
                .into_iter()
                .map(|(path, files)| (PathBuf::from(path.to_slash_lossy().as_ref()), files))
                .collect()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns an iterator over all files
    pub fn files(&self) -> impl Iterator<Item = &PathBuf> {
        self.0.keys()
    }

    /// Returns an iterator over the source files' IDs and path.
    pub fn into_ids(self) -> impl Iterator<Item = (u32, PathBuf)> {
        self.into_sources().map(|(path, source)| (source.id, path))
    }

    /// Returns an iterator over the source files' paths and IDs.
    pub fn into_paths(self) -> impl Iterator<Item = (PathBuf, u32)> {
        self.into_ids().map(|(id, path)| (path, id))
    }

    /// Returns an iterator over the source files' IDs and path.
    pub fn into_ids_with_version(self) -> impl Iterator<Item = (u32, PathBuf, Version)> {
        self.into_sources_with_version().map(|(path, source, version)| (source.id, path, version))
    }

    /// Finds the _first_ source file with the given path.
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::{artifacts::*, Project};
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let output = project.compile()?.into_output();
    /// let source_file = output.sources.find_file("src/Greeter.sol".as_ref()).unwrap();
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn find_file(&self, path: &Path) -> Option<&SourceFile> {
        self.sources().find(|&(p, _)| p == path).map(|(_, sf)| sf)
    }

    /// Same as [Self::find_file] but also checks for version
    pub fn find_file_and_version(&self, path: &Path, version: &Version) -> Option<&SourceFile> {
        self.0.get(path).and_then(|contracts| {
            contracts.iter().find_map(|source| {
                if source.version == *version {
                    Some(&source.source_file)
                } else {
                    None
                }
            })
        })
    }

    /// Finds the _first_ source file with the given id
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::{artifacts::*, Project};
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let output = project.compile()?.into_output();
    /// let source_file = output.sources.find_id(0).unwrap();
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn find_id(&self, id: u32) -> Option<&SourceFile> {
        self.sources().filter(|(_, source)| source.id == id).map(|(_, source)| source).next()
    }

    /// Same as [Self::find_id] but also checks for version
    pub fn find_id_and_version(&self, id: u32, version: &Version) -> Option<&SourceFile> {
        self.sources_with_version()
            .filter(|(_, source, v)| source.id == id && *v == version)
            .map(|(_, source, _)| source)
            .next()
    }

    /// Removes the _first_ source_file with the given path from the set
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::{artifacts::*, Project};
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let (mut sources, _) = project.compile()?.into_output().split();
    /// let source_file = sources.remove_by_path("src/Greeter.sol".as_ref()).unwrap();
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn remove_by_path(&mut self, path: &Path) -> Option<SourceFile> {
        self.0.get_mut(path).and_then(|all_sources| {
            if !all_sources.is_empty() {
                Some(all_sources.remove(0).source_file)
            } else {
                None
            }
        })
    }

    /// Removes the _first_ source_file with the given id from the set
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::{artifacts::*, Project};
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let (mut sources, _) = project.compile()?.into_output().split();
    /// let source_file = sources.remove_by_id(0).unwrap();
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn remove_by_id(&mut self, id: u32) -> Option<SourceFile> {
        self.0
            .values_mut()
            .filter_map(|sources| {
                sources
                    .iter()
                    .position(|source| source.source_file.id == id)
                    .map(|pos| sources.remove(pos).source_file)
            })
            .next()
    }

    /// Returns an iterator over all contracts and their names.
    pub fn sources(&self) -> impl Iterator<Item = (&PathBuf, &SourceFile)> {
        self.0.iter().flat_map(|(path, sources)| {
            sources.iter().map(move |source| (path, &source.source_file))
        })
    }

    /// Returns an iterator over (`file`,  `SourceFile`, `Version`)
    pub fn sources_with_version(&self) -> impl Iterator<Item = (&PathBuf, &SourceFile, &Version)> {
        self.0.iter().flat_map(|(file, sources)| {
            sources.iter().map(move |c| (file, &c.source_file, &c.version))
        })
    }

    /// Returns an iterator over all contracts and their source names.
    pub fn into_sources(self) -> impl Iterator<Item = (PathBuf, SourceFile)> {
        self.0.into_iter().flat_map(|(path, sources)| {
            sources.into_iter().map(move |source| (path.clone(), source.source_file))
        })
    }

    /// Returns an iterator over all contracts and their source names.
    pub fn into_sources_with_version(self) -> impl Iterator<Item = (PathBuf, SourceFile, Version)> {
        self.0.into_iter().flat_map(|(path, sources)| {
            sources
                .into_iter()
                .map(move |source| (path.clone(), source.source_file, source.version))
        })
    }

    /// Sets the sources' file paths to `root` adjoined to `self.file`.
    pub fn join_all(&mut self, root: &Path) -> &mut Self {
        self.0 = std::mem::take(&mut self.0)
            .into_iter()
            .map(|(file_path, sources)| (root.join(file_path), sources))
            .collect();
        self
    }

    /// Removes `base` from all source file paths
    pub fn strip_prefix_all(&mut self, base: &Path) -> &mut Self {
        self.0 = std::mem::take(&mut self.0)
            .into_iter()
            .map(|(file, sources)| (strip_prefix_owned(file, base), sources))
            .collect();
        self
    }
}

impl AsRef<BTreeMap<PathBuf, Vec<VersionedSourceFile>>> for VersionedSourceFiles {
    fn as_ref(&self) -> &BTreeMap<PathBuf, Vec<VersionedSourceFile>> {
        &self.0
    }
}

impl AsMut<BTreeMap<PathBuf, Vec<VersionedSourceFile>>> for VersionedSourceFiles {
    fn as_mut(&mut self) -> &mut BTreeMap<PathBuf, Vec<VersionedSourceFile>> {
        &mut self.0
    }
}

impl IntoIterator for VersionedSourceFiles {
    type Item = (PathBuf, Vec<VersionedSourceFile>);
    type IntoIter = std::collections::btree_map::IntoIter<PathBuf, Vec<VersionedSourceFile>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// A [SourceFile] and the compiler version used to compile it
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionedSourceFile {
    pub source_file: SourceFile,
    pub version: Version,
    pub build_id: String,
    pub profile: String,
}
