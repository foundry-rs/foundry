use std::{fmt, str::FromStr};

use crate::{
    cmd::{
        build::{self, BuildArgs},
        Cmd,
    },
    opts::forge::CompilerArgs,
};
use clap::Parser;
use ethers::prelude::artifacts::output_selection::{
    ContractOutputSelection, EvmOutputSelection, EwasmOutputSelection,
};
use serde_json::{to_value, Value};

/// Contract level output selection
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ContractArtifactFields {
    Abi,
    Bytecode,
    DeployedBytecode,
    Assembly,
    AssemblyOptimized,
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
            ContractArtifactFields::AssemblyOptimized => f.write_str("assemblyOptimized"),
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
            "asmOptimized" | "assemblyOptimized" | "assemblyoptimized" | "assembly_optimized" |
            "amso" => Ok(ContractArtifactFields::AssemblyOptimized),
            "methodIdentifiers" | "method_identifiers" | "method-identifiers" => {
                Ok(ContractArtifactFields::MethodIdentifiers)
            }
            "gasEstimates" | "gas" | "gas_estimates" | "gas-estimates" | "gasestimates" => {
                Ok(ContractArtifactFields::GasEstimates)
            }
            "metadata" | "meta" => Ok(ContractArtifactFields::Metadata),
            "storageLayout" | "storage_layout" | "storage-layout" | "storagelayout" | "storage" => {
                Ok(ContractArtifactFields::StorageLayout)
            }
            "userdoc" => Ok(ContractArtifactFields::UserDoc),
            "devdoc" => Ok(ContractArtifactFields::DevDoc),
            "ir" => Ok(ContractArtifactFields::Ir),
            "ir-optimized" | "irOptimized" | "iroptimized" | "iro" => {
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
        let InspectArgs { contract, mode, build } = self;

        // Map mode to ContractOutputSelection
        let mut cos = if let Some(v) = build.compiler.extra_output { v } else { vec![] };
        if !cos.iter().any(|&i| i.to_string() == mode.to_string()) {
            match mode {
                ContractArtifactFields::Abi => cos.push(ContractOutputSelection::Abi),
                ContractArtifactFields::Bytecode => { /* Auto Generated */ }
                ContractArtifactFields::DeployedBytecode => { /* Auto Generated */ }
                ContractArtifactFields::Assembly | ContractArtifactFields::AssemblyOptimized => {
                    cos.push(ContractOutputSelection::Evm(EvmOutputSelection::Assembly))
                }
                ContractArtifactFields::MethodIdentifiers => {
                    cos.push(ContractOutputSelection::Evm(EvmOutputSelection::MethodIdentifiers))
                }
                ContractArtifactFields::GasEstimates => {
                    cos.push(ContractOutputSelection::Evm(EvmOutputSelection::GasEstimates))
                }
                ContractArtifactFields::Metadata => cos.push(ContractOutputSelection::Metadata),
                ContractArtifactFields::StorageLayout => {
                    cos.push(ContractOutputSelection::StorageLayout)
                }
                ContractArtifactFields::UserDoc => cos.push(ContractOutputSelection::UserDoc),
                ContractArtifactFields::DevDoc => cos.push(ContractOutputSelection::DevDoc),
                ContractArtifactFields::Ir => cos.push(ContractOutputSelection::Ir),
                ContractArtifactFields::IrOptimized => {
                    cos.push(ContractOutputSelection::IrOptimized)
                }
                ContractArtifactFields::Ewasm => {
                    cos.push(ContractOutputSelection::Ewasm(EwasmOutputSelection::All))
                }
            }
        }

        // Run Optimized?
        let optimized = if let ContractArtifactFields::AssemblyOptimized = mode {
            true
        } else {
            build.compiler.optimize
        };

        // Build modified Args
        let modified_build_args = BuildArgs {
            compiler: CompilerArgs {
                extra_output: Some(cos),
                optimize: optimized,
                ..build.compiler
            },
            ..build
        };

        // Build the project
        let project = modified_build_args.project()?;
        let outcome = super::suppress_compile(&project)?;

        // If the project is unchanged, used cached artifacts
        let artifacts = if outcome.is_unchanged() {
            outcome.cached_artifacts()
        } else {
            outcome.compiled_artifacts()
        };

        // For the compiled artifacts, find the contract
        let found_artifact = artifacts.find(contract.clone());

        // Unwrap the inner artifact
        let artifact = found_artifact.unwrap_or_else(|| {
            eyre::eyre!("Could not find artifact `{}` in the compiled artifacts", contract);
            panic!("Could not find artifact `{}` in the compiled artifacts", contract)
        });

        // Match on ContractArtifactFields and Pretty Print
        match mode {
            ContractArtifactFields::Abi => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&to_value(&artifact.abi).unwrap()).unwrap()
                );
            }
            ContractArtifactFields::Bytecode => {
                let tval: Value = to_value(&artifact.bytecode).unwrap();
                println!("{}", tval.get("object").unwrap_or(&tval).clone().as_str().unwrap());
            }
            ContractArtifactFields::DeployedBytecode => {
                let tval: Value = to_value(&artifact.deployed_bytecode).unwrap();
                println!("{}", tval.get("object").unwrap_or(&tval).clone().as_str().unwrap());
            }
            ContractArtifactFields::Assembly | ContractArtifactFields::AssemblyOptimized => {
                println!("{}", to_value(&artifact.assembly).unwrap().as_str().unwrap());
            }
            ContractArtifactFields::MethodIdentifiers => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&to_value(&artifact.method_identifiers).unwrap())
                        .unwrap()
                );
            }
            ContractArtifactFields::GasEstimates => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&to_value(&artifact.gas_estimates).unwrap())
                        .unwrap()
                );
            }
            ContractArtifactFields::Metadata => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&to_value(&artifact.metadata).unwrap()).unwrap()
                );
            }
            ContractArtifactFields::StorageLayout => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&to_value(&artifact.storage_layout).unwrap())
                        .unwrap()
                );
            }
            ContractArtifactFields::UserDoc => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&to_value(&artifact.userdoc).unwrap()).unwrap()
                );
            }
            ContractArtifactFields::DevDoc => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&to_value(&artifact.devdoc).unwrap()).unwrap()
                );
            }
            ContractArtifactFields::Ir => {
                println!("{}", to_value(&artifact.ir).unwrap().as_str().unwrap());
            }
            ContractArtifactFields::IrOptimized => {
                println!("{}", to_value(&artifact.ir_optimized).unwrap().as_str().unwrap());
            }
            ContractArtifactFields::Ewasm => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&to_value(&artifact.ewasm).unwrap()).unwrap()
                );
            }
        };

        Ok(())
    }
}
