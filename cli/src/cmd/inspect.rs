use crate::cmd::{build, Cmd};
use clap::{Parser};
use ethers::prelude::artifacts::output_selection::ContractOutputSelection;

#[derive(Debug, Clone, Parser)]
pub struct InspectArgs {
    /// All build arguments are supported
    #[clap(flatten)]
    build: build::BuildArgs,

    #[clap(help = "the contract to inspect")]
    pub contract: String,

    #[clap(long, short, help = "the contract output selection")]
    pub mode: Option<ContractOutputSelection>,
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
        let artifacts = outcome.compiled_artifacts().find(contract.clone());

        // Unwrap the inner artifact
        let artifact = match artifacts {
          Some(a) => a,
          None => {
            eyre::eyre!(
              "Could not find artifact `{}` in the compiled artifacts",
              contract
            );
            return Ok(());
          }
        };

        // Match on ContractOutputSelection
        if let Some(m) = mode {
          match m {
            ContractOutputSelection::Abi => println!("{:?}", artifact.abi),
            ContractOutputSelection::DevDoc => println!("{:?}", artifact.devdoc),
            ContractOutputSelection::UserDoc => println!("{:?}", artifact.userdoc),
            ContractOutputSelection::Metadata => println!("{:?}", artifact.metadata),
            ContractOutputSelection::Ir => println!("{:?}", artifact.ir),
            ContractOutputSelection::IrOptimized => println!("{:?}", artifact.ir_optimized),
            ContractOutputSelection::StorageLayout => println!("{:?}", artifact.storage_layout),
            ContractOutputSelection::Evm(e) => println!("{:?}", artifact.bytecode),
            ContractOutputSelection::Ewasm(e) => println!("{:?}", artifact.ewasm),
          }
        } else {
          // Otherwise, by default, print the bytecode
          println!("{:?}", artifact.bytecode);
        }

        Ok(())
    }
}
