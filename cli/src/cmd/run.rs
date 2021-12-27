use crate::{cmd::Cmd};
use structopt::StructOpt;
use std::{
    path::{PathBuf}
};

use ethers::{
    solc::{
        artifacts::{Optimizer, Settings},
        EvmVersion, MinimalCombinedArtifacts, Project, ProjectCompileOutput, ProjectPathsConfig,
        SolcConfig,
    },
};

#[derive(Debug, Clone, StructOpt)]
pub struct RunArgs {
    #[structopt(
        help = "the path to the contract to run",
        long
    )]
    pub path: PathBuf,
}

impl Cmd for RunArgs {
    type Output = ProjectCompileOutput<MinimalCombinedArtifacts>;
    fn run(self) -> eyre::Result<Self::Output> {
        let project = self.project()?;
        let output = project.compile()?;
        if output.has_compiler_errors() {
            // return the diagnostics error back to the user.
            eyre::bail!(output.to_string())
        } else if output.is_unchanged() {
            println!("no files changed, compilation skippped.");
        } else {
            println!("success.");
        }
        Ok(output)
    }
}

impl RunArgs {
    pub fn project(&self) -> eyre::Result<Project> {
        let path = self.path.clone();
        let canonical_path = std::fs::canonicalize(&path)?;
        let contracts = &path;

        // build the path
        let paths_builder =
            ProjectPathsConfig::builder().root(&canonical_path).sources(contracts);


        let paths = paths_builder.build()?;

        let optimizer =
            Optimizer { enabled: Some(true), runs: Some(200) };

        let solc_settings =
            Settings { optimizer, evm_version: Some(EvmVersion::London), ..Default::default() };
        let mut builder = Project::builder()
            .paths(paths)
            .allowed_path(&path)
            .solc_config(SolcConfig::builder().settings(solc_settings).build()?);

        builder = builder.no_auto_detect();
        builder = builder.no_artifacts();

        let project = builder.build()?;
        

        project.cleanup()?;

        Ok(project)
    }
}