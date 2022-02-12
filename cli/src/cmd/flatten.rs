use std::path::PathBuf;

use ethers::solc::remappings::Remapping;

use crate::cmd::{build::BuildArgs, Cmd};
use clap::{Parser, ValueHint};
use foundry_config::Config;

#[derive(Debug, Clone, Parser)]
pub struct FlattenArgs {
    #[clap(help = "the path to the contract to flatten", value_hint = ValueHint::FilePath)]
    pub target_path: PathBuf,

    #[clap(long, short, help = "output path for the flattened contract", value_hint = ValueHint::FilePath)]
    pub output: Option<PathBuf>,

    #[clap(
        help = "the project's root path. By default, this is the root directory of the current Git repository or the current working directory if it is not part of a Git repository",
        long,
        value_hint = ValueHint::DirPath
    )]
    pub root: Option<PathBuf>,

    #[clap(
        env = "DAPP_SRC",
        help = "the directory relative to the root under which the smart contracts are",
        long,
        short,
        value_hint = ValueHint::DirPath
    )]
    pub contracts: Option<PathBuf>,

    #[clap(help = "the remappings", long, short)]
    pub remappings: Vec<Remapping>,
    #[clap(long = "remappings-env", env = "DAPP_REMAPPINGS")]
    pub remappings_env: Option<String>,

    #[clap(
        help = "the paths where your libraries are installed",
        long,
        value_hint = ValueHint::DirPath
    )]
    pub lib_paths: Vec<PathBuf>,

    #[clap(
        help = "uses hardhat style project layout. This a convenience flag and is the same as `--contracts contracts --lib-paths node_modules`",
        long,
        conflicts_with = "contracts",
        alias = "hh"
    )]
    pub hardhat: bool,
}

impl Cmd for FlattenArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        let FlattenArgs {
            target_path,
            output,
            root,
            contracts,
            remappings,
            remappings_env,
            lib_paths,
            hardhat,
        } = self;
        // flatten is a subset of `BuildArgs` so we can reuse that to get the config
        let build_args = BuildArgs {
            root,
            contracts,
            remappings,
            remappings_env,
            lib_paths,
            out_path: None,
            compiler: Default::default(),
            ignored_error_codes: vec![],
            no_auto_detect: false,
            offline: false,
            force: false,
            hardhat,
            libraries: vec![],
        };

        let config = Config::from(&build_args);

        let paths = config.project_paths();
        let target_path = dunce::canonicalize(target_path)?;
        let flattened = paths
            .flatten(&target_path)
            .map_err(|err| eyre::Error::msg(format!("failed to flatten the file: {}", err)))?;

        match output {
            Some(output) => {
                std::fs::create_dir_all(&output.parent().unwrap())?;
                std::fs::write(&output, flattened)?;
                println!("Flattened file written at {}", output.display());
            }
            None => println!("{}", flattened),
        };

        Ok(())
    }
}
