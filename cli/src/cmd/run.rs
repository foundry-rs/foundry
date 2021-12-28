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

impl RunArgs {
    /// Compiles the file with auto-detection and compiler params.
    // TODO: This is too verbose. We definitely want an easier way to do "take this file, detect
    // its solc version and give me all its ABIs & Bytecodes in memory w/o touching disk".
    pub fn build(&self) -> eyre::Result<CompactContract> {
        let paths = ProjectPathsConfig::builder().root(&self.path).sources(&self.path).build()?;

        let optimizer = Optimizer {
            enabled: Some(self.compiler.optimize),
            runs: Some(self.compiler.optimize_runs as usize),
        };

        let solc_settings = Settings {
            optimizer,
            evm_version: Some(self.compiler.evm_version),
            ..Default::default()
        };
        let solc_cfg = SolcConfig::builder().settings(solc_settings).build()?;

        // setup the compiler
        let mut builder = Project::builder()
            .paths(paths)
            .allowed_path(&self.path)
            .solc_config(solc_cfg)
            // we do not want to generate any compilation artifacts in the script run mode
            .no_artifacts()
            // no cache
            .ephemeral();
        if self.no_auto_detect {
            builder = builder.no_auto_detect();
        }
        let project = builder.build()?;

        println!("compiling...");
        let output = project.compile()?;
        if output.has_compiler_errors() {
            // return the diagnostics error back to the user.
            eyre::bail!(output.to_string())
        } else if output.is_unchanged() {
            println!("no files changed, compilation skippped.");
        } else {
            println!("success.");
        };

        // get the contracts
        let contracts = output.output();

        // get the specific contract
        let contract = if let Some(ref contract) = self.contract {
            let contract = contracts.find(contract).ok_or_else(|| {
                eyre::Error::msg("contract not found, did you type the name wrong?")
            })?;
            CompactContract::from(contract)
        } else {
            let mut contracts =
                contracts.contracts_into_iter().filter(|(_, contract)| contract.evm.is_some());
            let contract = contracts.next().ok_or_else(|| eyre::Error::msg("no contract found"))?.1;
            if contracts.peekable().peek().is_some() {
                eyre::bail!(
                    ">1 contracts found, please provide a contract name to choose one of them"
                )
            }
            CompactContract::from(contract)
        };
        Ok(contract)
    }
}
