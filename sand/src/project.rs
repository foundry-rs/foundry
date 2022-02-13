//! Represents a cairo/starknet workspace.

use crate::{cmd::StarknetCompile, config::ProjectPathsConfig, error::Result};
use starknet::core::types::ContractCode;
use std::path::PathBuf;

#[derive(Debug)]
pub struct Project {
    paths: ProjectPathsConfig,
    compiler: StarknetCompile,
}

impl Project {
    /// Returns the path to the artifacts directory
    pub fn artifacts_path(&self) -> &PathBuf {
        &self.paths.artifacts
    }

    /// Returns the path to the sources directory
    pub fn sources_path(&self) -> &PathBuf {
        &self.paths.sources
    }

    /// Returns the root directory of the project
    pub fn root(&self) -> &PathBuf {
        &self.paths.root
    }

    /// Convenience function to call `ProjectBuilder::default()`
    /// ```rust
    /// use foundry_sand::Project;
    /// let project = Project::builder().build().unwrap();
    /// ```
    pub fn builder() -> ProjectBuilder {
        ProjectBuilder::default()
    }

    pub fn compile(&self) -> Result<Vec<ContractCode>> {
        self.compiler.compile_dir(self.sources_path())
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProjectBuilder {
    /// The layout of the
    paths: Option<ProjectPathsConfig>,
    /// Where to find the compiler
    compiler: Option<StarknetCompile>,
}

impl ProjectBuilder {
    #[must_use]
    pub fn paths(mut self, paths: ProjectPathsConfig) -> Self {
        self.paths = Some(paths);
        self
    }

    #[must_use]
    pub fn compiler(mut self, compiler: impl Into<StarknetCompile>) -> Self {
        self.compiler = Some(compiler.into());
        self
    }

    pub fn build(self) -> Project {
        let Self { paths, compiler } = self;

        let paths = paths.unwrap_or_default();
        let mut compiler = compiler.unwrap_or_default();

        if compiler.get_import_paths().is_empty() {
            // configure `--cairo-path`
            compiler =
                compiler.import_path(paths.sources.clone()).import_paths(paths.libraries.clone());
        }

        Project { paths, compiler }
    }
}
