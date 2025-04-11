use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::{
    opts::{BuildOpts, ProjectPathOpts},
    utils::LoadConfig,
};
use foundry_common::{flatten, fs};
use std::path::PathBuf;

/// CLI arguments for `forge flatten`.
#[derive(Clone, Debug, Parser)]
pub struct FlattenArgs {
    /// The path to the contract to flatten.
    #[arg(value_hint = ValueHint::FilePath, value_name = "PATH")]
    pub target_path: PathBuf,

    /// The path to output the flattened contract.
    ///
    /// If not specified, the flattened contract will be output to stdout.
    #[arg(
        long,
        short,
        value_hint = ValueHint::FilePath,
        value_name = "PATH",
    )]
    pub output: Option<PathBuf>,

    #[command(flatten)]
    pub project_paths: ProjectPathOpts,
}

impl FlattenArgs {
    pub fn run(self) -> Result<()> {
        let Self { target_path, output, project_paths } = self;

        // flatten is a subset of `BuildArgs` so we can reuse that to get the config
        let build = BuildOpts { project_paths, ..Default::default() };
        let config = build.load_config()?;
        let project = config.ephemeral_project()?;

        let target_path = dunce::canonicalize(target_path)?;
        let flattened = flatten(project, &target_path)?;

        match output {
            Some(output) => {
                fs::create_dir_all(output.parent().unwrap())?;
                fs::write(&output, flattened)?;
                sh_println!("Flattened file written at {}", output.display())?;
            }
            None => sh_println!("{flattened}")?,
        };

        Ok(())
    }
}
