use std::{fmt, str::FromStr};

use crate::cmd::{build, Cmd};
use clap::Parser;
use serde_json::{Value, to_value};
use tracing_subscriber::fmt::format::Json;

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
    #[clap(help = "the contract to inspect")]
    pub contract: String,

    #[clap(help = "the contract artifact field to inspect")]
    pub mode: ContractArtifactFields,

    /// All build arguments are supported
    #[clap(flatten)]
    build: build::BuildArgs,
}

impl Cmd for InspectArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        let InspectArgs { contract, mode, build  } = self;

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
        let output: Value = match mode {
            ContractArtifactFields::Abi => to_value(&artifact.abi).unwrap(),
            ContractArtifactFields::Bytecode => to_value(&artifact.bytecode).unwrap(),
            ContractArtifactFields::DeployedBytecode => to_value(&artifact.deployed_bytecode).unwrap(),
            ContractArtifactFields::Assembly => to_value(&artifact.assembly).unwrap(),
            ContractArtifactFields::MethodIdentifiers => to_value(&artifact.method_identifiers).unwrap(),
            ContractArtifactFields::GasEstimates => to_value(&artifact.gas_estimates).unwrap(),
            ContractArtifactFields::Metadata => to_value(&artifact.metadata).unwrap(),
            ContractArtifactFields::StorageLayout => to_value(&artifact.storage_layout).unwrap(),
            ContractArtifactFields::UserDoc => to_value(&artifact.userdoc).unwrap(),
            ContractArtifactFields::DevDoc => to_value(&artifact.devdoc).unwrap(),
            ContractArtifactFields::Ir => to_value(&artifact.ir).unwrap(),
            ContractArtifactFields::IrOptimized => to_value(&artifact.ir_optimized).unwrap(),
            ContractArtifactFields::Ewasm => to_value(&artifact.ewasm).unwrap(),
        };

        // Pretty print the output with serde_json
        println!("{}", serde_json::to_string_pretty(&output).unwrap());

        Ok(())
    }
}
