use std::path::PathBuf;

use ethers::solc::{remappings::Remapping, ProjectPathsConfig};

use crate::{cmd::Cmd, utils};
use clap::{Parser, ValueHint};

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
        let root = self.root.clone().unwrap_or_else(|| {
            utils::find_git_root_path().unwrap_or_else(|_| std::env::current_dir().unwrap())
        });
        let root = dunce::canonicalize(&root)?;

        let contracts = match self.contracts {
            Some(ref contracts) => root.join(contracts),
            None => {
                if self.hardhat {
                    root.join("contracts")
                } else {
                    // no contract source directory was provided, determine the source directory
                    ProjectPathsConfig::find_source_dir(&root)
                }
            }
        };

        let lib_paths = utils::find_libs(&root, &self.lib_paths, self.hardhat);

        let remappings = utils::find_remappings(
            &lib_paths,
            &self.remappings,
            &root.join("remappings.txt"),
            &self.remappings_env,
        );

        // build the path
        let mut paths_builder = ProjectPathsConfig::builder().root(&root).sources(contracts);

        if !remappings.is_empty() {
            paths_builder = paths_builder.remappings(remappings);
        }

        let paths = paths_builder.build()?;
        let target_path = dunce::canonicalize(self.target_path)?;
        let flattened = paths
            .flatten(&target_path)
            .map_err(|err| eyre::Error::msg(format!("failed to flatten the file: {}", err)))?;

        match self.output {
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
