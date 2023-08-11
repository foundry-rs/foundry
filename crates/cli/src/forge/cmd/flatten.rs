use clap::{Parser, ValueHint};
use foundry_cli::{
    opts::{CoreBuildArgs, ProjectPathsArgs},
    utils::{Cmd, LoadConfig},
};
use foundry_common::fs;
use std::path::PathBuf;

/// CLI arguments for `forge flatten`.
#[derive(Debug, Clone, Parser)]
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

impl Cmd for FlattenArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        let FlattenArgs { target_path, output, project_paths } = self;

        // flatten is a subset of `BuildArgs` so we can reuse that to get the config
        let build_args = CoreBuildArgs {
            project_paths,
            out_path: Default::default(),
            compiler: Default::default(),
            ignored_error_codes: vec![],
            deny_warnings: false,
            no_auto_detect: false,
            use_solc: None,
            offline: false,
            force: false,
            libraries: vec![],
            via_ir: false,
            revert_strings: None,
            silent: false,
            build_info: false,
            build_info_path: None,
        };

        let config = build_args.try_load_config_emit_warnings()?;

        let paths = config.project_paths();
        let target_path = dunce::canonicalize(target_path)?;
        let flattened = paths
            .flatten(&target_path)
            .map_err(|err| eyre::Error::msg(format!("Failed to flatten the file: {err}")))?;

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
