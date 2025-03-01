//! Utilities for mocking project workspaces.

use crate::{
    cache::CompilerCache,
    compilers::{
        multi::{MultiCompiler, MultiCompilerSettings},
        Compiler,
    },
    config::ProjectPathsConfigBuilder,
    solc::SolcSettings,
    Artifact, ArtifactOutput, Artifacts, ConfigurableArtifacts, HardhatArtifacts, PathStyle,
    Project, ProjectBuilder, ProjectCompileOutput, ProjectPathsConfig,
};
use foundry_compilers_artifacts::{ConfigurableContractArtifact, Remapping, Settings};
use foundry_compilers_core::{
    error::{Result, SolcError, SolcIoError},
    utils::{self, tempdir},
};
use fs_extra::{dir, file};
use mock::{MockProjectGenerator, MockProjectSettings};
use std::{
    fmt,
    path::{Path, PathBuf},
    process,
    process::Command,
};
use tempfile::TempDir;

pub mod mock;

/// A [`Project`] wrapper that lives in a new temporary directory
///
/// Once `TempProject` is dropped, the temp dir is automatically removed, see [`TempDir::drop()`]
pub struct TempProject<
    C: Compiler = MultiCompiler,
    T: ArtifactOutput<CompilerContract = C::CompilerContract> = ConfigurableArtifacts,
> {
    /// temporary workspace root
    _root: TempDir,
    /// actual project workspace with the `root` tempdir as its root
    inner: Project<C, T>,
}

impl<
        T: ArtifactOutput<CompilerContract = <MultiCompiler as Compiler>::CompilerContract> + Default,
    > TempProject<MultiCompiler, T>
{
    /// Creates a new temp project using the provided paths and artifacts handler.
    /// sets the project root to a temp dir
    #[cfg(feature = "svm-solc")]
    pub fn with_artifacts(paths: ProjectPathsConfigBuilder, artifacts: T) -> Result<Self> {
        Self::prefixed_with_artifacts("temp-project", paths, artifacts)
    }

    /// Overwrites the settings to pass to `solc`
    pub fn with_solc_settings(mut self, settings: impl Into<Settings>) -> Self {
        self.inner.settings.solc = SolcSettings { settings: settings.into(), ..Default::default() };
        self
    }

    /// Explicitly sets the solc version for the project
    #[cfg(feature = "svm-solc")]
    pub fn set_solc(&mut self, solc: &str) -> &mut Self {
        use crate::solc::{Solc, SolcCompiler};

        self.inner.compiler.solc =
            Some(SolcCompiler::Specific(Solc::find_or_install(&solc.parse().unwrap()).unwrap()));

        self
    }
}

impl<
        T: ArtifactOutput<CompilerContract = <MultiCompiler as Compiler>::CompilerContract> + Default,
    > TempProject<MultiCompiler, T>
{
    /// Creates a new temp project for the given `PathStyle`
    #[cfg(feature = "svm-solc")]
    pub fn with_style(prefix: &str, style: PathStyle) -> Result<Self> {
        let tmp_dir = tempdir(prefix)?;
        let paths = style.paths(tmp_dir.path())?;
        let inner =
            Project::builder().artifacts(T::default()).paths(paths).build(Default::default())?;
        Ok(Self::create_new(tmp_dir, inner)?)
    }
}

impl<
        T: ArtifactOutput<CompilerContract = <MultiCompiler as Compiler>::CompilerContract> + Default,
    > fmt::Debug for TempProject<MultiCompiler, T>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TempProject").field("paths", &self.inner.paths).finish()
    }
}

pub(crate) fn create_contract_file(path: &Path, content: impl AsRef<str>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| SolcIoError::new(err, parent.to_path_buf()))?;
    }
    std::fs::write(path, content.as_ref()).map_err(|err| SolcIoError::new(err, path))?;
    Ok(())
}

fn contract_file_name(name: &str) -> String {
    let name = name.trim();
    if name.ends_with(".sol") || name.ends_with(".vy") || name.ends_with(".vyi") {
        name.to_string()
    } else {
        format!("{name}.sol")
    }
}

#[cfg(feature = "svm-solc")]
impl TempProject<MultiCompiler, HardhatArtifacts> {
    /// Creates an empty new hardhat style workspace in a new temporary dir
    pub fn hardhat() -> Result<Self> {
        let tmp_dir = tempdir("tmp_hh")?;

        let paths = ProjectPathsConfig::hardhat(tmp_dir.path())?;

        let inner = Project::builder()
            .artifacts(HardhatArtifacts::default())
            .paths(paths)
            .build(Default::default())?;
        Ok(Self::create_new(tmp_dir, inner)?)
    }
}

impl<
        C: Compiler + Default,
        T: ArtifactOutput<CompilerContract = C::CompilerContract> + Default,
    > TempProject<C, T>
{
    /// Makes sure all resources are created
    pub fn create_new(
        root: TempDir,
        inner: Project<C, T>,
    ) -> std::result::Result<Self, SolcIoError> {
        let mut project = Self { _root: root, inner };
        project.inner.paths.create_all()?;
        // ignore license warnings
        project.inner.ignored_error_codes.push(1878);
        Ok(project)
    }

    /// Creates a new temp project using the provided paths and setting the project root to a temp
    /// dir
    #[cfg(feature = "svm-solc")]
    pub fn new(paths: ProjectPathsConfigBuilder) -> Result<Self> {
        Self::prefixed("temp-project", paths)
    }

    /// Creates a new temp project inside a tempdir with a prefixed directory
    #[cfg(feature = "svm-solc")]
    pub fn prefixed(prefix: &str, paths: ProjectPathsConfigBuilder) -> Result<Self> {
        Self::prefixed_with_artifacts(prefix, paths, T::default())
    }

    /// Creates a new temp project inside a tempdir with a prefixed directory and the given
    /// artifacts handler
    #[cfg(feature = "svm-solc")]
    pub fn prefixed_with_artifacts(
        prefix: &str,
        paths: ProjectPathsConfigBuilder,
        artifacts: T,
    ) -> Result<Self> {
        let tmp_dir = tempdir(prefix)?;
        let paths = paths.build_with_root(tmp_dir.path());
        let inner = ProjectBuilder::<C, T>::new(Default::default())
            .artifacts(artifacts)
            .paths(paths)
            .build(Default::default())?;
        Ok(Self::create_new(tmp_dir, inner)?)
    }

    /// Creates an empty new dapptools style workspace in a new temporary dir
    pub fn dapptools() -> Result<Self> {
        let tmp_dir = tempdir("tmp_dapp")?;
        let paths = ProjectPathsConfig::dapptools(tmp_dir.path())?;

        let inner = ProjectBuilder::<C, T>::new(Default::default())
            .paths(paths)
            .build(Default::default())?;
        Ok(Self::create_new(tmp_dir, inner)?)
    }

    /// Creates an initialized dapptools style workspace in a new temporary dir
    pub fn dapptools_init() -> Result<Self> {
        let mut project = Self::dapptools()?;
        let orig_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/dapp-sample");
        copy_dir(&orig_root, project.root())?;
        project.project_mut().paths.remappings = Remapping::find_many(project.root());
        project.project_mut().paths.remappings.iter_mut().for_each(|r| r.slash_path());

        Ok(project)
    }

    pub fn project(&self) -> &Project<C, T> {
        &self.inner
    }

    pub fn project_mut(&mut self) -> &mut Project<C, T> {
        &mut self.inner
    }

    /// The configured paths of the project
    pub fn paths(&self) -> &ProjectPathsConfig<C::Language> {
        &self.project().paths
    }

    /// The configured paths of the project
    pub fn paths_mut(&mut self) -> &mut ProjectPathsConfig<C::Language> {
        &mut self.project_mut().paths
    }

    /// Copies a single file into the projects source
    pub fn copy_source(&self, source: &Path) -> Result<()> {
        copy_file(source, &self.paths().sources)
    }

    pub fn copy_sources<I, S>(&self, sources: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<Path>,
    {
        for path in sources {
            self.copy_source(path.as_ref())?;
        }
        Ok(())
    }

    fn get_lib(&self) -> Result<PathBuf> {
        self.paths()
            .libraries
            .first()
            .cloned()
            .ok_or_else(|| SolcError::msg("No libraries folders configured"))
    }

    /// Copies a single file into the project's main library directory
    pub fn copy_lib(&self, lib: &Path) -> Result<()> {
        let lib_dir = self.get_lib()?;
        copy_file(lib, &lib_dir)
    }

    /// Copy a series of files into the main library dir
    pub fn copy_libs<I, S>(&self, libs: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<Path>,
    {
        for path in libs {
            self.copy_lib(path.as_ref())?;
        }
        Ok(())
    }

    /// Adds a new library file
    pub fn add_lib(&self, name: &str, content: impl AsRef<str>) -> Result<PathBuf> {
        let name = contract_file_name(name);
        let lib_dir = self.get_lib()?;
        let lib = lib_dir.join(name);
        create_contract_file(&lib, content)?;
        Ok(lib)
    }

    /// Adds a basic lib contract `contract <name> {}` as a new file
    pub fn add_basic_lib(&self, name: &str, version: &str) -> Result<PathBuf> {
        let name = name.strip_suffix(".sol").unwrap_or(name);
        self.add_lib(
            name,
            format!(
                r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity {version};
contract {name} {{}}
            "#,
            ),
        )
    }

    /// Adds a new test file inside the project's test dir
    pub fn add_test(&self, name: &str, content: impl AsRef<str>) -> Result<PathBuf> {
        let name = contract_file_name(name);
        let tests = self.paths().tests.join(name);
        create_contract_file(&tests, content)?;
        Ok(tests)
    }

    /// Adds a new script file inside the project's script dir
    pub fn add_script(&self, name: &str, content: impl AsRef<str>) -> Result<PathBuf> {
        let name = contract_file_name(name);
        let script = self.paths().scripts.join(name);
        create_contract_file(&script, content)?;
        Ok(script)
    }

    /// Adds a new source file inside the project's source dir
    pub fn add_source(&self, name: &str, content: impl AsRef<str>) -> Result<PathBuf> {
        let name = contract_file_name(name);
        let source = self.paths().sources.join(name);
        create_contract_file(&source, content)?;
        Ok(source)
    }

    /// Adds a basic source contract `contract <name> {}` as a new file
    pub fn add_basic_source(&self, name: &str, version: &str) -> Result<PathBuf> {
        let name = name.strip_suffix(".sol").unwrap_or(name);
        self.add_source(
            name,
            format!(
                r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity {version};
contract {name} {{}}
            "#,
            ),
        )
    }

    /// Adds a solidity contract in the project's root dir.
    /// This will also create all intermediary dirs.
    pub fn add_contract(&self, name: &str, content: impl AsRef<str>) -> Result<PathBuf> {
        let name = contract_file_name(name);
        let source = self.root().join(name);
        create_contract_file(&source, content)?;
        Ok(source)
    }

    /// Returns the path to the artifacts directory
    pub fn artifacts_path(&self) -> &PathBuf {
        &self.paths().artifacts
    }

    /// Returns the path to the sources directory
    pub fn sources_path(&self) -> &PathBuf {
        &self.paths().sources
    }

    /// Returns the path to the cache file
    pub fn cache_path(&self) -> &PathBuf {
        &self.paths().cache
    }

    /// The root path of the temporary workspace
    pub fn root(&self) -> &Path {
        self.project().paths.root.as_path()
    }

    pub fn compile(&self) -> Result<ProjectCompileOutput<C, T>> {
        self.project().compile()
    }

    /// Returns a snapshot of all cached artifacts
    pub fn artifacts_snapshot(&self) -> Result<ArtifactsSnapshot<T::Artifact, C::Settings>> {
        let cache = self.project().read_cache_file()?;
        let artifacts = cache.read_artifacts::<T::Artifact>()?;
        Ok(ArtifactsSnapshot { cache, artifacts })
    }

    /// Populate the project with mock files
    pub fn mock(&self, gen: &MockProjectGenerator, version: &str) -> Result<()> {
        gen.write_to(self.paths(), version)
    }

    /// Compiles the project and ensures that the output does not contain errors
    pub fn ensure_no_errors(&self) -> Result<&Self> {
        let compiled = self.compile().unwrap();
        if compiled.has_compiler_errors() {
            bail!("Compiled with errors {}", compiled)
        }
        Ok(self)
    }

    /// Compiles the project and ensures that the output is __unchanged__
    pub fn ensure_unchanged(&self) -> Result<&Self> {
        let compiled = self.compile().unwrap();
        if !compiled.is_unchanged() {
            bail!("Compiled with detected changes {}", compiled)
        }
        Ok(self)
    }

    /// Compiles the project and ensures that the output has __changed__
    pub fn ensure_changed(&self) -> Result<&Self> {
        let compiled = self.compile().unwrap();
        if compiled.is_unchanged() {
            bail!("Compiled without detecting changes {}", compiled)
        }
        Ok(self)
    }

    /// Compiles the project and ensures that the output does not contain errors and no changes
    /// exists on recompiled.
    ///
    /// This is a convenience function for `ensure_no_errors` + `ensure_unchanged`.
    pub fn ensure_no_errors_recompile_unchanged(&self) -> Result<&Self> {
        self.ensure_no_errors()?.ensure_unchanged()
    }

    /// Compiles the project and asserts that the output does not contain errors and no changes
    /// exists on recompiled.
    ///
    /// This is a convenience function for `assert_no_errors` + `assert_unchanged`.
    #[track_caller]
    pub fn assert_no_errors_recompile_unchanged(&self) -> &Self {
        self.assert_no_errors().assert_unchanged()
    }

    /// Compiles the project and asserts that the output does not contain errors
    pub fn assert_no_errors(&self) -> &Self {
        let compiled = self.compile().unwrap();
        compiled.assert_success();
        self
    }

    /// Compiles the project and asserts that the output is unchanged
    #[track_caller]
    pub fn assert_unchanged(&self) -> &Self {
        let compiled = self.compile().unwrap();
        assert!(compiled.is_unchanged());
        self
    }

    /// Compiles the project and asserts that the output is _changed_
    pub fn assert_changed(&self) -> &Self {
        let compiled = self.compile().unwrap();
        assert!(!compiled.is_unchanged());
        self
    }

    /// Returns a list of all source files in the project's `src` directory
    pub fn list_source_files(&self) -> Vec<PathBuf> {
        utils::sol_source_files(self.project().sources_path())
    }
}

#[cfg(feature = "svm-solc")]
impl TempProject {
    pub fn dapptools_with_ignore_paths(paths_to_ignore: Vec<PathBuf>) -> Result<Self> {
        let tmp_dir = tempdir("tmp_dapp")?;
        let paths = ProjectPathsConfig::dapptools(tmp_dir.path())?;

        let inner = Project::builder()
            .paths(paths)
            .ignore_paths(paths_to_ignore)
            .build(Default::default())?;
        Ok(Self::create_new(tmp_dir, inner)?)
    }

    /// Clones the given repo into a temp dir, initializes it recursively and configures it.
    pub fn checkout(repo: &str) -> Result<Self> {
        let tmp_dir = tempdir("tmp_checkout")?;
        clone_remote(&format!("https://github.com/{repo}"), tmp_dir.path())
            .map_err(|err| SolcIoError::new(err, tmp_dir.path()))?;
        let paths = ProjectPathsConfig::dapptools(tmp_dir.path())?;

        let inner = Project::builder().paths(paths).build(Default::default())?;
        Ok(Self::create_new(tmp_dir, inner)?)
    }

    /// Create a new temporary project and populate it with mock files.
    pub fn mocked(settings: &MockProjectSettings, version: &str) -> Result<Self> {
        let mut tmp = Self::dapptools()?;
        let gen = MockProjectGenerator::new(settings);
        tmp.mock(&gen, version)?;
        let remappings = gen.remappings_at(tmp.root());
        tmp.paths_mut().remappings.extend(remappings);
        Ok(tmp)
    }

    /// Create a new temporary project and populate it with a random layout.
    pub fn mocked_random(version: &str) -> Result<Self> {
        Self::mocked(&MockProjectSettings::random(), version)
    }
}

impl<
        T: ArtifactOutput<CompilerContract = <MultiCompiler as Compiler>::CompilerContract> + Default,
    > AsRef<Project<MultiCompiler, T>> for TempProject<MultiCompiler, T>
{
    fn as_ref(&self) -> &Project<MultiCompiler, T> {
        self.project()
    }
}

/// The cache file and all the artifacts it references
#[derive(Clone, Debug)]
pub struct ArtifactsSnapshot<T, S> {
    pub cache: CompilerCache<S>,
    pub artifacts: Artifacts<T>,
}

impl ArtifactsSnapshot<ConfigurableContractArtifact, MultiCompilerSettings> {
    /// Ensures that all artifacts have abi, bytecode, deployedbytecode
    pub fn assert_artifacts_essentials_present(&self) {
        for artifact in self.artifacts.artifact_files() {
            let c = artifact.artifact.clone().into_compact_contract();
            assert!(c.abi.is_some());
            assert!(c.bin.is_some());
            assert!(c.bin_runtime.is_some());
        }
    }
}

/// commonly used options for copying entire folders
fn dir_copy_options() -> dir::CopyOptions {
    dir::CopyOptions {
        overwrite: true,
        skip_exist: false,
        buffer_size: 64000, //64kb
        copy_inside: true,
        content_only: true,
        depth: 0,
    }
}

/// commonly used options for copying files
fn file_copy_options() -> file::CopyOptions {
    file::CopyOptions {
        overwrite: true,
        skip_exist: false,
        buffer_size: 64000, //64kb
    }
}

/// Copies a single file into the given dir
pub fn copy_file(source: &Path, target_dir: &Path) -> Result<()> {
    let target = target_dir.join(
        source
            .file_name()
            .ok_or_else(|| SolcError::msg(format!("No file name for {}", source.display())))?,
    );
    fs_extra::file::copy(source, target, &file_copy_options())?;
    Ok(())
}

/// Copies all content of the source dir into the target dir
pub fn copy_dir(source: &Path, target_dir: &Path) -> Result<()> {
    fs_extra::dir::copy(source, target_dir, &dir_copy_options())?;
    Ok(())
}

/// Clones a remote repository into the specified directory.
pub fn clone_remote(repo_url: &str, target_dir: &Path) -> std::io::Result<process::Output> {
    Command::new("git")
        .args(["clone", "--depth", "1", "--recursive", repo_url])
        .arg(target_dir)
        .output()
}

#[cfg(test)]
#[cfg(feature = "svm-solc")]
mod tests {
    use super::*;

    #[test]
    fn can_mock_project() {
        let _prj = TempProject::mocked(&Default::default(), "^0.8.11").unwrap();
        let _prj = TempProject::mocked_random("^0.8.11").unwrap();
    }
}
