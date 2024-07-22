use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::{
    opts::{CoreBuildArgs, ProjectPathsArgs},
    utils::LoadConfig,
};
use foundry_common::{compile::with_compilation_reporter, fs};
use foundry_compilers::{
    compilers::solc::SolcLanguage,
    error::SolcError,
    flatten::{Flattener, FlattenerError},
};
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
    project_paths: ProjectPathsArgs,
}

impl FlattenArgs {
    pub fn run(self) -> Result<()> {
        let Self { target_path, output, project_paths } = self;

        // flatten is a subset of `BuildArgs` so we can reuse that to get the config
        let build_args = CoreBuildArgs { project_paths, ..Default::default() };
        let config = build_args.try_load_config_emit_warnings()?;
        let project = config.create_project(false, true)?;

        let target_path = dunce::canonicalize(target_path)?;

        let flattener =
            with_compilation_reporter(true, || Flattener::new(project.clone(), &target_path));

        let flattened = match flattener {
            Ok(flattener) => Ok(flattener.flatten()),
            Err(FlattenerError::Compilation(_)) => {
                // Fallback to the old flattening implementation if we couldn't compile the target
                // successfully. This would be the case if the target has invalid
                // syntax. (e.g. Solang)
                project.paths.with_language::<SolcLanguage>().flatten(&target_path)
            }
            Err(FlattenerError::Other(err)) => Err(err),
        }
        .map_err(|err: SolcError| eyre::eyre!("Failed to flatten: {err}"))?;

        match output {
            Some(output) => {
                fs::create_dir_all(output.parent().unwrap())?;
                fs::write(&output, flattened)?;
                println!("Flattened file written at {}", output.display());
            }
            None => println!("{flattened}"),
        };

        Ok(())
    }
}
