use crate::{
    cmd::Cmd,
    opts::forge::{CompilerArgs, EvmOpts},
};
use forge::ContractRunner;
use foundry_utils::IntoFunction;
use std::path::PathBuf;
use structopt::StructOpt;

use ethers::{
    prelude::artifacts::CompactContract,
    solc::{
        artifacts::{Optimizer, Settings},
        Project, ProjectPathsConfig, SolcConfig,
    },
};

use evm_adapters::Evm;

#[derive(Debug, Clone, StructOpt)]
pub struct RunArgs {
    #[structopt(help = "the path to the contract to run")]
    pub path: PathBuf,

    #[structopt(flatten)]
    pub compiler: CompilerArgs,

    #[structopt(flatten)]
    pub evm_opts: EvmOpts,

    #[structopt(
        long,
        short,
        help = "the function you want to call on the script contract, defaults to run()"
    )]
    pub sig: Option<String>,

    #[structopt(
        long,
        short,
        help = "the contract you want to call and deploy, only necessary if there are more than 1 contract (Interfaces do not count) definitions on the script"
    )]
    pub contract: Option<String>,

    #[structopt(
        help = "if set to true, skips auto-detecting solc and uses what is in the user's $PATH ",
        long
    )]
    pub no_auto_detect: bool,
}

impl Cmd for RunArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        Ok(())
    }
}
