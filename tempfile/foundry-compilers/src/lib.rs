#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

#[cfg(feature = "project-util")]
#[macro_use]
extern crate foundry_compilers_core;

mod artifact_output;
pub use artifact_output::*;

pub mod buildinfo;

pub mod cache;

pub mod flatten;

pub mod resolver;
pub use resolver::Graph;

pub mod compilers;
pub use compilers::*;

mod compile;
pub use compile::{
    output::{AggregatedCompilerOutput, ProjectCompileOutput},
    *,
};

mod config;
pub use config::{PathStyle, ProjectPaths, ProjectPathsConfig, SolcConfig};

mod filter;
pub use filter::{FileFilter, SparseOutputFilter, TestFileFilter};

pub mod report;

/// Utilities for creating, mocking and testing of (temporary) projects
#[cfg(feature = "project-util")]
pub mod project_util;

pub use foundry_compilers_artifacts as artifacts;
pub use foundry_compilers_core::{error, utils};

use cache::CompilerCache;
use compile::output::contracts::VersionedContracts;
use compilers::multi::MultiCompiler;
use foundry_compilers_artifacts::{
    output_selection::OutputSelection,
    solc::{
        sources::{Source, SourceCompilationKind, Sources},
        Severity, SourceFile, StandardJsonCompilerInput,
    },
};
use foundry_compilers_core::error::{Result, SolcError, SolcIoError};
use output::sources::{VersionedSourceFile, VersionedSourceFiles};
use project::ProjectCompiler;
use semver::Version;
use solc::SolcSettings;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
};

/// Represents a project workspace and handles `solc` compiling of all contracts in that workspace.
#[derive(Clone, derive_more::Debug)]
pub struct Project<
    C: Compiler = MultiCompiler,
    T: ArtifactOutput<CompilerContract = C::CompilerContract> = ConfigurableArtifacts,
> {
    pub compiler: C,
    /// The layout of the project
    pub paths: ProjectPathsConfig<C::Language>,
    /// The compiler settings
    pub settings: C::Settings,
    /// Additional settings for cases when default compiler settings are not enough to cover all
    /// possible restrictions.
    pub additional_settings: BTreeMap<String, C::Settings>,
    /// Mapping from file path to requirements on settings to compile it.
    ///
    /// This file will only be included into compiler inputs with profiles which satisfy the
    /// restrictions.
    pub restrictions:
        BTreeMap<PathBuf, RestrictionsWithVersion<<C::Settings as CompilerSettings>::Restrictions>>,
    /// Whether caching is enabled
    pub cached: bool,
    /// Whether to output build information with each solc call.
    pub build_info: bool,
    /// Whether writing artifacts to disk is enabled
    pub no_artifacts: bool,
    /// Handles all artifacts related tasks, reading and writing from the artifact dir.
    pub artifacts: T,
    /// Errors/Warnings which match these error codes are not going to be logged
    pub ignored_error_codes: Vec<u64>,
    /// Errors/Warnings which match these file paths are not going to be logged
    pub ignored_file_paths: Vec<PathBuf>,
    /// The minimum severity level that is treated as a compiler error
    pub compiler_severity_filter: Severity,
    /// Maximum number of `solc` processes to run simultaneously.
    solc_jobs: usize,
    /// Offline mode, if set, network access (download solc) is disallowed
    pub offline: bool,
    /// Windows only config value to ensure the all paths use `/` instead of `\\`, same as `solc`
    ///
    /// This is a noop on other platforms
    pub slash_paths: bool,
    /// Optional sparse output filter used to optimize compilation.
    #[debug(skip)]
    pub sparse_output: Option<Box<dyn FileFilter>>,
}

impl Project {
    /// Convenience function to call `ProjectBuilder::default()`.
    ///
    /// # Examples
    ///
    /// Configure with [ConfigurableArtifacts] artifacts output and [MultiCompiler] compiler:
    /// ```no_run
    /// use foundry_compilers::Project;
    ///
    /// let config = Project::builder().build(Default::default())?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// To configure any a project with any `ArtifactOutput` use either:
    /// ```no_run
    /// use foundry_compilers::Project;
    ///
    /// let config = Project::builder().build(Default::default())?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// or use the builder directly:
    /// ```no_run
    /// use foundry_compilers::{multi::MultiCompiler, ConfigurableArtifacts, ProjectBuilder};
    ///
    /// let config = ProjectBuilder::<MultiCompiler>::default().build(Default::default())?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn builder() -> ProjectBuilder {
        ProjectBuilder::default()
    }
}

impl<T: ArtifactOutput<CompilerContract = C::CompilerContract>, C: Compiler> Project<C, T> {
    /// Returns the handler that takes care of processing all artifacts
    pub fn artifacts_handler(&self) -> &T {
        &self.artifacts
    }

    pub fn settings_profiles(&self) -> impl Iterator<Item = (&str, &C::Settings)> {
        std::iter::once(("default", &self.settings))
            .chain(self.additional_settings.iter().map(|(p, s)| (p.as_str(), s)))
    }
}

impl<C: Compiler, T: ArtifactOutput<CompilerContract = C::CompilerContract>> Project<C, T>
where
    C::Settings: Into<SolcSettings>,
{
    /// Returns standard-json-input to compile the target contract
    pub fn standard_json_input(&self, target: &Path) -> Result<StandardJsonCompilerInput> {
        trace!(?target, "Building standard-json-input");
        let graph = Graph::<C::ParsedSource>::resolve(&self.paths)?;
        let target_index = graph.files().get(target).ok_or_else(|| {
            SolcError::msg(format!("cannot resolve file at {:?}", target.display()))
        })?;

        let mut sources = Vec::new();
        let mut unique_paths = HashSet::new();
        let (path, source) = graph.node(*target_index).unpack();
        unique_paths.insert(path.clone());
        sources.push((path, source));
        sources.extend(
            graph
                .all_imported_nodes(*target_index)
                .map(|index| graph.node(index).unpack())
                .filter(|(p, _)| unique_paths.insert(p.to_path_buf())),
        );

        let root = self.root();
        let sources = sources
            .into_iter()
            .map(|(path, source)| (rebase_path(root, path), source.clone()))
            .collect();

        let mut settings = self.settings.clone().into();
        // strip the path to the project root from all remappings
        settings.remappings = self
            .paths
            .remappings
            .clone()
            .into_iter()
            .map(|r| r.into_relative(self.root()).to_relative_remapping())
            .collect::<Vec<_>>();

        let input = StandardJsonCompilerInput::new(sources, settings.settings);

        Ok(input)
    }
}

impl<T: ArtifactOutput<CompilerContract = C::CompilerContract>, C: Compiler> Project<C, T> {
    /// Returns the path to the artifacts directory
    pub fn artifacts_path(&self) -> &PathBuf {
        &self.paths.artifacts
    }

    /// Returns the path to the sources directory
    pub fn sources_path(&self) -> &PathBuf {
        &self.paths.sources
    }

    /// Returns the path to the cache file
    pub fn cache_path(&self) -> &PathBuf {
        &self.paths.cache
    }

    /// Returns the path to the `build-info` directory nested in the artifacts dir
    pub fn build_info_path(&self) -> &PathBuf {
        &self.paths.build_infos
    }

    /// Returns the root directory of the project
    pub fn root(&self) -> &PathBuf {
        &self.paths.root
    }

    /// Convenience function to read the cache file.
    /// See also [CompilerCache::read_joined()]
    pub fn read_cache_file(&self) -> Result<CompilerCache<C::Settings>> {
        CompilerCache::read_joined(&self.paths)
    }

    /// Sets the maximum number of parallel `solc` processes to run simultaneously.
    ///
    /// # Panics
    ///
    /// if `jobs == 0`
    pub fn set_solc_jobs(&mut self, jobs: usize) {
        assert!(jobs > 0);
        self.solc_jobs = jobs;
    }

    /// Returns all sources found under the project's configured sources path
    #[instrument(skip_all, fields(name = "sources"))]
    pub fn sources(&self) -> Result<Sources> {
        self.paths.read_sources()
    }

    /// Emit the cargo [`rerun-if-changed`](https://doc.rust-lang.org/cargo/reference/build-scripts.html#cargorerun-if-changedpath) instruction.
    ///
    /// This tells Cargo to re-run the build script if a file inside the project's sources directory
    /// has changed.
    ///
    /// Use this if you compile a project in a `build.rs` file.
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::{Project, ProjectPathsConfig};
    ///
    /// // Configure the project with all its paths, solc, cache etc.
    /// // where the root dir is the current Rust project.
    /// let paths = ProjectPathsConfig::hardhat(env!("CARGO_MANIFEST_DIR").as_ref())?;
    /// let project = Project::builder().paths(paths).build(Default::default())?;
    /// let output = project.compile()?;
    ///
    /// // Tell Cargo to rerun this build script that if a source file changes.
    /// project.rerun_if_sources_changed();
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn rerun_if_sources_changed(&self) {
        println!("cargo:rerun-if-changed={}", self.paths.sources.display())
    }

    pub fn compile(&self) -> Result<ProjectCompileOutput<C, T>> {
        project::ProjectCompiler::new(self)?.compile()
    }

    /// Convenience function to compile a single solidity file with the project's settings.
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::Project;
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let output = project.compile_file("example/Greeter.sol")?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn compile_file(&self, file: impl Into<PathBuf>) -> Result<ProjectCompileOutput<C, T>> {
        let file = file.into();
        let source = Source::read(&file)?;
        project::ProjectCompiler::with_sources(self, Sources::from([(file, source)]))?.compile()
    }

    /// Convenience function to compile a series of solidity files with the project's settings.
    /// Same as [`Self::compile()`] but with the given `files` as input.
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::Project;
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let output = project.compile_files(["examples/Foo.sol", "examples/Bar.sol"])?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn compile_files<P, I>(&self, files: I) -> Result<ProjectCompileOutput<C, T>>
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        let sources = Source::read_all(files)?;

        ProjectCompiler::with_sources(self, sources)?.compile()
    }

    /// Removes the project's artifacts and cache file
    ///
    /// If the cache file was the only file in the folder, this also removes the empty folder.
    ///
    /// # Examples
    /// ```
    /// use foundry_compilers::Project;
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let _ = project.compile()?;
    /// assert!(project.artifacts_path().exists());
    /// assert!(project.cache_path().exists());
    ///
    /// project.cleanup();
    /// assert!(!project.artifacts_path().exists());
    /// assert!(!project.cache_path().exists());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn cleanup(&self) -> std::result::Result<(), SolcIoError> {
        trace!("clean up project");
        if self.cache_path().exists() {
            std::fs::remove_file(self.cache_path())
                .map_err(|err| SolcIoError::new(err, self.cache_path()))?;
            if let Some(cache_folder) =
                self.cache_path().parent().filter(|cache_folder| self.root() != cache_folder)
            {
                // remove the cache folder if the cache file was the only file
                if cache_folder
                    .read_dir()
                    .map_err(|err| SolcIoError::new(err, cache_folder))?
                    .next()
                    .is_none()
                {
                    std::fs::remove_dir(cache_folder)
                        .map_err(|err| SolcIoError::new(err, cache_folder))?;
                }
            }
            trace!("removed cache file \"{}\"", self.cache_path().display());
        }

        // clean the artifacts dir
        if self.artifacts_path().exists() && self.root() != self.artifacts_path() {
            std::fs::remove_dir_all(self.artifacts_path())
                .map_err(|err| SolcIoError::new(err, self.artifacts_path().clone()))?;
            trace!("removed artifacts dir \"{}\"", self.artifacts_path().display());
        }

        // also clean the build-info dir, in case it's not nested in the artifacts dir
        if self.build_info_path().exists() && self.root() != self.build_info_path() {
            std::fs::remove_dir_all(self.build_info_path())
                .map_err(|err| SolcIoError::new(err, self.build_info_path().clone()))?;
            tracing::trace!("removed build-info dir \"{}\"", self.build_info_path().display());
        }

        Ok(())
    }

    /// Parses the sources in memory and collects all the contract names mapped to their file paths.
    fn collect_contract_names(&self) -> Result<HashMap<String, Vec<PathBuf>>>
    where
        T: Clone,
        C: Clone,
    {
        let graph = Graph::<C::ParsedSource>::resolve(&self.paths)?;
        let mut contracts: HashMap<String, Vec<PathBuf>> = HashMap::new();
        if !graph.is_empty() {
            for node in &graph.nodes {
                for contract_name in node.data.contract_names() {
                    contracts
                        .entry(contract_name.clone())
                        .or_default()
                        .push(node.path().to_path_buf());
                }
            }
        }
        Ok(contracts)
    }

    /// Finds the path of the contract with the given name.
    /// Throws error if multiple or no contracts with the same name are found.
    pub fn find_contract_path(&self, target_name: &str) -> Result<PathBuf>
    where
        T: Clone,
        C: Clone,
    {
        let mut contracts = self.collect_contract_names()?;

        if contracts.get(target_name).is_none_or(|paths| paths.is_empty()) {
            return Err(SolcError::msg(format!("No contract found with the name `{target_name}`")));
        }
        let mut paths = contracts.remove(target_name).unwrap();
        if paths.len() > 1 {
            return Err(SolcError::msg(format!(
                "Multiple contracts found with the name `{target_name}`"
            )));
        }

        Ok(paths.remove(0))
    }

    /// Invokes [CompilerSettings::update_output_selection] on the project's settings and all
    /// additional settings profiles.
    pub fn update_output_selection(&mut self, f: impl FnOnce(&mut OutputSelection) + Copy) {
        self.settings.update_output_selection(f);
        self.additional_settings.iter_mut().for_each(|(_, s)| {
            s.update_output_selection(f);
        });
    }
}

pub struct ProjectBuilder<
    C: Compiler = MultiCompiler,
    T: ArtifactOutput<CompilerContract = C::CompilerContract> = ConfigurableArtifacts,
> {
    /// The layout of the
    paths: Option<ProjectPathsConfig<C::Language>>,
    /// How solc invocation should be configured.
    settings: Option<C::Settings>,
    additional_settings: BTreeMap<String, C::Settings>,
    restrictions:
        BTreeMap<PathBuf, RestrictionsWithVersion<<C::Settings as CompilerSettings>::Restrictions>>,
    /// Whether caching is enabled, default is true.
    cached: bool,
    /// Whether to output build information with each solc call.
    build_info: bool,
    /// Whether writing artifacts to disk is enabled, default is true.
    no_artifacts: bool,
    /// Use offline mode
    offline: bool,
    /// Whether to slash paths of the `ProjectCompilerOutput`
    slash_paths: bool,
    /// handles all artifacts related tasks
    artifacts: T,
    /// Which error codes to ignore
    pub ignored_error_codes: Vec<u64>,
    /// Which file paths to ignore
    pub ignored_file_paths: Vec<PathBuf>,
    /// The minimum severity level that is treated as a compiler error
    compiler_severity_filter: Severity,
    solc_jobs: Option<usize>,
    /// Optional sparse output filter used to optimize compilation.
    sparse_output: Option<Box<dyn FileFilter>>,
}

impl<C: Compiler, T: ArtifactOutput<CompilerContract = C::CompilerContract>> ProjectBuilder<C, T> {
    /// Create a new builder with the given artifacts handler
    pub fn new(artifacts: T) -> Self {
        Self {
            paths: None,
            cached: true,
            build_info: false,
            no_artifacts: false,
            offline: false,
            slash_paths: true,
            artifacts,
            ignored_error_codes: Vec::new(),
            ignored_file_paths: Vec::new(),
            compiler_severity_filter: Severity::Error,
            solc_jobs: None,
            settings: None,
            sparse_output: None,
            additional_settings: BTreeMap::new(),
            restrictions: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn paths(mut self, paths: ProjectPathsConfig<C::Language>) -> Self {
        self.paths = Some(paths);
        self
    }

    #[must_use]
    pub fn settings(mut self, settings: C::Settings) -> Self {
        self.settings = Some(settings);
        self
    }

    #[must_use]
    pub fn ignore_error_code(mut self, code: u64) -> Self {
        self.ignored_error_codes.push(code);
        self
    }

    #[must_use]
    pub fn ignore_error_codes(mut self, codes: impl IntoIterator<Item = u64>) -> Self {
        for code in codes {
            self = self.ignore_error_code(code);
        }
        self
    }

    pub fn ignore_paths(mut self, paths: Vec<PathBuf>) -> Self {
        self.ignored_file_paths = paths;
        self
    }

    #[must_use]
    pub fn set_compiler_severity_filter(mut self, compiler_severity_filter: Severity) -> Self {
        self.compiler_severity_filter = compiler_severity_filter;
        self
    }

    /// Disables cached builds
    #[must_use]
    pub fn ephemeral(self) -> Self {
        self.set_cached(false)
    }

    /// Sets the cache status
    #[must_use]
    pub fn set_cached(mut self, cached: bool) -> Self {
        self.cached = cached;
        self
    }

    /// Sets the build info value
    #[must_use]
    pub fn set_build_info(mut self, build_info: bool) -> Self {
        self.build_info = build_info;
        self
    }

    /// Activates offline mode
    ///
    /// Prevents network possible access to download/check solc installs
    #[must_use]
    pub fn offline(self) -> Self {
        self.set_offline(true)
    }

    /// Sets the offline status
    #[must_use]
    pub fn set_offline(mut self, offline: bool) -> Self {
        self.offline = offline;
        self
    }

    /// Sets whether to slash all paths on windows
    ///
    /// If set to `true` all `\\` separators are replaced with `/`, same as solc
    #[must_use]
    pub fn set_slashed_paths(mut self, slashed_paths: bool) -> Self {
        self.slash_paths = slashed_paths;
        self
    }

    /// Disables writing artifacts to disk
    #[must_use]
    pub fn no_artifacts(self) -> Self {
        self.set_no_artifacts(true)
    }

    /// Sets the no artifacts status
    #[must_use]
    pub fn set_no_artifacts(mut self, artifacts: bool) -> Self {
        self.no_artifacts = artifacts;
        self
    }

    /// Sets the maximum number of parallel `solc` processes to run simultaneously.
    ///
    /// # Panics
    ///
    /// `jobs` must be at least 1
    #[must_use]
    pub fn solc_jobs(mut self, jobs: usize) -> Self {
        assert!(jobs > 0);
        self.solc_jobs = Some(jobs);
        self
    }

    /// Sets the number of parallel `solc` processes to `1`, no parallelization
    #[must_use]
    pub fn single_solc_jobs(self) -> Self {
        self.solc_jobs(1)
    }

    #[must_use]
    pub fn sparse_output<F>(mut self, filter: F) -> Self
    where
        F: FileFilter + 'static,
    {
        self.sparse_output = Some(Box::new(filter));
        self
    }

    #[must_use]
    pub fn additional_settings(mut self, additional: BTreeMap<String, C::Settings>) -> Self {
        self.additional_settings = additional;
        self
    }

    #[must_use]
    pub fn restrictions(
        mut self,
        restrictions: BTreeMap<
            PathBuf,
            RestrictionsWithVersion<<C::Settings as CompilerSettings>::Restrictions>,
        >,
    ) -> Self {
        self.restrictions = restrictions;
        self
    }

    /// Set arbitrary `ArtifactOutputHandler`
    pub fn artifacts<A: ArtifactOutput<CompilerContract = C::CompilerContract>>(
        self,
        artifacts: A,
    ) -> ProjectBuilder<C, A> {
        let Self {
            paths,
            cached,
            no_artifacts,
            ignored_error_codes,
            compiler_severity_filter,
            solc_jobs,
            offline,
            build_info,
            slash_paths,
            ignored_file_paths,
            settings,
            sparse_output,
            additional_settings,
            restrictions,
            ..
        } = self;
        ProjectBuilder {
            paths,
            cached,
            no_artifacts,
            additional_settings,
            restrictions,
            offline,
            slash_paths,
            artifacts,
            ignored_error_codes,
            ignored_file_paths,
            compiler_severity_filter,
            solc_jobs,
            build_info,
            settings,
            sparse_output,
        }
    }

    pub fn build(self, compiler: C) -> Result<Project<C, T>> {
        let Self {
            paths,
            cached,
            no_artifacts,
            artifacts,
            ignored_error_codes,
            ignored_file_paths,
            compiler_severity_filter,
            solc_jobs,
            offline,
            build_info,
            slash_paths,
            settings,
            sparse_output,
            additional_settings,
            restrictions,
        } = self;

        let mut paths = paths.map(Ok).unwrap_or_else(ProjectPathsConfig::current_hardhat)?;

        if slash_paths {
            // ensures we always use `/` paths
            paths.slash_paths();
        }

        Ok(Project {
            compiler,
            paths,
            cached,
            build_info,
            no_artifacts,
            artifacts,
            ignored_error_codes,
            ignored_file_paths,
            compiler_severity_filter,
            solc_jobs: solc_jobs
                .or_else(|| std::thread::available_parallelism().ok().map(|n| n.get()))
                .unwrap_or(1),
            offline,
            slash_paths,
            settings: settings.unwrap_or_default(),
            sparse_output,
            additional_settings,
            restrictions,
        })
    }
}

impl<C: Compiler, T: ArtifactOutput<CompilerContract = C::CompilerContract> + Default> Default
    for ProjectBuilder<C, T>
{
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: ArtifactOutput<CompilerContract = C::CompilerContract>, C: Compiler> ArtifactOutput
    for Project<C, T>
{
    type Artifact = T::Artifact;
    type CompilerContract = C::CompilerContract;

    fn on_output<CP>(
        &self,
        contracts: &VersionedContracts<C::CompilerContract>,
        sources: &VersionedSourceFiles,
        layout: &ProjectPathsConfig<CP>,
        ctx: OutputContext<'_>,
        primary_profiles: &HashMap<PathBuf, &str>,
    ) -> Result<Artifacts<Self::Artifact>> {
        self.artifacts_handler().on_output(contracts, sources, layout, ctx, primary_profiles)
    }

    fn handle_artifacts(
        &self,
        contracts: &VersionedContracts<C::CompilerContract>,
        artifacts: &Artifacts<Self::Artifact>,
    ) -> Result<()> {
        self.artifacts_handler().handle_artifacts(contracts, artifacts)
    }

    fn output_file_name(
        name: &str,
        version: &Version,
        profile: &str,
        with_version: bool,
        with_profile: bool,
    ) -> PathBuf {
        T::output_file_name(name, version, profile, with_version, with_profile)
    }

    fn output_file(
        contract_file: &Path,
        name: &str,
        version: &Version,
        profile: &str,
        with_version: bool,
        with_profile: bool,
    ) -> PathBuf {
        T::output_file(contract_file, name, version, profile, with_version, with_profile)
    }

    fn contract_name(file: &Path) -> Option<String> {
        T::contract_name(file)
    }

    fn read_cached_artifact(path: &Path) -> Result<Self::Artifact> {
        T::read_cached_artifact(path)
    }

    fn read_cached_artifacts<P, I>(files: I) -> Result<BTreeMap<PathBuf, Self::Artifact>>
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        T::read_cached_artifacts(files)
    }

    fn contract_to_artifact(
        &self,
        file: &Path,
        name: &str,
        contract: C::CompilerContract,
        source_file: Option<&SourceFile>,
    ) -> Self::Artifact {
        self.artifacts_handler().contract_to_artifact(file, name, contract, source_file)
    }

    fn output_to_artifacts<CP>(
        &self,
        contracts: &VersionedContracts<C::CompilerContract>,
        sources: &VersionedSourceFiles,
        ctx: OutputContext<'_>,
        layout: &ProjectPathsConfig<CP>,
        primary_profiles: &HashMap<PathBuf, &str>,
    ) -> Artifacts<Self::Artifact> {
        self.artifacts_handler().output_to_artifacts(
            contracts,
            sources,
            ctx,
            layout,
            primary_profiles,
        )
    }

    fn standalone_source_file_to_artifact(
        &self,
        path: &Path,
        file: &VersionedSourceFile,
    ) -> Option<Self::Artifact> {
        self.artifacts_handler().standalone_source_file_to_artifact(path, file)
    }

    fn is_dirty(&self, artifact_file: &ArtifactFile<Self::Artifact>) -> Result<bool> {
        self.artifacts_handler().is_dirty(artifact_file)
    }

    fn handle_cached_artifacts(&self, artifacts: &Artifacts<Self::Artifact>) -> Result<()> {
        self.artifacts_handler().handle_cached_artifacts(artifacts)
    }
}

// Rebases the given path to the base directory lexically.
//
// For instance, given the base `/home/user/project` and the path `/home/user/project/src/A.sol`,
// this function returns `src/A.sol`.
//
// This function transforms a path into a form that is relative to the base directory. The returned
// path starts either with a normal component (e.g., `src`) or a parent directory component (i.e.,
// `..`). It also converts the path into a UTF-8 string and replaces all separators with forward
// slashes (`/`), if they're not.
//
// The rebasing process can be conceptualized as follows:
//
// 1. Remove the leading components from the path that match those in the base.
// 2. Prepend `..` components to the path, matching the number of remaining components in the base.
//
// # Examples
//
// `rebase_path("/home/user/project", "/home/user/project/src/A.sol")` returns `src/A.sol`. The
// common part, `/home/user/project`, is removed from the path.
//
// `rebase_path("/home/user/project", "/home/user/A.sol")` returns `../A.sol`. First, the common
// part, `/home/user`, is removed, leaving `A.sol`. Next, as `project` remains in the base, `..` is
// prepended to the path.
//
// On Windows, paths like `a\b\c` are converted to `a/b/c`.
//
// For more examples, see the test.
fn rebase_path(base: &Path, path: &Path) -> PathBuf {
    use path_slash::PathExt;

    let mut base_components = base.components();
    let mut path_components = path.components();

    let mut new_path = PathBuf::new();

    while let Some(path_component) = path_components.next() {
        let base_component = base_components.next();

        if Some(path_component) != base_component {
            if base_component.is_some() {
                new_path.extend(std::iter::repeat_n(
                    std::path::Component::ParentDir,
                    base_components.count() + 1,
                ));
            }

            new_path.push(path_component);
            new_path.extend(path_components);

            break;
        }
    }

    new_path.to_slash_lossy().into_owned().into()
}

#[cfg(test)]
#[cfg(feature = "svm-solc")]
mod tests {
    use foundry_compilers_artifacts::Remapping;
    use foundry_compilers_core::utils::{self, mkdir_or_touch, tempdir};

    use super::*;

    #[test]
    #[cfg_attr(windows, ignore = "<0.7 solc is flaky")]
    fn test_build_all_versions() {
        let paths = ProjectPathsConfig::builder()
            .root("../../test-data/test-contract-versions")
            .sources("../../test-data/test-contract-versions")
            .build()
            .unwrap();
        let project = Project::builder()
            .paths(paths)
            .no_artifacts()
            .ephemeral()
            .build(Default::default())
            .unwrap();
        let contracts = project.compile().unwrap().succeeded().into_output().contracts;
        // Contracts A to F
        assert_eq!(contracts.contracts().count(), 3);
    }

    #[test]
    fn test_build_many_libs() {
        let root = utils::canonicalize("../../test-data/test-contract-libs").unwrap();

        let paths = ProjectPathsConfig::builder()
            .root(&root)
            .sources(root.join("src"))
            .lib(root.join("lib1"))
            .lib(root.join("lib2"))
            .remappings(
                Remapping::find_many(&root.join("lib1"))
                    .into_iter()
                    .chain(Remapping::find_many(&root.join("lib2"))),
            )
            .build()
            .unwrap();
        let project = Project::builder()
            .paths(paths)
            .no_artifacts()
            .ephemeral()
            .no_artifacts()
            .build(Default::default())
            .unwrap();
        let contracts = project.compile().unwrap().succeeded().into_output().contracts;
        assert_eq!(contracts.contracts().count(), 3);
    }

    #[test]
    fn test_build_remappings() {
        let root = utils::canonicalize("../../test-data/test-contract-remappings").unwrap();
        let paths = ProjectPathsConfig::builder()
            .root(&root)
            .sources(root.join("src"))
            .lib(root.join("lib"))
            .remappings(Remapping::find_many(&root.join("lib")))
            .build()
            .unwrap();
        let project = Project::builder()
            .no_artifacts()
            .paths(paths)
            .ephemeral()
            .build(Default::default())
            .unwrap();
        let contracts = project.compile().unwrap().succeeded().into_output().contracts;
        assert_eq!(contracts.contracts().count(), 2);
    }

    #[test]
    fn can_rebase_path() {
        let rebase_path = |a: &str, b: &str| rebase_path(a.as_ref(), b.as_ref());

        assert_eq!(rebase_path("a/b", "a/b/c"), PathBuf::from("c"));
        assert_eq!(rebase_path("a/b", "a/c"), PathBuf::from("../c"));
        assert_eq!(rebase_path("a/b", "c"), PathBuf::from("../../c"));

        assert_eq!(
            rebase_path("/home/user/project", "/home/user/project/A.sol"),
            PathBuf::from("A.sol")
        );
        assert_eq!(
            rebase_path("/home/user/project", "/home/user/project/src/A.sol"),
            PathBuf::from("src/A.sol")
        );
        assert_eq!(
            rebase_path("/home/user/project", "/home/user/project/lib/forge-std/src/Test.sol"),
            PathBuf::from("lib/forge-std/src/Test.sol")
        );
        assert_eq!(
            rebase_path("/home/user/project", "/home/user/A.sol"),
            PathBuf::from("../A.sol")
        );
        assert_eq!(rebase_path("/home/user/project", "/home/A.sol"), PathBuf::from("../../A.sol"));
        assert_eq!(rebase_path("/home/user/project", "/A.sol"), PathBuf::from("../../../A.sol"));
        assert_eq!(
            rebase_path("/home/user/project", "/tmp/A.sol"),
            PathBuf::from("../../../tmp/A.sol")
        );

        assert_eq!(
            rebase_path("/Users/ah/temp/verif", "/Users/ah/temp/remapped/Child.sol"),
            PathBuf::from("../remapped/Child.sol")
        );
        assert_eq!(
            rebase_path("/Users/ah/temp/verif", "/Users/ah/temp/verif/../remapped/Parent.sol"),
            PathBuf::from("../remapped/Parent.sol")
        );
    }

    #[test]
    fn can_resolve_oz_remappings() {
        let tmp_dir = tempdir("node_modules").unwrap();
        let tmp_dir_node_modules = tmp_dir.path().join("node_modules");
        let paths = [
            "node_modules/@openzeppelin/contracts/interfaces/IERC1155.sol",
            "node_modules/@openzeppelin/contracts/finance/VestingWallet.sol",
            "node_modules/@openzeppelin/contracts/proxy/Proxy.sol",
            "node_modules/@openzeppelin/contracts/token/ERC20/IERC20.sol",
        ];
        mkdir_or_touch(tmp_dir.path(), &paths[..]);
        let remappings = Remapping::find_many(&tmp_dir_node_modules);
        let mut paths = ProjectPathsConfig::<()>::hardhat(tmp_dir.path()).unwrap();
        paths.remappings = remappings;

        let resolved = paths
            .resolve_library_import(
                tmp_dir.path(),
                Path::new("@openzeppelin/contracts/token/ERC20/IERC20.sol"),
            )
            .unwrap();
        assert!(resolved.exists());

        // adjust remappings
        paths.remappings[0].name = "@openzeppelin/".to_string();

        let resolved = paths
            .resolve_library_import(
                tmp_dir.path(),
                Path::new("@openzeppelin/contracts/token/ERC20/IERC20.sol"),
            )
            .unwrap();
        assert!(resolved.exists());
    }
}
