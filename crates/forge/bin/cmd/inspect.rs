use alloy_json_abi::JsonAbi;
use clap::Parser;
use comfy_table::{presets::ASCII_MARKDOWN, Table};
use eyre::Result;
use foundry_cli::opts::{CompilerArgs, CoreBuildArgs};
use foundry_common::compile::ProjectCompiler;
use foundry_compilers::{
    artifacts::{
        output_selection::{
            BytecodeOutputSelection, ContractOutputSelection, DeployedBytecodeOutputSelection,
            EvmOutputSelection, EwasmOutputSelection,
        },
        StorageLayout,
    },
    info::ContractInfo,
    utils::canonicalize,
    ConfigurableContractArtifact,
};
use std::{collections::BTreeMap, fmt};

/// CLI arguments for `forge inspect`.
#[derive(Clone, Debug, Parser)]
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

impl InspectArgs {
    pub fn run(self) -> Result<()> {
        let InspectArgs { mut contract, field, build, pretty } = self;

        trace!(target: "forge", ?field, ?contract, "running forge inspect");

        // Map field to ContractOutputSelection
        let mut cos = build.compiler.extra_output;
        if !field.is_default() && !cos.iter().any(|selected| field == *selected) {
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
        let mut compiler = ProjectCompiler::new().quiet(true);
        if let Some(contract_path) = &mut contract.path {
            let target_path = canonicalize(&*contract_path)?;
            *contract_path = target_path.to_string_lossy().to_string();
            compiler = compiler.files([target_path]);
        }
        let output = compiler.compile(&project)?;

        let is_project_level = contract.name == "." &&
            matches!(
                field,
                ContractArtifactField::Events |
                    ContractArtifactField::Errors |
                    ContractArtifactField::MethodIdentifiers
            );

        let project_level_artifact_dummy = if is_project_level {
            let mut all_method_identifiers = BTreeMap::<String, String>::new();
            let mut all_events = BTreeMap::new();
            let mut all_errors = BTreeMap::new();

            output.artifacts().for_each(|(_, artifact)| {
                artifact.method_identifiers.iter().for_each(|method_identifiers| {
                    all_method_identifiers.extend(method_identifiers.clone());
                });
                artifact.abi.iter().for_each(|abi| {
                    all_events.extend(abi.events.clone());
                    all_errors.extend(abi.errors.clone());
                });
            });
            let mut json_abi = JsonAbi::new();
            json_abi.errors = all_errors;
            json_abi.events = all_events;

            Some(ConfigurableContractArtifact {
                abi: Some(json_abi),
                bytecode: None,
                deployed_bytecode: None,
                assembly: None,
                method_identifiers: Some(all_method_identifiers),
                gas_estimates: None,
                storage_layout: None,
                devdoc: None,
                ir: None,
                ir_optimized: None,
                metadata: None,
                userdoc: None,
                ewasm: None,
                opcodes: None,
                generated_sources: vec![],
                function_debug_data: None,
                raw_metadata: None,
                ast: None,
                id: None,
            })
        } else {
            None
        };

        // Find the artifact
        let artifact = if is_project_level {
            let inner = project_level_artifact_dummy.as_ref().unwrap();
            inner
        } else {
            output.find_contract(&contract).ok_or_else(|| {
                eyre::eyre!("Could not find artifact `{contract}` in the compiled artifacts")
            })?
        };

        // Match on ContractArtifactFields and pretty-print
        match field {
            ContractArtifactField::Abi => {
                let abi = artifact
                    .abi
                    .as_ref()
                    .ok_or_else(|| eyre::eyre!("Failed to fetch lossless ABI"))?;
                if pretty {
                    let source = foundry_cli::utils::abi_to_solidity(abi, &contract.name)?;
                    println!("{source}");
                } else {
                    print_json(abi)?;
                }
            }
            ContractArtifactField::Bytecode => {
                print_json_str(&artifact.bytecode, Some("object"))?;
            }
            ContractArtifactField::DeployedBytecode => {
                print_json_str(&artifact.deployed_bytecode, Some("object"))?;
            }
            ContractArtifactField::Assembly | ContractArtifactField::AssemblyOptimized => {
                print_json_str(&artifact.assembly, None)?;
            }
            ContractArtifactField::MethodIdentifiers => {
                print_json(&artifact.method_identifiers)?;
            }
            ContractArtifactField::GasEstimates => {
                print_json(&artifact.gas_estimates)?;
            }
            ContractArtifactField::StorageLayout => {
                print_storage_layout(artifact.storage_layout.as_ref(), pretty)?;
            }
            ContractArtifactField::DevDoc => {
                print_json(&artifact.devdoc)?;
            }
            ContractArtifactField::Ir => {
                print_json_str(&artifact.ir, None)?;
            }
            ContractArtifactField::IrOptimized => {
                print_json_str(&artifact.ir_optimized, None)?;
            }
            ContractArtifactField::Metadata => {
                print_json(&artifact.metadata)?;
            }
            ContractArtifactField::UserDoc => {
                print_json(&artifact.userdoc)?;
            }
            ContractArtifactField::Ewasm => {
                print_json_str(&artifact.ewasm, None)?;
            }
            ContractArtifactField::Errors => {
                let mut out = serde_json::Map::new();
                if let Some(abi) = &artifact.abi {
                    let abi = &abi;
                    // Print the signature of all errors
                    for er in abi.errors.iter().flat_map(|(_, errors)| errors) {
                        let types = er.inputs.iter().map(|p| p.ty.clone()).collect::<Vec<_>>();
                        let sig = format!("{:x}", er.selector());
                        let sig_trimmed = &sig[0..8];
                        out.insert(
                            format!("{}({})", er.name, types.join(",")),
                            sig_trimmed.to_string().into(),
                        );
                    }
                }
                print_json(&out)?;
            }
            ContractArtifactField::Events => {
                let mut out = serde_json::Map::new();
                if let Some(abi) = &artifact.abi {
                    let abi = &abi;

                    // print the signature of all events including anonymous
                    for ev in abi.events.iter().flat_map(|(_, events)| events) {
                        let types = ev.inputs.iter().map(|p| p.ty.clone()).collect::<Vec<_>>();
                        out.insert(
                            format!("{}({})", ev.name, types.join(",")),
                            format!("{:?}", ev.signature()).into(),
                        );
                    }
                }
                print_json(&out)?;
            }
        };

        Ok(())
    }
}

pub fn print_storage_layout(storage_layout: Option<&StorageLayout>, pretty: bool) -> Result<()> {
    let Some(storage_layout) = storage_layout else {
        eyre::bail!("Could not get storage layout");
    };

    if !pretty {
        return print_json(&storage_layout)
    }

    let mut table = Table::new();
    table.load_preset(ASCII_MARKDOWN);
    table.set_header(["Name", "Type", "Slot", "Offset", "Bytes", "Contract"]);

    for slot in &storage_layout.storage {
        let storage_type = storage_layout.types.get(&slot.storage_type);
        table.add_row([
            slot.label.as_str(),
            storage_type.map_or("?", |t| &t.label),
            &slot.slot,
            &slot.offset.to_string(),
            &storage_type.map_or("?", |t| &t.number_of_bytes),
            &slot.contract,
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
            pub const ALL: &'static [Self] = &[$(Self::$field),+];

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

fn print_json(obj: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(obj)?);
    Ok(())
}

fn print_json_str(obj: &impl serde::Serialize, key: Option<&str>) -> Result<()> {
    let value = serde_json::to_value(obj)?;
    let mut value_ref = &value;
    if let Some(key) = key {
        if let Some(value2) = value.get(key) {
            value_ref = value2;
        }
    }
    match value_ref.as_str() {
        Some(s) => println!("{s}"),
        None => println!("{value_ref:#}"),
    }
    Ok(())
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
