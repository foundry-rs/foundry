use std::{
  path::PathBuf,
  str::FromStr,
};

use crate::cmd::{build, Cmd};
use clap::{Parser, ValueHint};
use foundry_config::Config;

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
    /// All build arguments are supported
    #[clap(flatten)]
    build: build::BuildArgs,

    #[clap(help = "the contract to inspect")]
    pub contract: String,

    #[clap(long, short, help = "the mode to build the ")]
    pub mode: Option<Mode>,
}

impl Cmd for InspectArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        let InspectArgs { build, contract, mode } = self;

        // Build the project
        let project = build.project()?;
        let outcome =  super::compile(
          &project,
          build.names,
          build.sizes
        )?;

        // For the compiled artifacts, find the contract
        let artifacts = outcome.compiled_artifacts().find(contract);

        println!("ConfigurableContractArtifact: {:?}", artifacts);

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
