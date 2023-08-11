use clap::Parser;
use comfy_table::{presets::ASCII_MARKDOWN, Table};
use ethers::{
    abi::RawAbi,
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
use foundry_cli::{
    opts::{CompilerArgs, CoreBuildArgs},
    utils::Cmd,
};
use foundry_common::compile;
use serde_json::{to_value, Value};
use std::fmt;
use tracing::trace;

/// CLI arguments for `forge inspect`.
#[derive(Debug, Clone, Parser)]
pub struct InspectArgs {
    /// The identifier of the contract to inspect in the form `(<path>:)?<contractname>`.
    pub contract: ContractInfo,

    /// The contract artifact field to inspect.
    #[clap(value_enum)]
    pub field: ContractArtifactField,

    /// Pretty print the selected field, if supported.
    #[clap(long)]
    pub pretty: bool,

    /// All build arguments are supported
    #[clap(flatten)]
    build: CoreBuildArgs,
}

impl Cmd for InspectArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let InspectArgs { mut contract, field, build, pretty } = self;

        trace!(target: "forge", ?field, ?contract, "running forge inspect");

        // Map field to ContractOutputSelection
        let mut cos = build.compiler.extra_output;
        if !field.is_default() && !cos.iter().any(|selected| field.eq(selected)) {
            cos.push(field.into());
        }

        // Run Optimized?
        let optimized = if let ContractArtifactField::AssemblyOptimized = field {
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

        trace!(target: "forge", artifact=?found_artifact, input=?contract, "Found contract");

        // Unwrap the inner artifact
        let artifact = found_artifact.ok_or_else(|| {
            eyre::eyre!("Could not find artifact `{contract}` in the compiled artifacts")
        })?;

        // Match on ContractArtifactFields and Pretty Print
        match field {
            ContractArtifactField::Abi => {
                let abi = artifact
                    .abi
                    .as_ref()
                    .ok_or_else(|| eyre::eyre!("Failed to fetch lossless ABI"))?;
                print_abi(abi, pretty)?;
            }
            ContractArtifactField::Bytecode => {
                let tval: Value = to_value(&artifact.bytecode)?;
                println!(
                    "{}",
                    tval.get("object").unwrap_or(&tval).as_str().ok_or_else(|| eyre::eyre!(
                        "Failed to extract artifact bytecode as a string"
                    ))?
                );
            }
            ContractArtifactField::DeployedBytecode => {
                let tval: Value = to_value(&artifact.deployed_bytecode)?;
                println!(
                    "{}",
                    tval.get("object").unwrap_or(&tval).as_str().ok_or_else(|| eyre::eyre!(
                        "Failed to extract artifact deployed bytecode as a string"
                    ))?
                );
            }
            ContractArtifactField::Assembly | ContractArtifactField::AssemblyOptimized => {
                println!(
                    "{}",
                    to_value(&artifact.assembly)?.as_str().ok_or_else(|| eyre::eyre!(
                        "Failed to extract artifact assembly as a string"
                    ))?
                );
            }
            ContractArtifactField::MethodIdentifiers => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&to_value(&artifact.method_identifiers)?)?
                );
            }
            ContractArtifactField::GasEstimates => {
                println!("{}", serde_json::to_string_pretty(&to_value(&artifact.gas_estimates)?)?);
            }
            ContractArtifactField::StorageLayout => {
                print_storage_layout(&artifact.storage_layout, pretty)?;
            }
            ContractArtifactField::DevDoc => {
                println!("{}", serde_json::to_string_pretty(&to_value(&artifact.devdoc)?)?);
            }
            ContractArtifactField::Ir => {
                println!(
                    "{}",
                    to_value(&artifact.ir)?
                        .as_str()
                        .ok_or_else(|| eyre::eyre!("Failed to extract artifact ir as a string"))?
                );
            }
            ContractArtifactField::IrOptimized => {
                println!(
                    "{}",
                    to_value(&artifact.ir_optimized)?.as_str().ok_or_else(|| eyre::eyre!(
                        "Failed to extract artifact optimized ir as a string"
                    ))?
                );
            }
            ContractArtifactField::Metadata => {
                println!("{}", serde_json::to_string_pretty(&to_value(&artifact.metadata)?)?);
            }
            ContractArtifactField::UserDoc => {
                println!("{}", serde_json::to_string_pretty(&to_value(&artifact.userdoc)?)?);
            }
            ContractArtifactField::Ewasm => {
                println!(
                    "{}",
                    to_value(&artifact.ewasm)?.as_str().ok_or_else(|| eyre::eyre!(
                        "Failed to extract artifact ewasm as a string"
                    ))?
                );
            }
            ContractArtifactField::Errors => {
                let mut out = serde_json::Map::new();
                if let Some(LosslessAbi { abi, .. }) = &artifact.abi {
                    // Print the signature of all errors
                    for er in abi.errors.iter().flat_map(|(_, errors)| errors) {
                        let types =
                            er.inputs.iter().map(|p| p.kind.to_string()).collect::<Vec<_>>();
                        let sig = format!("{:x}", er.signature());
                        let sig_trimmed = &sig[0..8];
                        out.insert(
                            format!("{}({})", er.name, types.join(",")),
                            sig_trimmed.to_string().into(),
                        );
                    }
                }
                println!("{}", serde_json::to_string_pretty(&out)?);
            }
            ContractArtifactField::Events => {
                let mut out = serde_json::Map::new();
                if let Some(LosslessAbi { abi, .. }) = &artifact.abi {
                    // print the signature of all events including anonymous
                    for ev in abi.events.iter().flat_map(|(_, events)| events) {
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

pub fn print_abi(abi: &LosslessAbi, pretty: bool) -> eyre::Result<()> {
    let abi_json = to_value(abi)?;
    if !pretty {
        println!("{}", serde_json::to_string_pretty(&abi_json)?);
        return Ok(())
    }

    let abi_json: RawAbi = serde_json::from_value(abi_json)?;
    let source = foundry_utils::abi::abi_to_solidity(&abi_json, "")?;
    println!("{}", source);

    Ok(())
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContractArtifactField {
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
    Errors,
    Events,
}

macro_rules! impl_value_enum {
    (enum $name:ident { $($field:ident => $main:literal $(| $alias:literal)*),+ $(,)? }) => {
        impl $name {
            /// All the variants of this enum.
            pub const ALL: &[Self] = &[$(Self::$field),+];

            /// Returns the string representation of `self`.
            #[inline]
            pub const fn as_str(&self) -> &'static str {
                match self {
                    $(
                        Self::$field => $main,
                    )+
                }
            }

            /// Returns all the aliases of `self`.
            #[inline]
            pub const fn aliases(&self) -> &'static [&'static str] {
                match self {
                    $(
                        Self::$field => &[$($alias),*],
                    )+
                }
            }
        }

        impl ::clap::ValueEnum for $name {
            #[inline]
            fn value_variants<'a>() -> &'a [Self] {
                Self::ALL
            }

            #[inline]
            fn to_possible_value(&self) -> Option<::clap::builder::PossibleValue> {
                Some(::clap::builder::PossibleValue::new(Self::as_str(self)).aliases(Self::aliases(self)))
            }

            #[inline]
            fn from_str(input: &str, ignore_case: bool) -> Result<Self, String> {
                let _ = ignore_case;
                <Self as ::std::str::FromStr>::from_str(input)
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $(
                        $main $(| $alias)* => Ok(Self::$field),
                    )+
                    _ => Err(format!(concat!("Invalid ", stringify!($name), " value: {}"), s)),
                }
            }
        }
    };
}

impl_value_enum! {
    enum ContractArtifactField {
        Abi               => "abi",
        Bytecode          => "bytecode" | "bytes" | "b",
        DeployedBytecode  => "deployedBytecode" | "deployed_bytecode" | "deployed-bytecode"
                             | "deployed" | "deployedbytecode",
        Assembly          => "assembly" | "asm",
        AssemblyOptimized => "assemblyOptimized" | "asmOptimized" | "assemblyoptimized"
                             | "assembly_optimized" | "asmopt" | "assembly-optimized"
                             | "asmo" | "asm-optimized" | "asmoptimized" | "asm_optimized",
        MethodIdentifiers => "methodIdentifiers" | "methodidentifiers" | "methods"
                             | "method_identifiers" | "method-identifiers" | "mi",
        GasEstimates      => "gasEstimates" | "gas" | "gas_estimates" | "gas-estimates"
                             | "gasestimates",
        StorageLayout     => "storageLayout" | "storage_layout" | "storage-layout"
                             | "storagelayout" | "storage",
        DevDoc            => "devdoc" | "dev-doc" | "devDoc",
        Ir                => "ir" | "iR" | "IR",
        IrOptimized       => "irOptimized" | "ir-optimized" | "iroptimized" | "iro" | "iropt",
        Metadata          => "metadata" | "meta",
        UserDoc           => "userdoc" | "userDoc" | "user-doc",
        Ewasm             => "ewasm" | "e-wasm",
        Errors            => "errors" | "er",
        Events            => "events" | "ev",
    }
}

impl From<ContractArtifactField> for ContractOutputSelection {
    fn from(field: ContractArtifactField) -> Self {
        type Caf = ContractArtifactField;
        match field {
            Caf::Abi => Self::Abi,
            Caf::Bytecode => Self::Evm(EvmOutputSelection::ByteCode(BytecodeOutputSelection::All)),
            Caf::DeployedBytecode => Self::Evm(EvmOutputSelection::DeployedByteCode(
                DeployedBytecodeOutputSelection::All,
            )),
            Caf::Assembly | Caf::AssemblyOptimized => Self::Evm(EvmOutputSelection::Assembly),
            Caf::MethodIdentifiers => Self::Evm(EvmOutputSelection::MethodIdentifiers),
            Caf::GasEstimates => Self::Evm(EvmOutputSelection::GasEstimates),
            Caf::StorageLayout => Self::StorageLayout,
            Caf::DevDoc => Self::DevDoc,
            Caf::Ir => Self::Ir,
            Caf::IrOptimized => Self::IrOptimized,
            Caf::Metadata => Self::Metadata,
            Caf::UserDoc => Self::UserDoc,
            Caf::Ewasm => Self::Ewasm(EwasmOutputSelection::All),
            Caf::Errors => Self::Abi,
            Caf::Events => Self::Abi,
        }
    }
}

impl PartialEq<ContractOutputSelection> for ContractArtifactField {
    fn eq(&self, other: &ContractOutputSelection) -> bool {
        type Cos = ContractOutputSelection;
        type Eos = EvmOutputSelection;
        matches!(
            (self, other),
            (Self::Abi | Self::Events, Cos::Abi) |
                (Self::Errors, Cos::Abi) |
                (Self::Bytecode, Cos::Evm(Eos::ByteCode(_))) |
                (Self::DeployedBytecode, Cos::Evm(Eos::DeployedByteCode(_))) |
                (Self::Assembly | Self::AssemblyOptimized, Cos::Evm(Eos::Assembly)) |
                (Self::MethodIdentifiers, Cos::Evm(Eos::MethodIdentifiers)) |
                (Self::GasEstimates, Cos::Evm(Eos::GasEstimates)) |
                (Self::StorageLayout, Cos::StorageLayout) |
                (Self::DevDoc, Cos::DevDoc) |
                (Self::Ir, Cos::Ir) |
                (Self::IrOptimized, Cos::IrOptimized) |
                (Self::Metadata, Cos::Metadata) |
                (Self::UserDoc, Cos::UserDoc) |
                (Self::Ewasm, Cos::Ewasm(_))
        )
    }
}

impl fmt::Display for ContractArtifactField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl ContractArtifactField {
    /// Returns true if this field is generated by default.
    pub const fn is_default(&self) -> bool {
        matches!(self, Self::Bytecode | Self::DeployedBytecode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_output_selection() {
        for &field in ContractArtifactField::ALL {
            let selection: ContractOutputSelection = field.into();
            assert_eq!(field, selection);

            let s = field.as_str();
            assert_eq!(s, field.to_string());
            assert_eq!(s.parse::<ContractArtifactField>().unwrap(), field);
            for alias in field.aliases() {
                assert_eq!(alias.parse::<ContractArtifactField>().unwrap(), field);
            }
        }
    }
}
