use std::{fmt, str::FromStr};

use crate::cmd::{build, Cmd};
use clap::Parser;

/// Contract level output selection
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ContractArtifactFields {
    Abi,
    Bytecode,
    DeployedBytecode,
    Assembly,
    MethodIdentifiers,
    GasEstimates,
    Metadata,
    StorageLayout,
    UserDoc,
    DevDoc,
    Ir,
    IrOptimized,
    Ewasm,
}

impl fmt::Display for ContractArtifactFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContractArtifactFields::Abi => f.write_str("abi"),
            ContractArtifactFields::Bytecode => f.write_str("bytecode"),
            ContractArtifactFields::DeployedBytecode => f.write_str("deployedBytecode"),
            ContractArtifactFields::Assembly => f.write_str("assembly"),
            ContractArtifactFields::MethodIdentifiers => f.write_str("methodIdentifiers"),
            ContractArtifactFields::GasEstimates => f.write_str("gasEstimates"),
            ContractArtifactFields::Metadata => f.write_str("metadata"),
            ContractArtifactFields::StorageLayout => f.write_str("storageLayout"),
            ContractArtifactFields::UserDoc => f.write_str("userdoc"),
            ContractArtifactFields::DevDoc => f.write_str("devdoc"),
            ContractArtifactFields::Ir => f.write_str("ir"),
            ContractArtifactFields::IrOptimized => f.write_str("irOptimized"),
            ContractArtifactFields::Ewasm => f.write_str("ewasm"),
        }
    }
}

impl FromStr for ContractArtifactFields {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "abi" => Ok(ContractArtifactFields::Abi),
            "bytecode" => Ok(ContractArtifactFields::Bytecode),
            "deployedBytecode" | "deployed_bytecode" | "deployed-bytecode" | "deployed" |
            "deployedbytecode" => Ok(ContractArtifactFields::DeployedBytecode),
            "assembly" | "asm" => Ok(ContractArtifactFields::Assembly),
            "methodIdentifiers" | "method_identifiers" | "method-identifiers" => {
                Ok(ContractArtifactFields::MethodIdentifiers)
            }
            "gasEstimates" | "gas_estimates" | "gas-estimates" | "gasestimates" => {
                Ok(ContractArtifactFields::GasEstimates)
            }
            "metadata" => Ok(ContractArtifactFields::Metadata),
            "storageLayout" | "storage_layout" | "storage-layout" | "storagelayout" => {
                Ok(ContractArtifactFields::StorageLayout)
            }
            "userdoc" => Ok(ContractArtifactFields::UserDoc),
            "devdoc" => Ok(ContractArtifactFields::DevDoc),
            "ir" => Ok(ContractArtifactFields::Ir),
            "ir-optimized" | "irOptimized" | "iroptimized" => {
                Ok(ContractArtifactFields::IrOptimized)
            }
            "ewasm" => Ok(ContractArtifactFields::Ewasm),
            _ => Ok(ContractArtifactFields::Bytecode),
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

    #[clap(long, short, help = "the contract artifact field to inspect")]
    pub mode: ContractArtifactFields,
}

impl Cmd for InspectArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        let InspectArgs { build, contract, mode } = self;

        // Build the project
        let project = build.project()?;
        let outcome = super::compile(&project, build.names, build.sizes)?;

        // For the compiled artifacts, find the contract
        let artifacts = outcome.compiled_artifacts().find(contract.clone());

        // Unwrap the inner artifact
        let artifact = artifacts
            .ok_or_else(|| {
                eyre::eyre!("Could not find artifact `{}` in the compiled artifacts", contract);
            })
            .unwrap();

        // Match on ContractOutputSelection
        match mode {
            ContractArtifactFields::Abi => println!("{:?}", artifact.abi),
            ContractArtifactFields::Bytecode => println!("{:?}", artifact.bytecode),
            ContractArtifactFields::DeployedBytecode => {
                println!("{:?}", artifact.deployed_bytecode)
            }
            ContractArtifactFields::Assembly => println!("{:?}", artifact.assembly),
            ContractArtifactFields::MethodIdentifiers => {
                println!("{:?}", artifact.method_identifiers)
            }
            ContractArtifactFields::GasEstimates => println!("{:?}", artifact.gas_estimates),
            ContractArtifactFields::Metadata => println!("{:?}", artifact.metadata),
            ContractArtifactFields::StorageLayout => println!("{:?}", artifact.storage_layout),
            ContractArtifactFields::UserDoc => println!("{:?}", artifact.userdoc),
            ContractArtifactFields::DevDoc => println!("{:?}", artifact.devdoc),
            ContractArtifactFields::Ir => println!("{:?}", artifact.ir),
            ContractArtifactFields::IrOptimized => println!("{:?}", artifact.ir_optimized),
            ContractArtifactFields::Ewasm => println!("{:?}", artifact.ewasm),
        }

        Ok(())
    }
}
