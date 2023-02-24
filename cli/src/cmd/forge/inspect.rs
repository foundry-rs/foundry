use crate::{
    cmd::{forge::build::CoreBuildArgs, Cmd},
    opts::forge::CompilerArgs,
};
use clap::Parser;
use comfy_table::{presets::ASCII_MARKDOWN, Table};
use ethers::{
    prelude::{
        artifacts::output_selection::{
            BytecodeOutputSelection, ContractOutputSelection, DeployedBytecodeOutputSelection,
            EvmOutputSelection, EwasmOutputSelection,
        },
        info::ContractInfo,
    },
    solc::{
        artifacts::{LosslessAbi, StorageLayout},
        utils::canonicalize,
    },
};
use foundry_common::compile;
use serde_json::{to_value, Value};
use std::{fmt, str::FromStr};
use tracing::trace;

/// CLI arguments for `forge inspect`.
#[derive(Debug, Clone, Parser)]
pub struct InspectArgs {
    #[clap(
        help = "The identifier of the contract to inspect in the form `(<path>:)?<contractname>`.",
        value_name = "CONTRACT"
    )]
    pub contract: ContractInfo,

    #[clap(
        value_name = "FIELD",
        help = r#"The contract artifact field to inspect.

possible_values = ["abi", "b/bytes/bytecode", "deployedBytecode/deployed_bytecode/deployed-bytecode/deployedbytecode/deployed", "assembly/asm", "asmOptimized/assemblyOptimized/assemblyoptimized/assembly_optimized/asmopt/assembly-optimized/asmo/asm-optimized/asmoptimized/asm_optimized",
"methods/methodidentifiers/methodIdentifiers/method_identifiers/method-identifiers/mi", "gasEstimates/gas/gas_estimates/gas-estimates/gasestimates",
"storageLayout/storage_layout/storage-layout/storagelayout/storage", "devdoc/dev-doc/devDoc",
"ir", "ir-optimized/irOptimized/iroptimized/iro/iropt", "metadata/meta", "userdoc/userDoc/user-doc", "ewasm/e-wasm", "events/ev"]"#
    )]
    pub field: ContractArtifactFields,

    #[clap(long, help = "Pretty print the selected field, if supported.")]
    pub pretty: bool,

    /// All build arguments are supported
    #[clap(flatten)]
    build: CoreBuildArgs,
}

impl Cmd for InspectArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        let InspectArgs { mut contract, field, build, pretty } = self;

        trace!(target : "forge", ?field, ?contract, "running forge inspect");

        // Map field to ContractOutputSelection
        let mut cos = build.compiler.extra_output;
        if !field.is_default() && !cos.iter().any(|selected| field.eq(selected)) {
            cos.push(field.into());
        }

        // Run Optimized?
        let optimized = if let ContractArtifactFields::AssemblyOptimized = field {
            true
        } else {
            build.compiler.optimize
        };

        // Build modified Args
        let modified_build_args = CoreBuildArgs {
            compiler: CompilerArgs { extra_output: cos, optimize: optimized, ..build.compiler },
            ..build
        };

        // Build the project
        let project = modified_build_args.project()?;
        let outcome = if let Some(ref mut contract_path) = contract.path {
            let target_path = canonicalize(&*contract_path)?;
            *contract_path = target_path.to_string_lossy().to_string();
            compile::compile_files(&project, vec![target_path], true)
        } else {
            compile::suppress_compile(&project)
        }?;

        // Find the artifact
        let found_artifact = outcome.find_contract(&contract);

        trace!(target : "forge", artifact=?found_artifact, input=?contract, "Found contract");

        // Unwrap the inner artifact
        let artifact = found_artifact.ok_or_else(|| {
            eyre::eyre!("Could not find artifact `{contract}` in the compiled artifacts")
        })?;

        // Match on ContractArtifactFields and Pretty Print
        match field {
            ContractArtifactFields::Abi => {
                println!("{}", serde_json::to_string_pretty(&to_value(&artifact.abi)?)?);
            }
            ContractArtifactFields::Bytecode => {
                let tval: Value = to_value(&artifact.bytecode)?;
                println!(
                    "{}",
                    tval.get("object").unwrap_or(&tval).clone().as_str().ok_or_else(
                        || eyre::eyre!("Failed to extract artifact bytecode as a string")
                    )?
                );
            }
            ContractArtifactFields::DeployedBytecode => {
                let tval: Value = to_value(&artifact.deployed_bytecode)?;
                println!(
                    "{}",
                    tval.get("object").unwrap_or(&tval).clone().as_str().ok_or_else(
                        || eyre::eyre!("Failed to extract artifact deployed bytecode as a string")
                    )?
                );
            }
            ContractArtifactFields::Assembly | ContractArtifactFields::AssemblyOptimized => {
                println!(
                    "{}",
                    to_value(&artifact.assembly)?.as_str().ok_or_else(|| eyre::eyre!(
                        "Failed to extract artifact assembly as a string"
                    ))?
                );
            }
            ContractArtifactFields::MethodIdentifiers => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&to_value(&artifact.method_identifiers)?)?
                );
            }
            ContractArtifactFields::GasEstimates => {
                println!("{}", serde_json::to_string_pretty(&to_value(&artifact.gas_estimates)?)?);
            }
            ContractArtifactFields::StorageLayout => {
                print_storage_layout(&artifact.storage_layout, pretty)?;
            }
            ContractArtifactFields::DevDoc => {
                println!("{}", serde_json::to_string_pretty(&to_value(&artifact.devdoc)?)?);
            }
            ContractArtifactFields::Ir => {
                println!(
                    "{}",
                    to_value(&artifact.ir)?
                        .as_str()
                        .ok_or_else(|| eyre::eyre!("Failed to extract artifact ir as a string"))?
                );
            }
            ContractArtifactFields::IrOptimized => {
                println!(
                    "{}",
                    to_value(&artifact.ir_optimized)?.as_str().ok_or_else(|| eyre::eyre!(
                        "Failed to extract artifact optimized ir as a string"
                    ))?
                );
            }
            ContractArtifactFields::Metadata => {
                println!("{}", serde_json::to_string_pretty(&to_value(&artifact.metadata)?)?);
            }
            ContractArtifactFields::UserDoc => {
                println!("{}", serde_json::to_string_pretty(&to_value(&artifact.userdoc)?)?);
            }
            ContractArtifactFields::Ewasm => {
                println!(
                    "{}",
                    to_value(&artifact.ewasm)?.as_str().ok_or_else(|| eyre::eyre!(
                        "Failed to extract artifact ewasm as a string"
                    ))?
                );
            }
            ContractArtifactFields::Events => {
                let mut out = serde_json::Map::new();
                if let Some(LosslessAbi { abi, .. }) = artifact.abi.as_ref() {
                    let events: Vec<_> = abi.events.iter().flat_map(|(_, events)| events).collect();
                    // print the signature of all events including anonymous
                    for ev in events.iter() {
                        let types =
                            ev.inputs.iter().map(|p| p.kind.to_string()).collect::<Vec<_>>();
                        out.insert(
                            format!("{}({})", ev.name, types.join(",")),
                            format!("{:?}", ev.signature()).into(),
                        );
                    }
                }
                println!("{}", serde_json::to_string_pretty(&out)?);
            }
        };

        Ok(())
    }
}

pub fn print_storage_layout(
    storage_layout: &Option<StorageLayout>,
    pretty: bool,
) -> eyre::Result<()> {
    if storage_layout.is_none() {
        eyre::bail!("Could not get storage layout")
    }

    let storage_layout = storage_layout.as_ref().unwrap();

    if !pretty {
        println!("{}", serde_json::to_string_pretty(&to_value(storage_layout)?)?);
        return Ok(())
    }

    let mut table = Table::new();
    table.load_preset(ASCII_MARKDOWN);
    table.set_header(vec!["Name", "Type", "Slot", "Offset", "Bytes", "Contract"]);

    for slot in &storage_layout.storage {
        let storage_type = storage_layout.types.get(&slot.storage_type);
        table.add_row(vec![
            slot.label.clone(),
            storage_type.as_ref().map_or("?".to_string(), |t| t.label.clone()),
            slot.slot.clone(),
            slot.offset.to_string(),
            storage_type.as_ref().map_or("?".to_string(), |t| t.number_of_bytes.clone()),
            slot.contract.clone(),
        ]);
    }

    println!("{table}");

    Ok(())
}

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
    StorageLayout,
    DevDoc,
    Ir,
    IrOptimized,
    Metadata,
    UserDoc,
    Ewasm,
    Events,
}

// === impl ContractArtifactFields ===

impl ContractArtifactFields {
    /// Returns true if this field is generated by default
    pub fn is_default(&self) -> bool {
        matches!(self, ContractArtifactFields::Bytecode | ContractArtifactFields::DeployedBytecode)
    }
}

impl From<ContractArtifactFields> for ContractOutputSelection {
    fn from(field: ContractArtifactFields) -> Self {
        match field {
            ContractArtifactFields::Abi => ContractOutputSelection::Abi,
            ContractArtifactFields::Bytecode => ContractOutputSelection::Evm(
                EvmOutputSelection::ByteCode(BytecodeOutputSelection::All),
            ),
            ContractArtifactFields::DeployedBytecode => ContractOutputSelection::Evm(
                EvmOutputSelection::DeployedByteCode(DeployedBytecodeOutputSelection::All),
            ),
            ContractArtifactFields::Assembly | ContractArtifactFields::AssemblyOptimized => {
                ContractOutputSelection::Evm(EvmOutputSelection::Assembly)
            }
            ContractArtifactFields::MethodIdentifiers => {
                ContractOutputSelection::Evm(EvmOutputSelection::MethodIdentifiers)
            }
            ContractArtifactFields::GasEstimates => {
                ContractOutputSelection::Evm(EvmOutputSelection::GasEstimates)
            }
            ContractArtifactFields::StorageLayout => ContractOutputSelection::StorageLayout,
            ContractArtifactFields::DevDoc => ContractOutputSelection::DevDoc,
            ContractArtifactFields::Ir => ContractOutputSelection::Ir,
            ContractArtifactFields::IrOptimized => ContractOutputSelection::IrOptimized,
            ContractArtifactFields::Metadata => ContractOutputSelection::Metadata,
            ContractArtifactFields::UserDoc => ContractOutputSelection::UserDoc,
            ContractArtifactFields::Ewasm => {
                ContractOutputSelection::Ewasm(EwasmOutputSelection::All)
            }
            ContractArtifactFields::Events => ContractOutputSelection::Abi,
        }
    }
}

impl PartialEq<ContractOutputSelection> for ContractArtifactFields {
    fn eq(&self, other: &ContractOutputSelection) -> bool {
        self.to_string() == other.to_string()
    }
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
            ContractArtifactFields::StorageLayout => f.write_str("storageLayout"),
            ContractArtifactFields::DevDoc => f.write_str("devdoc"),
            ContractArtifactFields::Ir => f.write_str("ir"),
            ContractArtifactFields::IrOptimized => f.write_str("irOptimized"),
            ContractArtifactFields::Metadata => f.write_str("metadata"),
            ContractArtifactFields::UserDoc => f.write_str("userdoc"),
            ContractArtifactFields::Ewasm => f.write_str("ewasm"),
            ContractArtifactFields::Events => f.write_str("events"),
        }
    }
}

impl FromStr for ContractArtifactFields {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "abi" => Ok(ContractArtifactFields::Abi),
            "b" | "bytes" | "bytecode" => Ok(ContractArtifactFields::Bytecode),
            "deployedBytecode" | "deployed_bytecode" | "deployed-bytecode" | "deployed" |
            "deployedbytecode" => Ok(ContractArtifactFields::DeployedBytecode),
            "assembly" | "asm" => Ok(ContractArtifactFields::Assembly),
            "asmOptimized" | "assemblyOptimized" | "assemblyoptimized" | "assembly_optimized" |
            "asmopt" | "assembly-optimized" | "asmo" | "asm-optimized" | "asmoptimized" |
            "asm_optimized" => Ok(ContractArtifactFields::AssemblyOptimized),
            "methods" | "methodidentifiers" | "methodIdentifiers" | "method_identifiers" |
            "method-identifiers" | "mi" => Ok(ContractArtifactFields::MethodIdentifiers),
            "gasEstimates" | "gas" | "gas_estimates" | "gas-estimates" | "gasestimates" => {
                Ok(ContractArtifactFields::GasEstimates)
            }
            "storageLayout" | "storage_layout" | "storage-layout" | "storagelayout" | "storage" => {
                Ok(ContractArtifactFields::StorageLayout)
            }
            "devdoc" | "dev-doc" | "devDoc" => Ok(ContractArtifactFields::DevDoc),
            "ir" | "iR" | "IR" => Ok(ContractArtifactFields::Ir),
            "ir-optimized" | "irOptimized" | "iroptimized" | "iro" | "iropt" => {
                Ok(ContractArtifactFields::IrOptimized)
            }
            "metadata" | "meta" => Ok(ContractArtifactFields::Metadata),
            "userdoc" | "userDoc" | "user-doc" => Ok(ContractArtifactFields::UserDoc),
            "ewasm" | "e-wasm" => Ok(ContractArtifactFields::Ewasm),
            "events" | "ev" => Ok(ContractArtifactFields::Events),
            _ => Err(format!("Unknown field: {s}")),
        }
    }
}
