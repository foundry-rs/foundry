use std::path::PathBuf;

use ethers::solc::remappings::Remapping;

use crate::cmd::{forge::build::BuildArgs, Cmd};
use clap::{Parser, ValueHint};
use foundry_config::Config;

#[derive(Debug, Clone, Parser)]
pub struct CoreFlattenArgs {
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
        help = "the path where cached compiled contracts are stored",
        long = "cache-path",
        value_hint = ValueHint::DirPath
    )]
    pub cache_path: Option<PathBuf>,

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

#[derive(Debug, Clone, Parser)]
pub struct FlattenArgs {
    #[clap(help = "the path to the contract to flatten", value_hint = ValueHint::FilePath)]
    pub target_path: PathBuf,

    #[clap(long, short, help = "output path for the flattened contract", value_hint = ValueHint::FilePath)]
    pub output: Option<PathBuf>,

    #[clap(flatten)]
    core_flatten_args: CoreFlattenArgs,
}

impl Cmd for FlattenArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        let FlattenArgs { target_path, output, core_flatten_args } = self;

        // flatten is a subset of `BuildArgs` so we can reuse that to get the config
        let build_args = BuildArgs {
            root: core_flatten_args.root,
            contracts: core_flatten_args.contracts,
            remappings: core_flatten_args.remappings,
            remappings_env: core_flatten_args.remappings_env,
            cache_path: core_flatten_args.cache_path,
            lib_paths: core_flatten_args.lib_paths,
            out_path: None,
            compiler: Default::default(),
            names: false,
            sizes: false,
            ignored_error_codes: vec![],
            no_auto_detect: false,
            use_solc: None,
            offline: false,
            force: false,
            hardhat: core_flatten_args.hardhat,
            libraries: vec![],
            watch: Default::default(),
            via_ir: false,
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
