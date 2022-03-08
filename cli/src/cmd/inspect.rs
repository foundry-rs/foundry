use std::{
  path::PathBuf,
  str::FromStr,
};

use ethers::solc::remappings::Remapping;

use crate::cmd::{build::BuildArgs, Cmd};
use clap::{Parser, ValueHint};
use foundry_config::Config;

#[derive(Debug, Clone, Parser)]
pub struct CoreInspectArgs {
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

#[derive(Debug, Clone)]
pub enum Mode {
    IR,
    Bytecode,
}

impl FromStr for Mode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ir" | "IR" | "ir-mode" | "-ir" => Ok(Mode::IR),
            "bytecode" | "BYTECODE" | "-bytecode" => Ok(Mode::Bytecode),
            _ => Err(format!("Unrecognized mode `{}`, must be one of [IR, Bytecode]", s)),
        }
    }
}

#[derive(Debug, Clone, Parser)]
pub struct InspectArgs {
    #[clap(help = "the path to the contract to inspect", value_hint = ValueHint::FilePath)]
    pub target_path: PathBuf,

    #[clap(long, short, help = "the mode to build the ", value_hint = ValueHint::FilePath)]
    pub mode: Option<Mode>,

    #[clap(flatten)]
    core_inspect_args: CoreInspectArgs,
}

impl Cmd for InspectArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        let InspectArgs { target_path, mode, core_inspect_args } = self;

        // Reuse `BuildArgs` to get the config (following flatten convention)
        let build_args = BuildArgs {
            root: core_inspect_args.root,
            contracts: core_inspect_args.contracts,
            remappings: core_inspect_args.remappings,
            remappings_env: core_inspect_args.remappings_env,
            lib_paths: core_inspect_args.lib_paths,
            out_path: None,
            compiler: Default::default(),
            names: false,
            sizes: false,
            ignored_error_codes: vec![],
            no_auto_detect: false,
            offline: false,
            force: false,
            hardhat: core_inspect_args.hardhat,
            libraries: vec![],
            watch: Default::default(),
        };

        let config = Config::from(&build_args);

        // let paths = config.project_paths();
        // let target_path = dunce::canonicalize(target_path)?;
        // let flattened = paths
        //     .flatten(&target_path)
        //     .map_err(|err| eyre::Error::msg(format!("failed to flatten the file: {}", err)))?;

        // TODO: fetch or generate the IR or bytecode

        // IR by default
        if let Some(Mode::Bytecode) = mode {
          // TODO: output bytecode
          println!("<bytecode>");
        } else {
          // TODO: output IR
          println!("<IR>");
        }

        Ok(())
    }
}
