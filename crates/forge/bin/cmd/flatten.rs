use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::{
    opts::{CoreBuildArgs, ProjectPathsArgs},
    utils::LoadConfig,
};
use foundry_common::{compile::ProjectCompiler, fs};
use foundry_compilers::{error::SolcError, flatten::Flattener};
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
        let mut config = build_args.try_load_config_emit_warnings()?;
        // `Flattener` uses the typed AST for better flattening results.
        config.ast = true;
        let project = config.ephemeral_no_artifacts_project()?;

        let target_path = dunce::canonicalize(target_path)?;
        let compiler_output = ProjectCompiler::new().files([target_path.clone()]).compile(&project);

        let flattened = match compiler_output {
            Ok(compiler_output) => {
                Flattener::new(&project, &compiler_output, &target_path).map(|f| f.flatten())
            }
            Err(_) => {
                // Fallback to the old flattening implementation if we couldn't compile the target
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
