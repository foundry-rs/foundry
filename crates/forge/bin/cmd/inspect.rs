use clap::Parser;
use comfy_table::{presets::ASCII_MARKDOWN, Table};
use ethers::{
    abi::{ErrorExt, EventExt, RawAbi},
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
use eyre::Result;
use foundry_cli::opts::{CompilerArgs, CoreBuildArgs};
use foundry_common::{compile::ProjectCompiler, Shell};
use std::{collections::BTreeMap, fmt};
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

impl InspectArgs {
    pub fn run(self) -> Result<()> {
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
        let mut compiler = ProjectCompiler::new().quiet(true);
        if let Some(contract_path) = &mut contract.path {
            let target_path = canonicalize(&*contract_path)?;
            *contract_path = target_path.to_string_lossy().to_string();
            compiler = compiler.files([target_path]);
        }
        let output = compiler.compile(&project)?;

        // Find the artifact
        let artifact = output.find_contract(&contract).ok_or_else(|| {
            eyre::eyre!("Could not find artifact `{contract}` in the compiled artifacts")
        })?;

        // Match on ContractArtifactFields and pretty-print
        match field {
            ContractArtifactField::Abi => {
                let abi = artifact
                    .abi
                    .as_ref()
                    .ok_or_else(|| eyre::eyre!("Failed to fetch lossless ABI"))?;
                let abi_json = &abi.abi_value;
                if pretty {
                    let abi_json: RawAbi = serde_json::from_value(abi_json.clone())?;
                    let source: String = foundry_utils::abi::abi_to_solidity(&abi_json, "")?;
                    Shell::get().write_stdout(source, &Default::default())
                } else {
                    Shell::get().print_json(abi_json)
                }?;
            }
            ContractArtifactField::Bytecode => {
                print_json_str(&artifact.bytecode, Some("object"))?;
            }
            ContractArtifactField::DeployedBytecode => {
                print_json_str(&artifact.deployed_bytecode, Some("object"))?;
            }
            ContractArtifactField::Assembly | ContractArtifactField::AssemblyOptimized => {
                print_json(&artifact.assembly)?;
            }
            ContractArtifactField::MethodIdentifiers => {
                print_json(&artifact.method_identifiers)?;
            }
            ContractArtifactField::GasEstimates => {
                print_json(&artifact.gas_estimates)?;
            }
            ContractArtifactField::StorageLayout => {
                print_storage_layout(&artifact.storage_layout, pretty)?;
            }
            ContractArtifactField::DevDoc => {
                print_json(&artifact.devdoc)?;
            }
            ContractArtifactField::Ir => {
                print_json(&artifact.ir)?;
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
                let Some(LosslessAbi { abi, .. }) = &artifact.abi else {
                    return sh_println!("{{}}")
                };
                let map = abi
                    .errors()
                    .map(|error| (error.abi_signature(), hex::encode(error.selector())))
                    .collect::<BTreeMap<_, _>>();
                print_json(&map)?;
            }
            ContractArtifactField::Events => {
                let Some(LosslessAbi { abi, .. }) = &artifact.abi else {
                    return sh_println!("{{}}")
                };
                let map = abi
                    .events()
                    .map(|event| (event.abi_signature(), hex::encode(event.signature())))
                    .collect::<BTreeMap<_, _>>();
                print_json(&map)?;
            }
        };

        Ok(())
    }
}

pub fn print_storage_layout(storage_layout: &Option<StorageLayout>, pretty: bool) -> Result<()> {
    let Some(storage_layout) = storage_layout.as_ref() else {
        eyre::bail!("Could not get storage layout")
    };

    if !pretty {
        return print_json(&storage_layout)
    }

    let mut table = Table::new();
    table.load_preset(ASCII_MARKDOWN);
    table.set_header(vec!["Name", "Type", "Slot", "Offset", "Bytes", "Contract"]);

    for slot in &storage_layout.storage {
        let storage_type = storage_layout.types.get(&slot.storage_type);
        table.add_row(vec![
            slot.label.clone(),
            storage_type.as_ref().map_or_else(|| "?".into(), |t| t.label.clone()),
            slot.slot.clone(),
            slot.offset.to_string(),
            storage_type.as_ref().map_or_else(|| "?".into(), |t| t.number_of_bytes.clone()),
            slot.contract.clone(),
        ]);
    }

    Shell::get().write_stdout(table, &Default::default())
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
    Shell::get().print_json(obj)
}

fn print_json_str(obj: &impl serde::Serialize, key: Option<&str>) -> Result<()> {
    let value = serde_json::to_value(obj)?;
    let mut value_ref = &value;
    if let Some(key) = key {
        if let Some(value2) = value.get(key) {
            value_ref = value2;
        }
    }
    let s = value_ref.as_str().ok_or_else(|| eyre::eyre!("not a string: {value}"))?;
    sh_println!("{s}")
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
