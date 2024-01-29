use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::{
    opts::{CoreBuildArgs, ProjectPathsArgs},
    utils::LoadConfig,
};
use foundry_common::{compile::ProjectCompiler, fs};
use foundry_compilers::{artifacts::Source, error::SolcError, flatten::Flattener, Graph};
use std::path::PathBuf;

/// CLI arguments for `forge flatten`.
#[derive(Clone, Debug, Parser)]
pub struct FlattenArgs {
    /// The path to the contract to flatten.
    #[clap(value_hint = ValueHint::FilePath, value_name = "PATH")]
    pub target_path: PathBuf,

    /// The path to output the flattened contract.
    ///
    /// If not specified, the flattened contract will be output to stdout.
    #[clap(
        long,
        short,
        value_hint = ValueHint::FilePath,
        value_name = "PATH",
    )]
    pub output: Option<PathBuf>,

    #[clap(flatten)]
    project_paths: ProjectPathsArgs,
}

impl FlattenArgs {
    pub fn run(self) -> Result<()> {
        let FlattenArgs { target_path, output, project_paths } = self;

        // flatten is a subset of `BuildArgs` so we can reuse that to get the config
        let build_args = CoreBuildArgs { project_paths, ..Default::default() };

        let config = build_args.try_load_config_emit_warnings()?;

        let target_path = dunce::canonicalize(target_path)?;

        // We need to provide Flattener with compiled output of target and all of its imports.
        let project = config.ephemeral_no_artifacts_project()?;
        let sources = Source::read_all(vec![target_path.clone()])?;
        let (sources, _) = Graph::resolve_sources(&project.paths, sources)?.into_sources();

        let compiler_output = ProjectCompiler::new().files(sources.into_keys()).compile(&project);

        let flattened = match compiler_output {
            Ok(compiler_output) => match Flattener::new(&project, &compiler_output, &target_path) {
                Ok(flattener) => Ok(flattener.flatten()),
                Err(err) => Err(err),
            },
            Err(_) => {
                // Fallback to the old flattening compilation if we couldn't compile the target
                // successfully. This would be the case if the target has invalid
                // syntax. (e.g. Solang)
                project.paths.flatten(&target_path)
            }
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
