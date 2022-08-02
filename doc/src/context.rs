use ethers_solc::ProjectPathsConfig;

use crate::config::DocConfig;

pub trait Context {
    fn config(&self) -> eyre::Result<DocConfig>;
    fn project_paths(&self) -> eyre::Result<ProjectPathsConfig>;
}
