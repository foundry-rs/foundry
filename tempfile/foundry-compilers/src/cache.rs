//! Support for compiling contracts.

use crate::{
    buildinfo::RawBuildInfo,
    compilers::{Compiler, CompilerSettings, Language},
    output::Builds,
    resolver::GraphEdges,
    ArtifactFile, ArtifactOutput, Artifacts, ArtifactsMap, Graph, OutputContext, Project,
    ProjectPaths, ProjectPathsConfig, SourceCompilationKind,
};
use foundry_compilers_artifacts::{
    sources::{Source, Sources},
    Settings,
};
use foundry_compilers_core::{
    error::{Result, SolcError},
    utils::{self, strip_prefix},
};
use semver::Version;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::{btree_map::BTreeMap, hash_map, BTreeSet, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

/// ethers-rs format version
///
/// `ethers-solc` uses a different format version id, but the actual format is consistent with
/// hardhat This allows ethers-solc to detect if the cache file was written by hardhat or
/// `ethers-solc`
const ETHERS_FORMAT_VERSION: &str = "ethers-rs-sol-cache-4";

/// The file name of the default cache file
pub const SOLIDITY_FILES_CACHE_FILENAME: &str = "solidity-files-cache.json";

/// A multi version cache file
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompilerCache<S = Settings> {
    #[serde(rename = "_format")]
    pub format: String,
    /// contains all directories used for the project
    pub paths: ProjectPaths,
    pub files: BTreeMap<PathBuf, CacheEntry>,
    pub builds: BTreeSet<String>,
    pub profiles: BTreeMap<String, S>,
}

impl<S> CompilerCache<S> {
    pub fn new(format: String, paths: ProjectPaths) -> Self {
        Self {
            format,
            paths,
            files: Default::default(),
            builds: Default::default(),
            profiles: Default::default(),
        }
    }
}

impl<S: CompilerSettings> CompilerCache<S> {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Removes entry for the given file
    pub fn remove(&mut self, file: &Path) -> Option<CacheEntry> {
        self.files.remove(file)
    }

    /// How many entries the cache contains where each entry represents a sourc file
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// How many `Artifacts` this cache references, where a source file can have multiple artifacts
    pub fn artifacts_len(&self) -> usize {
        self.entries().map(|entry| entry.artifacts().count()).sum()
    }

    /// Returns an iterator over all `CacheEntry` this cache contains
    pub fn entries(&self) -> impl Iterator<Item = &CacheEntry> {
        self.files.values()
    }

    /// Returns the corresponding `CacheEntry` for the file if it exists
    pub fn entry(&self, file: &Path) -> Option<&CacheEntry> {
        self.files.get(file)
    }

    /// Returns the corresponding `CacheEntry` for the file if it exists
    pub fn entry_mut(&mut self, file: &Path) -> Option<&mut CacheEntry> {
        self.files.get_mut(file)
    }

    /// Reads the cache json file from the given path
    ///
    /// See also [`Self::read_joined()`]
    ///
    /// # Errors
    ///
    /// If the cache file does not exist
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::{cache::CompilerCache, solc::SolcSettings, Project};
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let mut cache = CompilerCache::<SolcSettings>::read(project.cache_path())?;
    /// cache.join_artifacts_files(project.artifacts_path());
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    #[instrument(skip_all, name = "sol-files-cache::read")]
    pub fn read(path: &Path) -> Result<Self> {
        trace!("reading solfiles cache at {}", path.display());
        let cache: Self = utils::read_json_file(path)?;
        trace!("read cache \"{}\" with {} entries", cache.format, cache.files.len());
        Ok(cache)
    }

    /// Reads the cache json file from the given path and returns the cache with paths adjoined to
    /// the `ProjectPathsConfig`.
    ///
    /// This expects the `artifact` files to be relative to the artifacts dir of the `paths` and the
    /// `CachEntry` paths to be relative to the root dir of the `paths`
    ///
    ///
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::{cache::CompilerCache, solc::SolcSettings, Project};
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let cache: CompilerCache<SolcSettings> = CompilerCache::read_joined(&project.paths)?;
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn read_joined<L>(paths: &ProjectPathsConfig<L>) -> Result<Self> {
        let mut cache = Self::read(&paths.cache)?;
        cache.join_entries(&paths.root).join_artifacts_files(&paths.artifacts);
        Ok(cache)
    }

    /// Write the cache as json file to the given path
    pub fn write(&self, path: &Path) -> Result<()> {
        trace!("writing cache with {} entries to json file: \"{}\"", self.len(), path.display());
        utils::create_parent_dir_all(path)?;
        utils::write_json_file(self, path, 128 * 1024)?;
        trace!("cache file located: \"{}\"", path.display());
        Ok(())
    }

    /// Removes build infos which don't have any artifacts linked to them.
    pub fn remove_outdated_builds(&mut self) {
        let mut outdated = Vec::new();
        for build_id in &self.builds {
            if !self
                .entries()
                .flat_map(|e| e.artifacts.values())
                .flat_map(|a| a.values())
                .flat_map(|a| a.values())
                .any(|a| a.build_id == *build_id)
            {
                outdated.push(build_id.to_owned());
            }
        }

        for build_id in outdated {
            self.builds.remove(&build_id);
            let path = self.paths.build_infos.join(build_id).with_extension("json");
            let _ = std::fs::remove_file(path);
        }
    }

    /// Sets the `CacheEntry`'s file paths to `root` adjoined to `self.file`.
    pub fn join_entries(&mut self, root: &Path) -> &mut Self {
        self.files = std::mem::take(&mut self.files)
            .into_iter()
            .map(|(path, entry)| (root.join(path), entry))
            .collect();
        self
    }

    /// Removes `base` from all `CacheEntry` paths
    pub fn strip_entries_prefix(&mut self, base: &Path) -> &mut Self {
        self.files = std::mem::take(&mut self.files)
            .into_iter()
            .map(|(path, entry)| (path.strip_prefix(base).map(Into::into).unwrap_or(path), entry))
            .collect();
        self
    }

    /// Sets the artifact files location to `base` adjoined to the `CachEntries` artifacts.
    pub fn join_artifacts_files(&mut self, base: &Path) -> &mut Self {
        self.files.values_mut().for_each(|entry| entry.join_artifacts_files(base));
        self
    }

    /// Removes `base` from all artifact file paths
    pub fn strip_artifact_files_prefixes(&mut self, base: &Path) -> &mut Self {
        self.files.values_mut().for_each(|entry| entry.strip_artifact_files_prefixes(base));
        self
    }

    /// Removes all `CacheEntry` which source files don't exist on disk
    ///
    /// **NOTE:** this assumes the `files` are absolute
    pub fn remove_missing_files(&mut self) {
        trace!("remove non existing files from cache");
        self.files.retain(|file, _| {
            let exists = file.exists();
            if !exists {
                trace!("remove {} from cache", file.display());
            }
            exists
        })
    }

    /// Checks if all artifact files exist
    pub fn all_artifacts_exist(&self) -> bool {
        self.files.values().all(|entry| entry.all_artifacts_exist())
    }

    /// Strips the given prefix from all `file` paths that identify a `CacheEntry` to make them
    /// relative to the given `base` argument
    ///
    /// In other words this sets the keys (the file path of a solidity file) relative to the `base`
    /// argument, so that the key `/Users/me/project/src/Greeter.sol` will be changed to
    /// `src/Greeter.sol` if `base` is `/Users/me/project`
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::{
    ///     artifacts::contract::CompactContract, cache::CompilerCache, solc::SolcSettings, Project,
    /// };
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let cache: CompilerCache<SolcSettings> =
    ///     CompilerCache::read(project.cache_path())?.with_stripped_file_prefixes(project.root());
    /// let artifact: CompactContract = cache.read_artifact("src/Greeter.sol".as_ref(), "Greeter")?;
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// **Note:** this only affects the source files, see [`Self::strip_artifact_files_prefixes()`]
    pub fn with_stripped_file_prefixes(mut self, base: &Path) -> Self {
        self.files = self
            .files
            .into_iter()
            .map(|(f, e)| (utils::source_name(&f, base).to_path_buf(), e))
            .collect();
        self
    }

    /// Returns the path to the artifact of the given `(file, contract)` pair
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::{cache::CompilerCache, solc::SolcSettings, Project};
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let cache: CompilerCache<SolcSettings> = CompilerCache::read_joined(&project.paths)?;
    /// cache.find_artifact_path("/Users/git/myproject/src/Greeter.sol".as_ref(), "Greeter");
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn find_artifact_path(&self, contract_file: &Path, contract_name: &str) -> Option<&Path> {
        let entry = self.entry(contract_file)?;
        entry.find_artifact_path(contract_name)
    }

    /// Finds the path to the artifact of the given `(file, contract)` pair (see
    /// [`Self::find_artifact_path()`]) and deserializes the artifact file as JSON.
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::{
    ///     artifacts::contract::CompactContract, cache::CompilerCache, solc::SolcSettings, Project,
    /// };
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let cache = CompilerCache::<SolcSettings>::read_joined(&project.paths)?;
    /// let artifact: CompactContract =
    ///     cache.read_artifact("/Users/git/myproject/src/Greeter.sol".as_ref(), "Greeter")?;
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// **NOTE**: unless the cache's `files` keys were modified `contract_file` is expected to be
    /// absolute.
    pub fn read_artifact<Artifact: DeserializeOwned>(
        &self,
        contract_file: &Path,
        contract_name: &str,
    ) -> Result<Artifact> {
        let artifact_path =
            self.find_artifact_path(contract_file, contract_name).ok_or_else(|| {
                SolcError::ArtifactNotFound(contract_file.to_path_buf(), contract_name.to_string())
            })?;
        utils::read_json_file(artifact_path)
    }

    /// Reads all cached artifacts from disk using the given ArtifactOutput handler
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::{
    ///     artifacts::contract::CompactContractBytecode, cache::CompilerCache, solc::SolcSettings,
    ///     Project,
    /// };
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let cache: CompilerCache<SolcSettings> = CompilerCache::read_joined(&project.paths)?;
    /// let artifacts = cache.read_artifacts::<CompactContractBytecode>()?;
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn read_artifacts<Artifact: DeserializeOwned + Send + Sync>(
        &self,
    ) -> Result<Artifacts<Artifact>> {
        use rayon::prelude::*;

        let artifacts = self
            .files
            .par_iter()
            .map(|(file, entry)| entry.read_artifact_files().map(|files| (file.clone(), files)))
            .collect::<Result<ArtifactsMap<_>>>()?;
        Ok(Artifacts(artifacts))
    }

    /// Reads all cached [BuildContext]s from disk. [BuildContext] is inlined into [RawBuildInfo]
    /// objects, so we are basically just partially deserializing build infos here.
    ///
    /// [BuildContext]: crate::buildinfo::BuildContext
    pub fn read_builds<L: Language>(&self, build_info_dir: &Path) -> Result<Builds<L>> {
        use rayon::prelude::*;

        self.builds
            .par_iter()
            .map(|build_id| {
                utils::read_json_file(&build_info_dir.join(build_id).with_extension("json"))
                    .map(|b| (build_id.clone(), b))
            })
            .collect::<Result<_>>()
            .map(|b| Builds(b))
    }
}

#[cfg(feature = "async")]
impl<S: CompilerSettings> CompilerCache<S> {
    pub async fn async_read(path: &Path) -> Result<Self> {
        let path = path.to_owned();
        Self::asyncify(move || Self::read(&path)).await
    }

    pub async fn async_write(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_vec(self)?;
        tokio::fs::write(path, content).await.map_err(|err| SolcError::io(err, path))
    }

    async fn asyncify<F, T>(f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        match tokio::task::spawn_blocking(f).await {
            Ok(res) => res,
            Err(_) => Err(SolcError::io(
                std::io::Error::new(std::io::ErrorKind::Other, "background task failed"),
                "",
            )),
        }
    }
}

impl<S> Default for CompilerCache<S> {
    fn default() -> Self {
        Self {
            format: ETHERS_FORMAT_VERSION.to_string(),
            builds: Default::default(),
            files: Default::default(),
            paths: Default::default(),
            profiles: Default::default(),
        }
    }
}

impl<'a, S: CompilerSettings> From<&'a ProjectPathsConfig> for CompilerCache<S> {
    fn from(config: &'a ProjectPathsConfig) -> Self {
        let paths = config.paths_relative();
        Self::new(Default::default(), paths)
    }
}

/// Cached artifact data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedArtifact {
    /// Path to the artifact file.
    pub path: PathBuf,
    /// Build id which produced the given artifact.
    pub build_id: String,
}

pub type CachedArtifacts = BTreeMap<String, BTreeMap<Version, BTreeMap<String, CachedArtifact>>>;

/// A `CacheEntry` in the cache file represents a solidity file
///
/// A solidity file can contain several contracts, for every contract a separate `Artifact` is
/// emitted. so the `CacheEntry` tracks the artifacts by name. A file can be compiled with multiple
/// `solc` versions generating version specific artifacts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheEntry {
    /// the last modification time of this file
    pub last_modification_date: u64,
    /// hash to identify whether the content of the file changed
    pub content_hash: String,
    /// identifier name see [`foundry_compilers_core::utils::source_name()`]
    pub source_name: PathBuf,
    /// fully resolved imports of the file
    ///
    /// all paths start relative from the project's root: `src/importedFile.sol`
    pub imports: BTreeSet<PathBuf>,
    /// The solidity version pragma
    pub version_requirement: Option<String>,
    /// all artifacts produced for this file
    ///
    /// In theory a file can be compiled by different solc versions:
    /// `A(<=0.8.10) imports C(>0.4.0)` and `B(0.8.11) imports C(>0.4.0)`
    /// file `C` would be compiled twice, with `0.8.10` and `0.8.11`, producing two different
    /// artifacts.
    ///
    /// This map tracks the artifacts by `name -> (Version -> profile -> PathBuf)`.
    /// This mimics the default artifacts directory structure
    pub artifacts: CachedArtifacts,
    /// Whether this file was compiled at least once.
    ///
    /// If this is true and `artifacts` are empty, it means that given version of the file does
    /// not produce any artifacts and it should not be compiled again.
    ///
    /// If this is false, then artifacts are definitely empty and it should be compiled if we may
    /// need artifacts.
    pub seen_by_compiler: bool,
}

impl CacheEntry {
    /// Returns the last modified timestamp `Duration`
    pub fn last_modified(&self) -> Duration {
        Duration::from_millis(self.last_modification_date)
    }

    /// Returns the artifact path for the contract name.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use foundry_compilers::cache::CacheEntry;
    ///
    /// # fn t(entry: CacheEntry) {
    /// # stringify!(
    /// let entry: CacheEntry = ...;
    /// # );
    /// entry.find_artifact_path("Greeter");
    /// # }
    /// ```
    pub fn find_artifact_path(&self, contract_name: &str) -> Option<&Path> {
        self.artifacts
            .get(contract_name)?
            .iter()
            .next()
            .and_then(|(_, a)| a.iter().next())
            .map(|(_, p)| p.path.as_path())
    }

    /// Reads the last modification date from the file's metadata
    pub fn read_last_modification_date(file: &Path) -> Result<u64> {
        let last_modification_date = fs::metadata(file)
            .map_err(|err| SolcError::io(err, file.to_path_buf()))?
            .modified()
            .map_err(|err| SolcError::io(err, file.to_path_buf()))?
            .duration_since(UNIX_EPOCH)
            .map_err(SolcError::msg)?
            .as_millis() as u64;
        Ok(last_modification_date)
    }

    /// Reads all artifact files associated with the `CacheEntry`
    ///
    /// **Note:** all artifact file paths should be absolute.
    fn read_artifact_files<Artifact: DeserializeOwned>(
        &self,
    ) -> Result<BTreeMap<String, Vec<ArtifactFile<Artifact>>>> {
        let mut artifacts = BTreeMap::new();
        for (artifact_name, versioned_files) in self.artifacts.iter() {
            let mut files = Vec::with_capacity(versioned_files.len());
            for (version, cached_artifact) in versioned_files {
                for (profile, cached_artifact) in cached_artifact {
                    let artifact: Artifact = utils::read_json_file(&cached_artifact.path)?;
                    files.push(ArtifactFile {
                        artifact,
                        file: cached_artifact.path.clone(),
                        version: version.clone(),
                        build_id: cached_artifact.build_id.clone(),
                        profile: profile.clone(),
                    });
                }
            }
            artifacts.insert(artifact_name.clone(), files);
        }
        Ok(artifacts)
    }

    pub(crate) fn merge_artifacts<'a, A, I, T: 'a>(&mut self, artifacts: I)
    where
        I: IntoIterator<Item = (&'a String, A)>,
        A: IntoIterator<Item = &'a ArtifactFile<T>>,
    {
        for (name, artifacts) in artifacts.into_iter() {
            for artifact in artifacts {
                self.artifacts
                    .entry(name.clone())
                    .or_default()
                    .entry(artifact.version.clone())
                    .or_default()
                    .insert(
                        artifact.profile.clone(),
                        CachedArtifact {
                            build_id: artifact.build_id.clone(),
                            path: artifact.file.clone(),
                        },
                    );
            }
        }
    }

    /// Returns `true` if the artifacts set contains the given version
    pub fn contains(&self, version: &Version, profile: &str) -> bool {
        self.artifacts.values().any(|artifacts| {
            artifacts.get(version).and_then(|artifacts| artifacts.get(profile)).is_some()
        })
    }

    /// Iterator that yields all artifact files and their version
    pub fn artifacts_versions(&self) -> impl Iterator<Item = (&Version, &str, &CachedArtifact)> {
        self.artifacts
            .values()
            .flatten()
            .flat_map(|(v, a)| a.iter().map(move |(p, a)| (v, p.as_str(), a)))
    }

    /// Returns the artifact file for the contract and version pair
    pub fn find_artifact(
        &self,
        contract: &str,
        version: &Version,
        profile: &str,
    ) -> Option<&CachedArtifact> {
        self.artifacts
            .get(contract)
            .and_then(|files| files.get(version))
            .and_then(|files| files.get(profile))
    }

    /// Iterator that yields all artifact files and their version
    pub fn artifacts_for_version<'a>(
        &'a self,
        version: &'a Version,
    ) -> impl Iterator<Item = &'a CachedArtifact> + 'a {
        self.artifacts_versions().filter_map(move |(ver, _, file)| (ver == version).then_some(file))
    }

    /// Iterator that yields all artifact files
    pub fn artifacts(&self) -> impl Iterator<Item = &CachedArtifact> {
        self.artifacts.values().flat_map(BTreeMap::values).flat_map(BTreeMap::values)
    }

    /// Mutable iterator over all artifact files
    pub fn artifacts_mut(&mut self) -> impl Iterator<Item = &mut CachedArtifact> {
        self.artifacts.values_mut().flat_map(BTreeMap::values_mut).flat_map(BTreeMap::values_mut)
    }

    /// Checks if all artifact files exist
    pub fn all_artifacts_exist(&self) -> bool {
        self.artifacts().all(|a| a.path.exists())
    }

    /// Sets the artifact's paths to `base` adjoined to the artifact's `path`.
    pub fn join_artifacts_files(&mut self, base: &Path) {
        self.artifacts_mut().for_each(|a| a.path = base.join(&a.path))
    }

    /// Removes `base` from the artifact's path
    pub fn strip_artifact_files_prefixes(&mut self, base: &Path) {
        self.artifacts_mut().for_each(|a| {
            if let Ok(rem) = a.path.strip_prefix(base) {
                a.path = rem.to_path_buf();
            }
        })
    }
}

/// Collection of source file paths mapped to versions.
#[derive(Clone, Debug, Default)]
pub struct GroupedSources {
    pub inner: HashMap<PathBuf, HashSet<Version>>,
}

impl GroupedSources {
    /// Inserts provided source and version into the collection.
    pub fn insert(&mut self, file: PathBuf, version: Version) {
        match self.inner.entry(file) {
            hash_map::Entry::Occupied(mut entry) => {
                entry.get_mut().insert(version);
            }
            hash_map::Entry::Vacant(entry) => {
                entry.insert(HashSet::from([version]));
            }
        }
    }

    /// Returns true if the file was included with the given version.
    pub fn contains(&self, file: &Path, version: &Version) -> bool {
        self.inner.get(file).is_some_and(|versions| versions.contains(version))
    }
}

/// A helper abstraction over the [`CompilerCache`] used to determine what files need to compiled
/// and which `Artifacts` can be reused.
#[derive(Debug)]
pub(crate) struct ArtifactsCacheInner<
    'a,
    T: ArtifactOutput<CompilerContract = C::CompilerContract>,
    C: Compiler,
> {
    /// The preexisting cache file.
    pub cache: CompilerCache<C::Settings>,

    /// All already existing artifacts.
    pub cached_artifacts: Artifacts<T::Artifact>,

    /// All already existing build infos.
    pub cached_builds: Builds<C::Language>,

    /// Relationship between all the files.
    pub edges: GraphEdges<C::ParsedSource>,

    /// The project.
    pub project: &'a Project<C, T>,

    /// Files that were invalidated and removed from cache.
    /// Those are not grouped by version and purged completely.
    pub dirty_sources: HashSet<PathBuf>,

    /// Artifact+version pairs which are in scope for each solc version.
    ///
    /// Only those files will be included into cached artifacts list for each version.
    pub sources_in_scope: GroupedSources,

    /// The file hashes.
    pub content_hashes: HashMap<PathBuf, String>,
}

impl<T: ArtifactOutput<CompilerContract = C::CompilerContract>, C: Compiler>
    ArtifactsCacheInner<'_, T, C>
{
    /// Creates a new cache entry for the file
    fn create_cache_entry(&mut self, file: PathBuf, source: &Source) {
        let imports = self
            .edges
            .imports(&file)
            .into_iter()
            .map(|import| strip_prefix(import, self.project.root()).into())
            .collect();

        let entry = CacheEntry {
            last_modification_date: CacheEntry::read_last_modification_date(&file)
                .unwrap_or_default(),
            content_hash: source.content_hash(),
            source_name: strip_prefix(&file, self.project.root()).into(),
            imports,
            version_requirement: self.edges.version_requirement(&file).map(|v| v.to_string()),
            // artifacts remain empty until we received the compiler output
            artifacts: Default::default(),
            seen_by_compiler: false,
        };

        self.cache.files.insert(file, entry);
    }

    /// Returns the set of [Source]s that need to be compiled to produce artifacts for requested
    /// input.
    ///
    /// Source file may have one of the two [SourceCompilationKind]s:
    /// 1. [SourceCompilationKind::Complete] - the file has been modified or compiled with different
    ///    settings and its cache is invalidated. For such sources we request full data needed for
    ///    artifact construction.
    /// 2. [SourceCompilationKind::Optimized] - the file is not dirty, but is imported by a dirty
    ///    file and thus will be processed by solc. For such files we don't need full data, so we
    ///    are marking them as clean to optimize output selection later.
    fn filter(&mut self, sources: &mut Sources, version: &Version, profile: &str) {
        // sources that should be passed to compiler.
        let mut compile_complete = HashSet::new();
        let mut compile_optimized = HashSet::new();

        for (file, source) in sources.iter() {
            self.sources_in_scope.insert(file.clone(), version.clone());

            // If we are missing artifact for file, compile it.
            if self.is_missing_artifacts(file, version, profile) {
                compile_complete.insert(file.clone());
            }

            // Ensure that we have a cache entry for all sources.
            if !self.cache.files.contains_key(file) {
                self.create_cache_entry(file.clone(), source);
            }
        }

        // Prepare optimization by collecting sources which are imported by files requiring complete
        // compilation.
        for source in &compile_complete {
            for import in self.edges.imports(source) {
                if !compile_complete.contains(import) {
                    compile_optimized.insert(import.clone());
                }
            }
        }

        sources.retain(|file, source| {
            source.kind = if compile_complete.contains(file) {
                SourceCompilationKind::Complete
            } else if compile_optimized.contains(file) {
                SourceCompilationKind::Optimized
            } else {
                return false;
            };
            true
        });
    }

    /// Returns whether we are missing artifacts for the given file and version.
    #[instrument(level = "trace", skip(self))]
    fn is_missing_artifacts(&self, file: &Path, version: &Version, profile: &str) -> bool {
        let Some(entry) = self.cache.entry(file) else {
            trace!("missing cache entry");
            return true;
        };

        // only check artifact's existence if the file generated artifacts.
        // e.g. a solidity file consisting only of import statements (like interfaces that
        // re-export) do not create artifacts
        if entry.seen_by_compiler && entry.artifacts.is_empty() {
            trace!("no artifacts");
            return false;
        }

        if !entry.contains(version, profile) {
            trace!("missing linked artifacts");
            return true;
        }

        if entry.artifacts_for_version(version).any(|artifact| {
            let missing_artifact = !self.cached_artifacts.has_artifact(&artifact.path);
            if missing_artifact {
                trace!("missing artifact \"{}\"", artifact.path.display());
            }
            missing_artifact
        }) {
            return true;
        }

        false
    }

    // Walks over all cache entires, detects dirty files and removes them from cache.
    fn find_and_remove_dirty(&mut self) {
        fn populate_dirty_files<D>(
            file: &Path,
            dirty_files: &mut HashSet<PathBuf>,
            edges: &GraphEdges<D>,
        ) {
            for file in edges.importers(file) {
                // If file is marked as dirty we either have already visited it or it was marked as
                // dirty initially and will be visited at some point later.
                if !dirty_files.contains(file) {
                    dirty_files.insert(file.to_path_buf());
                    populate_dirty_files(file, dirty_files, edges);
                }
            }
        }

        let existing_profiles = self.project.settings_profiles().collect::<BTreeMap<_, _>>();

        let mut dirty_profiles = HashSet::new();
        for (profile, settings) in &self.cache.profiles {
            if !existing_profiles.get(profile.as_str()).is_some_and(|p| p.can_use_cached(settings))
            {
                trace!("dirty profile: {}", profile);
                dirty_profiles.insert(profile.clone());
            }
        }

        for profile in &dirty_profiles {
            self.cache.profiles.remove(profile);
        }

        self.cache.files.retain(|_, entry| {
            // keep entries which already had no artifacts
            if entry.artifacts.is_empty() {
                return true;
            }
            entry.artifacts.retain(|_, artifacts| {
                artifacts.retain(|_, artifacts| {
                    artifacts.retain(|profile, _| !dirty_profiles.contains(profile));
                    !artifacts.is_empty()
                });
                !artifacts.is_empty()
            });
            !entry.artifacts.is_empty()
        });

        for (profile, settings) in existing_profiles {
            if !self.cache.profiles.contains_key(profile) {
                self.cache.profiles.insert(profile.to_string(), settings.clone());
            }
        }

        // Iterate over existing cache entries.
        let files = self.cache.files.keys().cloned().collect::<HashSet<_>>();

        let mut sources = Sources::new();

        // Read all sources, marking entries as dirty on I/O errors.
        for file in &files {
            let Ok(source) = Source::read(file) else {
                self.dirty_sources.insert(file.clone());
                continue;
            };
            sources.insert(file.clone(), source);
        }

        // Build a temporary graph for walking imports. We need this because `self.edges`
        // only contains graph data for in-scope sources but we are operating on cache entries.
        if let Ok(graph) = Graph::<C::ParsedSource>::resolve_sources(&self.project.paths, sources) {
            let (sources, edges) = graph.into_sources();

            // Calculate content hashes for later comparison.
            self.fill_hashes(&sources);

            // Pre-add all sources that are guaranteed to be dirty
            for file in sources.keys() {
                if self.is_dirty_impl(file) {
                    self.dirty_sources.insert(file.clone());
                }
            }

            // Perform DFS to find direct/indirect importers of dirty files.
            for file in self.dirty_sources.clone().iter() {
                populate_dirty_files(file, &mut self.dirty_sources, &edges);
            }
        } else {
            // Purge all sources on graph resolution error.
            self.dirty_sources.extend(files);
        }

        // Remove all dirty files from cache.
        for file in &self.dirty_sources {
            debug!("removing dirty file from cache: {}", file.display());
            self.cache.remove(file);
        }
    }

    fn is_dirty_impl(&self, file: &Path) -> bool {
        let Some(hash) = self.content_hashes.get(file) else {
            trace!("missing content hash");
            return true;
        };

        let Some(entry) = self.cache.entry(file) else {
            trace!("missing cache entry");
            return true;
        };

        if entry.content_hash != *hash {
            trace!("content hash changed");
            return true;
        }

        // If any requested extra files are missing for any artifact, mark source as dirty to
        // generate them
        for artifacts in self.cached_artifacts.values() {
            for artifacts in artifacts.values() {
                for artifact_file in artifacts {
                    if self.project.artifacts_handler().is_dirty(artifact_file).unwrap_or(true) {
                        return true;
                    }
                }
            }
        }

        // all things match, can be reused
        false
    }

    /// Adds the file's hashes to the set if not set yet
    fn fill_hashes(&mut self, sources: &Sources) {
        for (file, source) in sources {
            if let hash_map::Entry::Vacant(entry) = self.content_hashes.entry(file.clone()) {
                entry.insert(source.content_hash());
            }
        }
    }
}

/// Abstraction over configured caching which can be either non-existent or an already loaded cache
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub(crate) enum ArtifactsCache<
    'a,
    T: ArtifactOutput<CompilerContract = C::CompilerContract>,
    C: Compiler,
> {
    /// Cache nothing on disk
    Ephemeral(GraphEdges<C::ParsedSource>, &'a Project<C, T>),
    /// Handles the actual cached artifacts, detects artifacts that can be reused
    Cached(ArtifactsCacheInner<'a, T, C>),
}

impl<'a, T: ArtifactOutput<CompilerContract = C::CompilerContract>, C: Compiler>
    ArtifactsCache<'a, T, C>
{
    /// Create a new cache instance with the given files
    pub fn new(project: &'a Project<C, T>, edges: GraphEdges<C::ParsedSource>) -> Result<Self> {
        /// Returns the [CompilerCache] to use
        ///
        /// Returns a new empty cache if the cache does not exist or `invalidate_cache` is set.
        fn get_cache<T: ArtifactOutput<CompilerContract = C::CompilerContract>, C: Compiler>(
            project: &Project<C, T>,
            invalidate_cache: bool,
        ) -> CompilerCache<C::Settings> {
            // the currently configured paths
            let paths = project.paths.paths_relative();

            if !invalidate_cache && project.cache_path().exists() {
                if let Ok(cache) = CompilerCache::read_joined(&project.paths) {
                    if cache.paths == paths {
                        // unchanged project paths
                        return cache;
                    }
                }
            }

            // new empty cache
            CompilerCache::new(Default::default(), paths)
        }

        let cache = if project.cached {
            // we only read the existing cache if we were able to resolve the entire graph
            // if we failed to resolve an import we invalidate the cache so don't get any false
            // positives
            let invalidate_cache = !edges.unresolved_imports().is_empty();

            // read the cache file if it already exists
            let mut cache = get_cache(project, invalidate_cache);

            cache.remove_missing_files();

            // read all artifacts
            let mut cached_artifacts = if project.paths.artifacts.exists() {
                trace!("reading artifacts from cache...");
                // if we failed to read the whole set of artifacts we use an empty set
                let artifacts = cache.read_artifacts::<T::Artifact>().unwrap_or_default();
                trace!("read {} artifacts from cache", artifacts.artifact_files().count());
                artifacts
            } else {
                Default::default()
            };

            trace!("reading build infos from cache...");
            let cached_builds = cache.read_builds(&project.paths.build_infos).unwrap_or_default();

            // Remove artifacts for which we are missing a build info.
            cached_artifacts.0.retain(|_, artifacts| {
                artifacts.retain(|_, artifacts| {
                    artifacts.retain(|artifact| cached_builds.contains_key(&artifact.build_id));
                    !artifacts.is_empty()
                });
                !artifacts.is_empty()
            });

            let cache = ArtifactsCacheInner {
                cache,
                cached_artifacts,
                cached_builds,
                edges,
                project,
                dirty_sources: Default::default(),
                content_hashes: Default::default(),
                sources_in_scope: Default::default(),
            };

            ArtifactsCache::Cached(cache)
        } else {
            // nothing to cache
            ArtifactsCache::Ephemeral(edges, project)
        };

        Ok(cache)
    }

    /// Returns the graph data for this project
    pub fn graph(&self) -> &GraphEdges<C::ParsedSource> {
        match self {
            ArtifactsCache::Ephemeral(graph, _) => graph,
            ArtifactsCache::Cached(inner) => &inner.edges,
        }
    }

    #[cfg(test)]
    #[allow(unused)]
    #[doc(hidden)]
    // only useful for debugging for debugging purposes
    pub fn as_cached(&self) -> Option<&ArtifactsCacheInner<'a, T, C>> {
        match self {
            ArtifactsCache::Ephemeral(..) => None,
            ArtifactsCache::Cached(cached) => Some(cached),
        }
    }

    pub fn output_ctx(&self) -> OutputContext<'_> {
        match self {
            ArtifactsCache::Ephemeral(..) => Default::default(),
            ArtifactsCache::Cached(inner) => OutputContext::new(&inner.cache),
        }
    }

    pub fn project(&self) -> &'a Project<C, T> {
        match self {
            ArtifactsCache::Ephemeral(_, project) => project,
            ArtifactsCache::Cached(cache) => cache.project,
        }
    }

    /// Adds the file's hashes to the set if not set yet
    pub fn remove_dirty_sources(&mut self) {
        match self {
            ArtifactsCache::Ephemeral(..) => {}
            ArtifactsCache::Cached(cache) => cache.find_and_remove_dirty(),
        }
    }

    /// Filters out those sources that don't need to be compiled
    pub fn filter(&mut self, sources: &mut Sources, version: &Version, profile: &str) {
        match self {
            ArtifactsCache::Ephemeral(..) => {}
            ArtifactsCache::Cached(cache) => cache.filter(sources, version, profile),
        }
    }

    /// Consumes the `Cache`, rebuilds the `SolFileCache` by merging all artifacts that were
    /// filtered out in the previous step (`Cache::filtered`) and the artifacts that were just
    /// compiled and written to disk `written_artifacts`.
    ///
    /// Returns all the _cached_ artifacts.
    pub fn consume<A>(
        self,
        written_artifacts: &Artifacts<A>,
        written_build_infos: &Vec<RawBuildInfo<C::Language>>,
        write_to_disk: bool,
    ) -> Result<(Artifacts<A>, Builds<C::Language>)>
    where
        T: ArtifactOutput<Artifact = A>,
    {
        let ArtifactsCache::Cached(cache) = self else {
            trace!("no cache configured, ephemeral");
            return Ok(Default::default());
        };

        let ArtifactsCacheInner {
            mut cache,
            mut cached_artifacts,
            cached_builds,
            dirty_sources,
            sources_in_scope,
            project,
            ..
        } = cache;

        // Remove cached artifacts which are out of scope, dirty or appear in `written_artifacts`.
        cached_artifacts.0.retain(|file, artifacts| {
            let file = Path::new(file);
            artifacts.retain(|name, artifacts| {
                artifacts.retain(|artifact| {
                    let version = &artifact.version;

                    if !sources_in_scope.contains(file, version) {
                        return false;
                    }
                    if dirty_sources.contains(file) {
                        return false;
                    }
                    if written_artifacts.find_artifact(file, name, version).is_some() {
                        return false;
                    }
                    true
                });
                !artifacts.is_empty()
            });
            !artifacts.is_empty()
        });

        // Update cache entries with newly written artifacts. We update data for any artifacts as
        // `written_artifacts` always contain the most recent data.
        for (file, artifacts) in written_artifacts.as_ref() {
            let file_path = Path::new(file);
            // Only update data for existing entries, we should have entries for all in-scope files
            // by now.
            if let Some(entry) = cache.files.get_mut(file_path) {
                entry.merge_artifacts(artifacts);
            }
        }

        for build_info in written_build_infos {
            cache.builds.insert(build_info.id.clone());
        }

        // write to disk
        if write_to_disk {
            cache.remove_outdated_builds();
            // make all `CacheEntry` paths relative to the project root and all artifact
            // paths relative to the artifact's directory
            cache
                .strip_entries_prefix(project.root())
                .strip_artifact_files_prefixes(project.artifacts_path());
            cache.write(project.cache_path())?;
        }

        Ok((cached_artifacts, cached_builds))
    }

    /// Marks the cached entry as seen by the compiler, if it's cached.
    pub fn compiler_seen(&mut self, file: &Path) {
        if let ArtifactsCache::Cached(cache) = self {
            if let Some(entry) = cache.cache.entry_mut(file) {
                entry.seen_by_compiler = true;
            }
        }
    }
}
