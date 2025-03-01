//! Output artifact handling

use alloy_json_abi::JsonAbi;
use alloy_primitives::Bytes;
use foundry_compilers_artifacts::{
    hh::HardhatArtifact,
    sourcemap::{SourceMap, SyntaxError},
    BytecodeObject, CompactBytecode, CompactContract, CompactContractBytecode,
    CompactContractBytecodeCow, CompactDeployedBytecode, Contract, FileToContractsMap, SourceFile,
};
use foundry_compilers_core::{
    error::{Result, SolcError, SolcIoError},
    utils::{self, strip_prefix_owned},
};
use path_slash::PathBufExt;
use semver::Version;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{btree_map::BTreeMap, HashMap, HashSet},
    ffi::OsString,
    fmt, fs,
    hash::Hash,
    ops::Deref,
    path::{Path, PathBuf},
};

mod configurable;
pub use configurable::*;

mod hh;
pub use hh::*;

use crate::{
    cache::{CachedArtifacts, CompilerCache},
    output::{
        contracts::VersionedContracts,
        sources::{VersionedSourceFile, VersionedSourceFiles},
    },
    CompilerContract, ProjectPathsConfig,
};

/// Represents unique artifact metadata for identifying artifacts on output
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ArtifactId {
    /// `artifact` cache path
    pub path: PathBuf,
    pub name: String,
    /// Original source file path
    pub source: PathBuf,
    /// `solc` version that produced this artifact
    pub version: Version,
    /// `solc` build id
    pub build_id: String,
    pub profile: String,
}

impl ArtifactId {
    /// Converts any `\\` separators in the `path` to `/`
    pub fn slash_paths(&mut self) {
        #[cfg(windows)]
        {
            self.path = self.path.to_slash_lossy().as_ref().into();
            self.source = self.source.to_slash_lossy().as_ref().into();
        }
    }

    /// Convenience function fo [`Self::slash_paths()`]
    pub fn with_slashed_paths(mut self) -> Self {
        self.slash_paths();
        self
    }

    /// Removes `base` from the source's path.
    pub fn strip_file_prefixes(&mut self, base: &Path) {
        if let Ok(stripped) = self.source.strip_prefix(base) {
            self.source = stripped.to_path_buf();
        }
    }

    /// Convenience function for [`Self::strip_file_prefixes()`]
    pub fn with_stripped_file_prefixes(mut self, base: &Path) -> Self {
        self.strip_file_prefixes(base);
        self
    }

    /// Returns a `<filename>:<name>` slug that identifies an artifact
    ///
    /// Note: This identifier is not necessarily unique. If two contracts have the same name, they
    /// will share the same slug. For a unique identifier see [ArtifactId::identifier].
    pub fn slug(&self) -> String {
        format!("{}.json:{}", self.path.file_stem().unwrap().to_string_lossy(), self.name)
    }

    /// Returns a `<source path>:<name>` slug that uniquely identifies an artifact
    pub fn identifier(&self) -> String {
        format!("{}:{}", self.source.display(), self.name)
    }

    /// Returns a `<filename><version>:<name>` slug that identifies an artifact
    pub fn slug_versioned(&self) -> String {
        format!(
            "{}.{}.{}.{}.json:{}",
            self.path.file_stem().unwrap().to_string_lossy(),
            self.version.major,
            self.version.minor,
            self.version.patch,
            self.name
        )
    }
}

/// Represents an artifact file representing a [`crate::compilers::CompilerContract`]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactFile<T> {
    /// The Artifact that was written
    pub artifact: T,
    /// path to the file where the `artifact` was written to
    pub file: PathBuf,
    /// `solc` version that produced this artifact
    pub version: Version,
    pub build_id: String,
    pub profile: String,
}

impl<T: Serialize> ArtifactFile<T> {
    /// Writes the given contract to the `out` path creating all parent directories
    pub fn write(&self) -> Result<()> {
        trace!("writing artifact file {:?} {}", self.file, self.version);
        utils::create_parent_dir_all(&self.file)?;
        utils::write_json_file(&self.artifact, &self.file, 64 * 1024)
    }
}

impl<T> ArtifactFile<T> {
    /// Sets the file to `root` adjoined to `self.file`.
    pub fn join(&mut self, root: &Path) {
        self.file = root.join(&self.file);
    }

    /// Removes `base` from the artifact's path
    pub fn strip_prefix(&mut self, base: &Path) {
        if let Ok(stripped) = self.file.strip_prefix(base) {
            self.file = stripped.to_path_buf();
        }
    }
}

/// local helper type alias `file name -> (contract name  -> Vec<..>)`
pub(crate) type ArtifactsMap<T> = FileToContractsMap<Vec<ArtifactFile<T>>>;

/// Represents a set of Artifacts
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifacts<T>(pub ArtifactsMap<T>);

impl<T> From<ArtifactsMap<T>> for Artifacts<T> {
    fn from(m: ArtifactsMap<T>) -> Self {
        Self(m)
    }
}

impl<'a, T> IntoIterator for &'a Artifacts<T> {
    type Item = (&'a PathBuf, &'a BTreeMap<String, Vec<ArtifactFile<T>>>);
    type IntoIter =
        std::collections::btree_map::Iter<'a, PathBuf, BTreeMap<String, Vec<ArtifactFile<T>>>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<T> IntoIterator for Artifacts<T> {
    type Item = (PathBuf, BTreeMap<String, Vec<ArtifactFile<T>>>);
    type IntoIter =
        std::collections::btree_map::IntoIter<PathBuf, BTreeMap<String, Vec<ArtifactFile<T>>>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<T> Default for Artifacts<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<T> AsRef<ArtifactsMap<T>> for Artifacts<T> {
    fn as_ref(&self) -> &ArtifactsMap<T> {
        &self.0
    }
}

impl<T> AsMut<ArtifactsMap<T>> for Artifacts<T> {
    fn as_mut(&mut self) -> &mut ArtifactsMap<T> {
        &mut self.0
    }
}

impl<T> Deref for Artifacts<T> {
    type Target = ArtifactsMap<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Serialize> Artifacts<T> {
    /// Writes all artifacts into the given `artifacts_root` folder
    pub fn write_all(&self) -> Result<()> {
        for artifact in self.artifact_files() {
            artifact.write()?;
        }
        Ok(())
    }
}

impl<T> Artifacts<T> {
    /// Converts all `\\` separators in _all_ paths to `/`
    pub fn slash_paths(&mut self) {
        #[cfg(windows)]
        {
            self.0 = std::mem::take(&mut self.0)
                .into_iter()
                .map(|(path, files)| (PathBuf::from(path.to_slash_lossy().as_ref()), files))
                .collect()
        }
    }

    pub fn into_inner(self) -> ArtifactsMap<T> {
        self.0
    }

    /// Sets the artifact files location to `root` adjoined to `self.file`.
    pub fn join_all(&mut self, root: &Path) -> &mut Self {
        self.artifact_files_mut().for_each(|artifact| artifact.join(root));
        self
    }

    /// Removes `base` from all artifacts
    pub fn strip_prefix_all(&mut self, base: &Path) -> &mut Self {
        self.artifact_files_mut().for_each(|artifact| artifact.strip_prefix(base));
        self
    }

    /// Returns all `ArtifactFile`s for the contract with the matching name
    fn get_contract_artifact_files(&self, contract_name: &str) -> Option<&Vec<ArtifactFile<T>>> {
        self.0.values().find_map(|all| all.get(contract_name))
    }

    /// Returns the `Artifact` with matching file, contract name and version
    pub fn find_artifact(
        &self,
        file: &Path,
        contract_name: &str,
        version: &Version,
    ) -> Option<&ArtifactFile<T>> {
        self.0
            .get(file)
            .and_then(|contracts| contracts.get(contract_name))
            .and_then(|artifacts| artifacts.iter().find(|artifact| artifact.version == *version))
    }

    /// Returns true if this type contains an artifact with the given path for the given contract
    pub fn has_contract_artifact(&self, contract_name: &str, artifact_path: &Path) -> bool {
        self.get_contract_artifact_files(contract_name)
            .map(|artifacts| artifacts.iter().any(|artifact| artifact.file == artifact_path))
            .unwrap_or_default()
    }

    /// Returns true if this type contains an artifact with the given path
    pub fn has_artifact(&self, artifact_path: &Path) -> bool {
        self.artifact_files().any(|artifact| artifact.file == artifact_path)
    }

    /// Iterate over all artifact files
    pub fn artifact_files(&self) -> impl Iterator<Item = &ArtifactFile<T>> {
        self.0.values().flat_map(BTreeMap::values).flatten()
    }

    /// Iterate over all artifact files
    pub fn artifact_files_mut(&mut self) -> impl Iterator<Item = &mut ArtifactFile<T>> {
        self.0.values_mut().flat_map(BTreeMap::values_mut).flatten()
    }

    /// Returns an iterator over _all_ artifacts and `<file name:contract name>`.
    ///
    /// Borrowed version of [`Self::into_artifacts`].
    pub fn artifacts<O: ArtifactOutput<Artifact = T>>(
        &self,
    ) -> impl Iterator<Item = (ArtifactId, &T)> + '_ {
        self.0.iter().flat_map(|(source, contract_artifacts)| {
            contract_artifacts.iter().flat_map(move |(_contract_name, artifacts)| {
                artifacts.iter().filter_map(move |artifact| {
                    O::contract_name(&artifact.file).map(|name| {
                        (
                            ArtifactId {
                                path: PathBuf::from(&artifact.file),
                                name,
                                source: source.clone(),
                                version: artifact.version.clone(),
                                build_id: artifact.build_id.clone(),
                                profile: artifact.profile.clone(),
                            }
                            .with_slashed_paths(),
                            &artifact.artifact,
                        )
                    })
                })
            })
        })
    }

    /// Returns an iterator over _all_ artifacts and `<file name:contract name>`
    pub fn into_artifacts<O: ArtifactOutput<Artifact = T>>(
        self,
    ) -> impl Iterator<Item = (ArtifactId, T)> {
        self.0.into_iter().flat_map(|(source, contract_artifacts)| {
            contract_artifacts.into_iter().flat_map(move |(_contract_name, artifacts)| {
                let source = source.clone();
                artifacts.into_iter().filter_map(move |artifact| {
                    O::contract_name(&artifact.file).map(|name| {
                        (
                            ArtifactId {
                                path: PathBuf::from(&artifact.file),
                                name,
                                source: source.clone(),
                                version: artifact.version,
                                build_id: artifact.build_id.clone(),
                                profile: artifact.profile.clone(),
                            }
                            .with_slashed_paths(),
                            artifact.artifact,
                        )
                    })
                })
            })
        })
    }

    /// Returns an iterator that yields the tuple `(file, contract name, artifact)`
    ///
    /// **NOTE** this returns the path as is
    ///
    /// Borrowed version of [`Self::into_artifacts_with_files`].
    pub fn artifacts_with_files(&self) -> impl Iterator<Item = (&PathBuf, &String, &T)> + '_ {
        self.0.iter().flat_map(|(f, contract_artifacts)| {
            contract_artifacts.iter().flat_map(move |(name, artifacts)| {
                artifacts.iter().map(move |artifact| (f, name, &artifact.artifact))
            })
        })
    }

    /// Returns an iterator that yields the tuple `(file, contract name, artifact)`
    ///
    /// **NOTE** this returns the path as is
    pub fn into_artifacts_with_files(self) -> impl Iterator<Item = (PathBuf, String, T)> {
        self.0.into_iter().flat_map(|(f, contract_artifacts)| {
            contract_artifacts.into_iter().flat_map(move |(name, artifacts)| {
                let contract_name = name;
                let file = f.clone();
                artifacts
                    .into_iter()
                    .map(move |artifact| (file.clone(), contract_name.clone(), artifact.artifact))
            })
        })
    }

    /// Strips the given prefix from all artifact file paths to make them relative to the given
    /// `root` argument
    pub fn into_stripped_file_prefixes(self, base: &Path) -> Self {
        let artifacts =
            self.0.into_iter().map(|(path, c)| (strip_prefix_owned(path, base), c)).collect();
        Self(artifacts)
    }

    /// Finds the first artifact `T` with a matching contract name
    pub fn find_first(&self, contract_name: &str) -> Option<&T> {
        self.0.iter().find_map(|(_file, contracts)| {
            contracts.get(contract_name).and_then(|c| c.first().map(|a| &a.artifact))
        })
    }

    ///  Finds the artifact with a matching path and name
    pub fn find(&self, contract_path: &Path, contract_name: &str) -> Option<&T> {
        self.0.iter().filter(|(path, _)| path.as_path() == contract_path).find_map(
            |(_file, contracts)| {
                contracts.get(contract_name).and_then(|c| c.first().map(|a| &a.artifact))
            },
        )
    }

    /// Removes the artifact with matching file and name
    pub fn remove(&mut self, contract_path: &Path, contract_name: &str) -> Option<T> {
        self.0.iter_mut().filter(|(path, _)| path.as_path() == contract_path).find_map(
            |(_file, contracts)| {
                let mut artifact = None;
                if let Some((c, mut artifacts)) = contracts.remove_entry(contract_name) {
                    if !artifacts.is_empty() {
                        artifact = Some(artifacts.remove(0).artifact);
                    }
                    if !artifacts.is_empty() {
                        contracts.insert(c, artifacts);
                    }
                }
                artifact
            },
        )
    }

    /// Removes the first artifact `T` with a matching contract name
    ///
    /// *Note:* if there are multiple artifacts (contract compiled with different solc) then this
    /// returns the first artifact in that set
    pub fn remove_first(&mut self, contract_name: &str) -> Option<T> {
        self.0.iter_mut().find_map(|(_file, contracts)| {
            let mut artifact = None;
            if let Some((c, mut artifacts)) = contracts.remove_entry(contract_name) {
                if !artifacts.is_empty() {
                    artifact = Some(artifacts.remove(0).artifact);
                }
                if !artifacts.is_empty() {
                    contracts.insert(c, artifacts);
                }
            }
            artifact
        })
    }
}

/// A trait representation for a [`crate::compilers::CompilerContract`] artifact
pub trait Artifact {
    /// Returns the artifact's [`JsonAbi`] and bytecode.
    fn into_inner(self) -> (Option<JsonAbi>, Option<Bytes>);

    /// Turns the artifact into a container type for abi, compact bytecode and deployed bytecode
    fn into_compact_contract(self) -> CompactContract;

    /// Turns the artifact into a container type for abi, full bytecode and deployed bytecode
    fn into_contract_bytecode(self) -> CompactContractBytecode;

    /// Returns the contents of this type as a single tuple of abi, bytecode and deployed bytecode
    fn into_parts(self) -> (Option<JsonAbi>, Option<Bytes>, Option<Bytes>);

    /// Consumes the type and returns the [JsonAbi]
    fn into_abi(self) -> Option<JsonAbi>
    where
        Self: Sized,
    {
        self.into_parts().0
    }

    /// Consumes the type and returns the `bytecode`
    fn into_bytecode_bytes(self) -> Option<Bytes>
    where
        Self: Sized,
    {
        self.into_parts().1
    }
    /// Consumes the type and returns the `deployed bytecode`
    fn into_deployed_bytecode_bytes(self) -> Option<Bytes>
    where
        Self: Sized,
    {
        self.into_parts().2
    }

    /// Same as [`Self::into_parts()`] but returns `Err` if an element is `None`
    fn try_into_parts(self) -> Result<(JsonAbi, Bytes, Bytes)>
    where
        Self: Sized,
    {
        let (abi, bytecode, deployed_bytecode) = self.into_parts();

        Ok((
            abi.ok_or_else(|| SolcError::msg("abi missing"))?,
            bytecode.ok_or_else(|| SolcError::msg("bytecode missing"))?,
            deployed_bytecode.ok_or_else(|| SolcError::msg("deployed bytecode missing"))?,
        ))
    }

    /// Returns the reference of container type for abi, compact bytecode and deployed bytecode if
    /// available
    fn get_contract_bytecode(&self) -> CompactContractBytecodeCow<'_>;

    /// Returns the reference to the `bytecode`
    fn get_bytecode(&self) -> Option<Cow<'_, CompactBytecode>> {
        self.get_contract_bytecode().bytecode
    }

    /// Returns the reference to the `bytecode` object
    fn get_bytecode_object(&self) -> Option<Cow<'_, BytecodeObject>> {
        let val = match self.get_bytecode()? {
            Cow::Borrowed(b) => Cow::Borrowed(&b.object),
            Cow::Owned(b) => Cow::Owned(b.object),
        };
        Some(val)
    }

    /// Returns the bytes of the `bytecode` object
    fn get_bytecode_bytes(&self) -> Option<Cow<'_, Bytes>> {
        let val = match self.get_bytecode_object()? {
            Cow::Borrowed(b) => Cow::Borrowed(b.as_bytes()?),
            Cow::Owned(b) => Cow::Owned(b.into_bytes()?),
        };
        Some(val)
    }

    /// Returns the reference to the `deployedBytecode`
    fn get_deployed_bytecode(&self) -> Option<Cow<'_, CompactDeployedBytecode>> {
        self.get_contract_bytecode().deployed_bytecode
    }

    /// Returns the reference to the `bytecode` object
    fn get_deployed_bytecode_object(&self) -> Option<Cow<'_, BytecodeObject>> {
        let val = match self.get_deployed_bytecode()? {
            Cow::Borrowed(b) => Cow::Borrowed(&b.bytecode.as_ref()?.object),
            Cow::Owned(b) => Cow::Owned(b.bytecode?.object),
        };
        Some(val)
    }

    /// Returns the bytes of the `deployed bytecode` object
    fn get_deployed_bytecode_bytes(&self) -> Option<Cow<'_, Bytes>> {
        let val = match self.get_deployed_bytecode_object()? {
            Cow::Borrowed(b) => Cow::Borrowed(b.as_bytes()?),
            Cow::Owned(b) => Cow::Owned(b.into_bytes()?),
        };
        Some(val)
    }

    /// Returns the reference to the [JsonAbi] if available
    fn get_abi(&self) -> Option<Cow<'_, JsonAbi>> {
        self.get_contract_bytecode().abi
    }

    /// Returns the `sourceMap` of the creation bytecode
    ///
    /// Returns `None` if no `sourceMap` string was included in the compiler output
    /// Returns `Some(Err)` if parsing the sourcemap failed
    fn get_source_map(&self) -> Option<std::result::Result<SourceMap, SyntaxError>> {
        self.get_bytecode()?.source_map()
    }

    /// Returns the creation bytecode `sourceMap` as str if it was included in the compiler output
    fn get_source_map_str(&self) -> Option<Cow<'_, str>> {
        match self.get_bytecode()? {
            Cow::Borrowed(code) => code.source_map.as_deref().map(Cow::Borrowed),
            Cow::Owned(code) => code.source_map.map(Cow::Owned),
        }
    }

    /// Returns the `sourceMap` of the runtime bytecode
    ///
    /// Returns `None` if no `sourceMap` string was included in the compiler output
    /// Returns `Some(Err)` if parsing the sourcemap failed
    fn get_source_map_deployed(&self) -> Option<std::result::Result<SourceMap, SyntaxError>> {
        self.get_deployed_bytecode()?.source_map()
    }

    /// Returns the runtime bytecode `sourceMap` as str if it was included in the compiler output
    fn get_source_map_deployed_str(&self) -> Option<Cow<'_, str>> {
        match self.get_bytecode()? {
            Cow::Borrowed(code) => code.source_map.as_deref().map(Cow::Borrowed),
            Cow::Owned(code) => code.source_map.map(Cow::Owned),
        }
    }
}

impl<T> Artifact for T
where
    T: Into<CompactContractBytecode> + Into<CompactContract>,
    for<'a> &'a T: Into<CompactContractBytecodeCow<'a>>,
{
    fn into_inner(self) -> (Option<JsonAbi>, Option<Bytes>) {
        let artifact = self.into_compact_contract();
        (artifact.abi, artifact.bin.and_then(|bin| bin.into_bytes()))
    }

    fn into_compact_contract(self) -> CompactContract {
        self.into()
    }

    fn into_contract_bytecode(self) -> CompactContractBytecode {
        self.into()
    }

    fn into_parts(self) -> (Option<JsonAbi>, Option<Bytes>, Option<Bytes>) {
        self.into_compact_contract().into_parts()
    }

    fn get_contract_bytecode(&self) -> CompactContractBytecodeCow<'_> {
        self.into()
    }
}

/// Handler invoked with the output of `solc`
///
/// Implementers of this trait are expected to take care of [`crate::compilers::CompilerContract`]
/// to [`crate::ArtifactOutput::Artifact`] conversion and how that `Artifact` type is stored on
/// disk, this includes artifact file location and naming.
///
/// Depending on the [`crate::Project`] contracts and their compatible versions,
/// The project compiler may invoke different `solc` executables on the same
/// solidity file leading to multiple [`crate::CompilerOutput`]s for the same `.sol` file.
/// In addition to the `solidity file` to `contract` relationship (1-N*)
/// [`crate::VersionedContracts`] also tracks the `contract` to (`artifact` + `solc version`)
/// relationship (1-N+).
pub trait ArtifactOutput {
    /// Represents the artifact that will be stored for a `Contract`
    type Artifact: Artifact + DeserializeOwned + Serialize + fmt::Debug + Send + Sync;
    type CompilerContract: CompilerContract;

    /// Handle the aggregated set of compiled contracts from the solc [`crate::CompilerOutput`].
    ///
    /// This will be invoked with all aggregated contracts from (multiple) solc `CompilerOutput`.
    /// See [`crate::AggregatedCompilerOutput`]
    fn on_output<L>(
        &self,
        contracts: &VersionedContracts<Self::CompilerContract>,
        sources: &VersionedSourceFiles,
        layout: &ProjectPathsConfig<L>,
        ctx: OutputContext<'_>,
        primary_profiles: &HashMap<PathBuf, &str>,
    ) -> Result<Artifacts<Self::Artifact>> {
        let mut artifacts =
            self.output_to_artifacts(contracts, sources, ctx, layout, primary_profiles);
        fs::create_dir_all(&layout.artifacts).map_err(|err| {
            error!(dir=?layout.artifacts, "Failed to create artifacts folder");
            SolcIoError::new(err, &layout.artifacts)
        })?;

        artifacts.join_all(&layout.artifacts);
        artifacts.write_all()?;

        self.handle_artifacts(contracts, &artifacts)?;

        Ok(artifacts)
    }

    /// Invoked after artifacts has been written to disk for additional processing.
    fn handle_artifacts(
        &self,
        _contracts: &VersionedContracts<Self::CompilerContract>,
        _artifacts: &Artifacts<Self::Artifact>,
    ) -> Result<()> {
        Ok(())
    }

    /// Returns the file name for the contract's artifact
    /// `Greeter.json`
    fn output_file_name(
        name: &str,
        version: &Version,
        profile: &str,
        with_version: bool,
        with_profile: bool,
    ) -> PathBuf {
        let mut name = name.to_string();
        if with_version {
            name.push_str(&format!(".{}.{}.{}", version.major, version.minor, version.patch));
        }
        if with_profile {
            name.push_str(&format!(".{profile}"));
        }
        name.push_str(".json");
        name.into()
    }

    /// Returns the appropriate file name for the conflicting file.
    ///
    /// This should ensure that the resulting `PathBuf` is conflict free, which could be possible if
    /// there are two separate contract files (in different folders) that contain the same contract:
    ///
    /// `src/A.sol::A`
    /// `src/nested/A.sol::A`
    ///
    /// Which would result in the same `PathBuf` if only the file and contract name is taken into
    /// account, [`Self::output_file`].
    ///
    /// This return a unique output file
    fn conflict_free_output_file(
        already_taken: &HashSet<String>,
        conflict: PathBuf,
        contract_file: &Path,
        artifacts_folder: &Path,
    ) -> PathBuf {
        let mut rel_candidate = conflict;
        if let Ok(stripped) = rel_candidate.strip_prefix(artifacts_folder) {
            rel_candidate = stripped.to_path_buf();
        }
        #[allow(clippy::redundant_clone)] // false positive
        let mut candidate = rel_candidate.clone();
        let mut current_parent = contract_file.parent();

        while let Some(parent_name) = current_parent.and_then(|f| f.file_name()) {
            // this is problematic if both files are absolute
            candidate = Path::new(parent_name).join(&candidate);
            let out_path = artifacts_folder.join(&candidate);
            if !already_taken.contains(&out_path.to_slash_lossy().to_lowercase()) {
                trace!("found alternative output file={:?} for {:?}", out_path, contract_file);
                return out_path;
            }
            current_parent = current_parent.and_then(|f| f.parent());
        }

        // this means we haven't found an alternative yet, which shouldn't actually happen since
        // `contract_file` are unique, but just to be safe, handle this case in which case
        // we simply numerate the parent folder

        trace!("no conflict free output file found after traversing the file");

        let mut num = 1;

        loop {
            // this will attempt to find an alternate path by numerating the first component in the
            // path: `<root>+_<num>/....sol`
            let mut components = rel_candidate.components();
            let first = components.next().expect("path not empty");
            let name = first.as_os_str();
            let mut numerated = OsString::with_capacity(name.len() + 2);
            numerated.push(name);
            numerated.push("_");
            numerated.push(num.to_string());

            let candidate: PathBuf = Some(numerated.as_os_str())
                .into_iter()
                .chain(components.map(|c| c.as_os_str()))
                .collect();
            if !already_taken.contains(&candidate.to_slash_lossy().to_lowercase()) {
                trace!("found alternative output file={:?} for {:?}", candidate, contract_file);
                return candidate;
            }

            num += 1;
        }
    }

    /// Returns the path to the contract's artifact location based on the contract's file and name
    ///
    /// This returns `contract.sol/contract.json` by default
    fn output_file(
        contract_file: &Path,
        name: &str,
        version: &Version,
        profile: &str,
        with_version: bool,
        with_profile: bool,
    ) -> PathBuf {
        contract_file
            .file_name()
            .map(Path::new)
            .map(|p| {
                p.join(Self::output_file_name(name, version, profile, with_version, with_profile))
            })
            .unwrap_or_else(|| {
                Self::output_file_name(name, version, profile, with_version, with_profile)
            })
    }

    /// The inverse of `contract_file_name`
    ///
    /// Expected to return the solidity contract's name derived from the file path
    /// `sources/Greeter.sol` -> `Greeter`
    fn contract_name(file: &Path) -> Option<String> {
        file.file_stem().and_then(|s| s.to_str().map(|s| s.to_string()))
    }

    /// Read the artifact that's stored at the given path
    ///
    /// # Errors
    ///
    /// Returns an error if
    ///     - The file does not exist
    ///     - The file's content couldn't be deserialized into the `Artifact` type
    fn read_cached_artifact(path: &Path) -> Result<Self::Artifact> {
        utils::read_json_file(path)
    }

    /// Read the cached artifacts that are located the paths the iterator yields
    ///
    /// See [`Self::read_cached_artifact()`]
    fn read_cached_artifacts<T, I>(files: I) -> Result<BTreeMap<PathBuf, Self::Artifact>>
    where
        I: IntoIterator<Item = T>,
        T: Into<PathBuf>,
    {
        let mut artifacts = BTreeMap::default();
        for path in files.into_iter() {
            let path = path.into();
            let artifact = Self::read_cached_artifact(&path)?;
            artifacts.insert(path, artifact);
        }
        Ok(artifacts)
    }

    /// Convert a contract to the artifact type
    ///
    /// This is the core conversion function that takes care of converting a `Contract` into the
    /// associated `Artifact` type.
    /// The `SourceFile` is also provided
    fn contract_to_artifact(
        &self,
        _file: &Path,
        _name: &str,
        contract: Self::CompilerContract,
        source_file: Option<&SourceFile>,
    ) -> Self::Artifact;

    /// Generates a path for an artifact based on already taken paths by either cached or compiled
    /// artifacts.
    #[allow(clippy::too_many_arguments)]
    fn get_artifact_path(
        ctx: &OutputContext<'_>,
        already_taken: &HashSet<String>,
        file: &Path,
        name: &str,
        artifacts_folder: &Path,
        version: &Version,
        profile: &str,
        with_version: bool,
        with_profile: bool,
    ) -> PathBuf {
        // if an artifact for the contract already exists (from a previous compile job)
        // we reuse the path, this will make sure that even if there are conflicting
        // files (files for witch `T::output_file()` would return the same path) we use
        // consistent output paths
        if let Some(existing_artifact) = ctx.existing_artifact(file, name, version, profile) {
            trace!("use existing artifact file {:?}", existing_artifact,);
            existing_artifact.to_path_buf()
        } else {
            let path = Self::output_file(file, name, version, profile, with_version, with_profile);

            let path = artifacts_folder.join(path);

            if already_taken.contains(&path.to_slash_lossy().to_lowercase()) {
                // preventing conflict
                Self::conflict_free_output_file(already_taken, path, file, artifacts_folder)
            } else {
                path
            }
        }
    }

    /// Convert the compiler output into a set of artifacts
    ///
    /// **Note:** This does only convert, but _NOT_ write the artifacts to disk, See
    /// [`Self::on_output()`]
    fn output_to_artifacts<C>(
        &self,
        contracts: &VersionedContracts<Self::CompilerContract>,
        sources: &VersionedSourceFiles,
        ctx: OutputContext<'_>,
        layout: &ProjectPathsConfig<C>,
        primary_profiles: &HashMap<PathBuf, &str>,
    ) -> Artifacts<Self::Artifact> {
        let mut artifacts = ArtifactsMap::new();

        // this tracks all the `SourceFile`s that we successfully mapped to a contract
        let mut non_standalone_sources = HashSet::new();

        // prepopulate taken paths set with cached artifacts
        let mut taken_paths_lowercase = ctx
            .existing_artifacts
            .values()
            .flat_map(|artifacts| artifacts.values())
            .flat_map(|artifacts| artifacts.values())
            .flat_map(|artifacts| artifacts.values())
            .map(|a| a.path.to_slash_lossy().to_lowercase())
            .collect::<HashSet<_>>();

        let mut files = contracts.keys().collect::<Vec<_>>();
        // Iterate starting with top-most files to ensure that they get the shortest paths.
        files.sort_by(|&file1, &file2| {
            (file1.components().count(), file1).cmp(&(file2.components().count(), file2))
        });
        for file in files {
            for (name, versioned_contracts) in &contracts[file] {
                let unique_versions =
                    versioned_contracts.iter().map(|c| &c.version).collect::<HashSet<_>>();
                let unique_profiles =
                    versioned_contracts.iter().map(|c| &c.profile).collect::<HashSet<_>>();
                let primary_profile = primary_profiles.get(file);

                for contract in versioned_contracts {
                    non_standalone_sources.insert(file);

                    // track `SourceFile`s that can be mapped to contracts
                    let source_file = sources.find_file_and_version(file, &contract.version);

                    let artifact_path = Self::get_artifact_path(
                        &ctx,
                        &taken_paths_lowercase,
                        file,
                        name,
                        layout.artifacts.as_path(),
                        &contract.version,
                        &contract.profile,
                        unique_versions.len() > 1,
                        unique_profiles.len() > 1
                            && primary_profile.is_none_or(|p| p != &contract.profile),
                    );

                    taken_paths_lowercase.insert(artifact_path.to_slash_lossy().to_lowercase());

                    trace!(
                        "use artifact file {:?} for contract file {} {}",
                        artifact_path,
                        file.display(),
                        contract.version
                    );

                    let artifact = self.contract_to_artifact(
                        file,
                        name,
                        contract.contract.clone(),
                        source_file,
                    );

                    let artifact = ArtifactFile {
                        artifact,
                        file: artifact_path,
                        version: contract.version.clone(),
                        build_id: contract.build_id.clone(),
                        profile: contract.profile.clone(),
                    };

                    artifacts
                        .entry(file.to_path_buf())
                        .or_default()
                        .entry(name.to_string())
                        .or_default()
                        .push(artifact);
                }
            }
        }

        // extend with standalone source files and convert them to artifacts
        // this is unfortunately necessary, so we can "mock" `Artifacts` for solidity files without
        // any contract definition, which are not included in the `CompilerOutput` but we want to
        // create Artifacts for them regardless
        for (file, sources) in sources.as_ref().iter() {
            let unique_versions = sources.iter().map(|s| &s.version).collect::<HashSet<_>>();
            let unique_profiles = sources.iter().map(|s| &s.profile).collect::<HashSet<_>>();
            for source in sources {
                if !non_standalone_sources.contains(file) {
                    // scan the ast as a safe measure to ensure this file does not include any
                    // source units
                    // there's also no need to create a standalone artifact for source files that
                    // don't contain an ast
                    if source.source_file.ast.is_none()
                        || source.source_file.contains_contract_definition()
                    {
                        continue;
                    }

                    // we use file and file stem
                    if let Some(name) = Path::new(file).file_stem().and_then(|stem| stem.to_str()) {
                        if let Some(artifact) =
                            self.standalone_source_file_to_artifact(file, source)
                        {
                            let artifact_path = Self::get_artifact_path(
                                &ctx,
                                &taken_paths_lowercase,
                                file,
                                name,
                                &layout.artifacts,
                                &source.version,
                                &source.profile,
                                unique_versions.len() > 1,
                                unique_profiles.len() > 1,
                            );

                            taken_paths_lowercase
                                .insert(artifact_path.to_slash_lossy().to_lowercase());

                            artifacts
                                .entry(file.clone())
                                .or_default()
                                .entry(name.to_string())
                                .or_default()
                                .push(ArtifactFile {
                                    artifact,
                                    file: artifact_path,
                                    version: source.version.clone(),
                                    build_id: source.build_id.clone(),
                                    profile: source.profile.clone(),
                                });
                        }
                    }
                }
            }
        }

        Artifacts(artifacts)
    }

    /// This converts a `SourceFile` that doesn't contain _any_ contract definitions (interfaces,
    /// contracts, libraries) to an artifact.
    ///
    /// We do this because not all `SourceFile`s emitted by solc have at least 1 corresponding entry
    /// in the `contracts`
    /// section of the solc output. For example for an `errors.sol` that only contains custom error
    /// definitions and no contract, no `Contract` object will be generated by solc. However, we
    /// still want to emit an `Artifact` for that file that may include the `ast`, docs etc.,
    /// because other tools depend on this, such as slither.
    fn standalone_source_file_to_artifact(
        &self,
        _path: &Path,
        _file: &VersionedSourceFile,
    ) -> Option<Self::Artifact>;

    /// Handler allowing artifacts handler to enforce artifact recompilation.
    fn is_dirty(&self, _artifact_file: &ArtifactFile<Self::Artifact>) -> Result<bool> {
        Ok(false)
    }

    /// Invoked with all artifacts that were not recompiled.
    fn handle_cached_artifacts(&self, _artifacts: &Artifacts<Self::Artifact>) -> Result<()> {
        Ok(())
    }
}

/// Additional context to use during [`ArtifactOutput::on_output()`]
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct OutputContext<'a> {
    /// Cache file of the project or empty if no caching is enabled
    ///
    /// This context is required for partially cached recompile with conflicting files, so that we
    /// can use the same adjusted output path for conflicting files like:
    ///
    /// ```text
    /// src
    /// ├── a.sol
    /// └── inner
    ///     └── a.sol
    /// ```
    pub existing_artifacts: BTreeMap<&'a Path, &'a CachedArtifacts>,
}

// === impl OutputContext

impl<'a> OutputContext<'a> {
    /// Create a new context with the given cache file
    pub fn new<S>(cache: &'a CompilerCache<S>) -> Self {
        let existing_artifacts = cache
            .files
            .iter()
            .map(|(file, entry)| (file.as_path(), &entry.artifacts))
            .collect::<BTreeMap<_, _>>();

        Self { existing_artifacts }
    }

    /// Returns the path of the already existing artifact for the `contract` of the `file` compiled
    /// with the `version`.
    ///
    /// Returns `None` if no file exists
    pub fn existing_artifact(
        &self,
        file: &Path,
        contract: &str,
        version: &Version,
        profile: &str,
    ) -> Option<&Path> {
        self.existing_artifacts
            .get(file)
            .and_then(|contracts| contracts.get(contract))
            .and_then(|versions| versions.get(version))
            .and_then(|profiles| profiles.get(profile))
            .map(|a| a.path.as_path())
    }
}

/// An `Artifact` implementation that uses a compact representation
///
/// Creates a single json artifact with
/// ```json
///  {
///    "abi": [],
///    "bytecode": {...},
///    "deployedBytecode": {...}
///  }
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MinimalCombinedArtifacts {
    _priv: (),
}

impl ArtifactOutput for MinimalCombinedArtifacts {
    type Artifact = CompactContractBytecode;
    type CompilerContract = Contract;

    fn contract_to_artifact(
        &self,
        _file: &Path,
        _name: &str,
        contract: Contract,
        _source_file: Option<&SourceFile>,
    ) -> Self::Artifact {
        Self::Artifact::from(contract)
    }

    fn standalone_source_file_to_artifact(
        &self,
        _path: &Path,
        _file: &VersionedSourceFile,
    ) -> Option<Self::Artifact> {
        None
    }
}

/// An Artifacts handler implementation that works the same as `MinimalCombinedArtifacts` but also
/// supports reading hardhat artifacts if an initial attempt to deserialize an artifact failed
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MinimalCombinedArtifactsHardhatFallback {
    _priv: (),
}

impl ArtifactOutput for MinimalCombinedArtifactsHardhatFallback {
    type Artifact = CompactContractBytecode;
    type CompilerContract = Contract;

    fn on_output<C>(
        &self,
        output: &VersionedContracts<Contract>,
        sources: &VersionedSourceFiles,
        layout: &ProjectPathsConfig<C>,
        ctx: OutputContext<'_>,
        primary_profiles: &HashMap<PathBuf, &str>,
    ) -> Result<Artifacts<Self::Artifact>> {
        MinimalCombinedArtifacts::default().on_output(
            output,
            sources,
            layout,
            ctx,
            primary_profiles,
        )
    }

    fn read_cached_artifact(path: &Path) -> Result<Self::Artifact> {
        let content = fs::read_to_string(path).map_err(|err| SolcError::io(err, path))?;
        if let Ok(a) = serde_json::from_str(&content) {
            Ok(a)
        } else {
            error!("Failed to deserialize compact artifact");
            trace!("Fallback to hardhat artifact deserialization");
            let artifact = serde_json::from_str::<HardhatArtifact>(&content)?;
            trace!("successfully deserialized hardhat artifact");
            Ok(artifact.into_contract_bytecode())
        }
    }

    fn contract_to_artifact(
        &self,
        file: &Path,
        name: &str,
        contract: Contract,
        source_file: Option<&SourceFile>,
    ) -> Self::Artifact {
        MinimalCombinedArtifacts::default().contract_to_artifact(file, name, contract, source_file)
    }

    fn standalone_source_file_to_artifact(
        &self,
        path: &Path,
        file: &VersionedSourceFile,
    ) -> Option<Self::Artifact> {
        MinimalCombinedArtifacts::default().standalone_source_file_to_artifact(path, file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_artifact() {
        fn assert_artifact<T: Artifact>() {}

        assert_artifact::<CompactContractBytecode>();
        assert_artifact::<serde_json::Value>();
    }

    #[test]
    fn can_find_alternate_paths() {
        let mut already_taken = HashSet::new();

        let file = Path::new("v1/tokens/Greeter.sol");
        let conflict = PathBuf::from("out/Greeter.sol/Greeter.json");
        let artifacts_folder = Path::new("out");

        let alternative = ConfigurableArtifacts::conflict_free_output_file(
            &already_taken,
            conflict.clone(),
            file,
            artifacts_folder,
        );
        assert_eq!(alternative.to_slash_lossy(), "out/tokens/Greeter.sol/Greeter.json");

        already_taken.insert("out/tokens/Greeter.sol/Greeter.json".to_lowercase());
        let alternative = ConfigurableArtifacts::conflict_free_output_file(
            &already_taken,
            conflict.clone(),
            file,
            artifacts_folder,
        );
        assert_eq!(alternative.to_slash_lossy(), "out/v1/tokens/Greeter.sol/Greeter.json");

        already_taken.insert("out/v1/tokens/Greeter.sol/Greeter.json".to_lowercase());
        let alternative = ConfigurableArtifacts::conflict_free_output_file(
            &already_taken,
            conflict,
            file,
            artifacts_folder,
        );
        assert_eq!(alternative, PathBuf::from("Greeter.sol_1/Greeter.json"));
    }

    #[test]
    fn can_find_alternate_path_conflict() {
        let mut already_taken = HashSet::new();

        let file = "/Users/carter/dev/goldfinch/mono/packages/protocol/test/forge/mainnet/utils/BaseMainnetForkingTest.t.sol";
        let conflict = PathBuf::from("/Users/carter/dev/goldfinch/mono/packages/protocol/artifacts/BaseMainnetForkingTest.t.sol/BaseMainnetForkingTest.json");
        already_taken.insert("/Users/carter/dev/goldfinch/mono/packages/protocol/artifacts/BaseMainnetForkingTest.t.sol/BaseMainnetForkingTest.json".into());

        let alternative = ConfigurableArtifacts::conflict_free_output_file(
            &already_taken,
            conflict,
            file.as_ref(),
            "/Users/carter/dev/goldfinch/mono/packages/protocol/artifacts".as_ref(),
        );

        assert_eq!(alternative.to_slash_lossy(), "/Users/carter/dev/goldfinch/mono/packages/protocol/artifacts/utils/BaseMainnetForkingTest.t.sol/BaseMainnetForkingTest.json");
    }

    fn assert_artifact<T: crate::Artifact>() {}

    #[test]
    fn test() {
        assert_artifact::<CompactContractBytecode>();
        assert_artifact::<CompactContractBytecodeCow<'static>>();
    }
}
