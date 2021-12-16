//! The forge-shell internal session

use crate::term;
use ethers::{
    prelude::{artifacts::Settings, SolcConfig},
    solc::{artifacts::CompactContract, EvmVersion, Project, ProjectPathsConfig},
};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Default)]
pub struct Session {
    /// All loaded projects identified by their name, the directory name
    pub projects: BTreeMap<String, Project>,
    /// Additional libraries
    pub libs: HashSet<PathBuf>,
    /// all artifacts (project name -> contract -> artifact)
    pub artifacts: BTreeMap<String, BTreeMap<String, CompactContract>>,
}

impl Session {
    /// Registers a new project
    pub fn add_project(&mut self, path: impl AsRef<Path>) -> eyre::Result<String> {
        let path = path.as_ref();
        if !path.is_dir() {
            eyre::bail!("\"{}\" is not a valid project path", path.display())
        }

        let dir_name = path
            .file_name()
            .ok_or_else(|| eyre::eyre!("Failed to get dir name \"{}\"", path.display()))?
            .to_string_lossy()
            .to_string();

        if self.projects.contains_key(&dir_name) {
            eyre::bail!("A project with the name \"{}\" already exists", dir_name)
        }

        let solc_settings =
            Settings { evm_version: Some(EvmVersion::default()), ..Default::default() };
        let paths = ProjectPathsConfig::dapptools(path)?;
        let project = Project::builder()
            .allowed_path(&paths.root)
            .allowed_paths(paths.libraries.clone())
            .paths(paths)
            .solc_config(SolcConfig::builder().settings(solc_settings).build()?)
            .build()?;
        self.projects.insert(dir_name.clone(), project);
        Ok(dir_name)
    }

    /// Compiles all registered projects
    pub fn compile_all(&mut self) -> eyre::Result<()> {
        for (name, project) in &self.projects {
            term::info(format!("compiling `{}`", name));
            let output = project.compile()?;
            if output.has_compiler_errors() {
                term::error(&output.to_string())
            } else if output.is_unchanged() {
                term::info(format!("no files changed, compilation skipped for `{}`.", name));
            } else {
                term::success(format!("successfully compiled `{}`", name));
            }
            let artifacts: BTreeMap<_, _> = output.into_artifacts().collect();
            self.artifacts.insert(name.clone(), artifacts);
        }
        Ok(())
    }
}
