use alloy_json_abi::{EventParam, InternalType, JsonAbi, Param};
use alloy_primitives::{U256, hex, keccak256};
use clap::Parser;
use comfy_table::{Cell, Table, modifiers::UTF8_ROUND_CORNERS, presets::ASCII_MARKDOWN};
use eyre::{Result, eyre};
use forge_doc::{Comment, CommentTag, Comments, CommentsRef};
use foundry_cli::opts::{BuildOpts, CompilerOpts};
use foundry_common::{
    compile::{PathOrContractInfo, ProjectCompiler},
    find_matching_contract_artifact, find_target_path, shell,
};
use foundry_compilers::{
    artifacts::{
        StorageLayout,
        output_selection::{
            BytecodeOutputSelection, ContractOutputSelection, DeployedBytecodeOutputSelection,
            EvmOutputSelection, EwasmOutputSelection,
        },
    },
    solc::SolcLanguage,
};
use regex::Regex;
use serde_json::{Map, Value};
use std::{collections::BTreeMap, fmt, str::FromStr, sync::LazyLock};

/// CLI arguments for `forge inspect`.
#[derive(Clone, Debug, Parser)]
pub struct InspectArgs {
    /// The identifier of the contract to inspect in the form `(<path>:)?<contractname>`.
    #[arg(value_parser = PathOrContractInfo::from_str)]
    pub contract: PathOrContractInfo,

    /// The contract artifact field to inspect.
    #[arg(value_enum)]
    pub field: ContractArtifactField,

    /// All build arguments are supported
    #[command(flatten)]
    build: BuildOpts,

    /// Whether to remove comments when inspecting `ir` and `irOptimized` artifact fields.
    #[arg(long, short, help_heading = "Display options")]
    pub strip_yul_comments: bool,

    /// Whether to wrap the table to the terminal width.
    #[arg(long, short, help_heading = "Display options")]
    pub wrap: bool,
}

impl InspectArgs {
    pub fn run(self) -> Result<()> {
        let Self { contract, field, build, strip_yul_comments, wrap } = self;

        trace!(target: "forge", ?field, ?contract, "running forge inspect");

        // Map field to ContractOutputSelection
        let mut cos = build.compiler.extra_output;
        if !field.can_skip_field() && !cos.iter().any(|selected| field == *selected) {
            cos.push(field.try_into()?);
        }

        // Run Optimized?
        let optimized = if field == ContractArtifactField::AssemblyOptimized {
            Some(true)
        } else {
            build.compiler.optimize
        };

        // Get the solc version if specified
        let solc_version = build.use_solc.clone();

        // Build modified Args
        let modified_build_args = BuildOpts {
            compiler: CompilerOpts { extra_output: cos, optimize: optimized, ..build.compiler },
            ..build
        };

        // Build the project
        let project = modified_build_args.project()?;
        let compiler = ProjectCompiler::new().quiet(true);
        let target_path = find_target_path(&project, &contract)?;
        let mut output = compiler.files([target_path.clone()]).compile(&project)?;

        // Find the artifact
        let artifact = find_matching_contract_artifact(&mut output, &target_path, contract.name())?;

        // Match on ContractArtifactFields and pretty-print
        match field {
            ContractArtifactField::Abi => {
                let abi = artifact.abi.as_ref().ok_or_else(|| missing_error("ABI"))?;
                print_abi(abi, wrap)?;
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
            ContractArtifactField::LegacyAssembly => {
                print_json_str(&artifact.legacy_assembly, None)?;
            }
            ContractArtifactField::MethodIdentifiers => {
                print_method_identifiers(&artifact.method_identifiers, wrap)?;
            }
            ContractArtifactField::GasEstimates => {
                print_json(&artifact.gas_estimates)?;
            }
            ContractArtifactField::StorageLayout => {
                let namespaced_rows =
                    parse_storage_locations(artifact.raw_metadata.as_ref()).unwrap_or_default();
                print_storage_layout(artifact.storage_layout.as_ref(), namespaced_rows, wrap)?;
            }
            ContractArtifactField::DevDoc => {
                print_json(&artifact.devdoc)?;
            }
            ContractArtifactField::Ir => {
                print_yul(artifact.ir.as_deref(), strip_yul_comments)?;
            }
            ContractArtifactField::IrOptimized => {
                print_yul(artifact.ir_optimized.as_deref(), strip_yul_comments)?;
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
                let out = artifact.abi.as_ref().map_or(Map::new(), parse_errors);
                print_errors_events(&out, true, wrap)?;
            }
            ContractArtifactField::Events => {
                let out = artifact.abi.as_ref().map_or(Map::new(), parse_events);
                print_errors_events(&out, false, wrap)?;
            }
            ContractArtifactField::StandardJson => {
                let standard_json = if let Some(version) = solc_version {
                    let version = version.parse()?;
                    let mut standard_json =
                        project.standard_json_input(&target_path)?.normalize_evm_version(&version);
                    standard_json.settings.sanitize(&version, SolcLanguage::Solidity);
                    standard_json
                } else {
                    project.standard_json_input(&target_path)?
                };
                print_json(&standard_json)?;
            }
            ContractArtifactField::Libraries => {
                let all_libs: Vec<String> = artifact
                    .all_link_references()
                    .into_iter()
                    .flat_map(|(path, libs)| {
                        libs.into_keys().map(move |lib| format!("{path}:{lib}"))
                    })
                    .collect();
                if shell::is_json() {
                    return print_json(&all_libs);
                } else {
                    sh_println!(
                        "Dynamically linked libraries:\n{}",
                        all_libs
                            .iter()
                            .map(|v| format!("  {v}"))
                            .collect::<Vec<String>>()
                            .join("\n")
                    )?;
                }
            }
        };

        Ok(())
    }
}

fn parse_errors(abi: &JsonAbi) -> Map<String, Value> {
    let mut out = serde_json::Map::new();
    for er in abi.errors.iter().flat_map(|(_, errors)| errors) {
        let types = get_ty_sig(&er.inputs);
        let sig = format!("{:x}", er.selector());
        let sig_trimmed = &sig[0..8];
        out.insert(format!("{}({})", er.name, types), sig_trimmed.to_string().into());
    }
    out
}

fn parse_events(abi: &JsonAbi) -> Map<String, Value> {
    let mut out = serde_json::Map::new();
    for ev in abi.events.iter().flat_map(|(_, events)| events) {
        let types = parse_event_params(&ev.inputs);
        let topic = hex::encode(keccak256(ev.signature()));
        out.insert(format!("{}({})", ev.name, types), format!("0x{topic}").into());
    }
    out
}

fn parse_event_params(ev_params: &[EventParam]) -> String {
    ev_params
        .iter()
        .map(|p| {
            if let Some(ty) = p.internal_type() {
                return internal_ty(ty);
            }
            p.ty.clone()
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn print_abi(abi: &JsonAbi, should_wrap: bool) -> Result<()> {
    if shell::is_json() {
        return print_json(abi);
    }

    let headers = vec![Cell::new("Type"), Cell::new("Signature"), Cell::new("Selector")];
    print_table(
        headers,
        |table| {
            // Print events
            for ev in abi.events.iter().flat_map(|(_, events)| events) {
                let types = parse_event_params(&ev.inputs);
                let selector = ev.selector().to_string();
                table.add_row(["event", &format!("{}({})", ev.name, types), &selector]);
            }

            // Print errors
            for er in abi.errors.iter().flat_map(|(_, errors)| errors) {
                let selector = er.selector().to_string();
                table.add_row([
                    "error",
                    &format!("{}({})", er.name, get_ty_sig(&er.inputs)),
                    &selector,
                ]);
            }

            // Print functions
            for func in abi.functions.iter().flat_map(|(_, f)| f) {
                let selector = func.selector().to_string();
                let state_mut = func.state_mutability.as_json_str();
                let func_sig = if !func.outputs.is_empty() {
                    format!(
                        "{}({}) {state_mut} returns ({})",
                        func.name,
                        get_ty_sig(&func.inputs),
                        get_ty_sig(&func.outputs)
                    )
                } else {
                    format!("{}({}) {state_mut}", func.name, get_ty_sig(&func.inputs))
                };
                table.add_row(["function", &func_sig, &selector]);
            }

            if let Some(constructor) = abi.constructor() {
                let state_mut = constructor.state_mutability.as_json_str();
                table.add_row([
                    "constructor",
                    &format!("constructor({}) {state_mut}", get_ty_sig(&constructor.inputs)),
                    "",
                ]);
            }

            if let Some(fallback) = &abi.fallback {
                let state_mut = fallback.state_mutability.as_json_str();
                table.add_row(["fallback", &format!("fallback() {state_mut}"), ""]);
            }

            if let Some(receive) = &abi.receive {
                let state_mut = receive.state_mutability.as_json_str();
                table.add_row(["receive", &format!("receive() {state_mut}"), ""]);
            }
        },
        should_wrap,
    )
}

fn get_ty_sig(inputs: &[Param]) -> String {
    inputs
        .iter()
        .map(|p| {
            if let Some(ty) = p.internal_type() {
                return internal_ty(ty);
            }
            p.ty.clone()
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn internal_ty(ty: &InternalType) -> String {
    let contract_ty =
        |c: Option<&str>, ty: &String| c.map_or_else(|| ty.clone(), |c| format!("{c}.{ty}"));
    match ty {
        InternalType::AddressPayable(addr) => addr.clone(),
        InternalType::Contract(contract) => contract.clone(),
        InternalType::Enum { contract, ty } => contract_ty(contract.as_deref(), ty),
        InternalType::Struct { contract, ty } => contract_ty(contract.as_deref(), ty),
        InternalType::Other { contract, ty } => contract_ty(contract.as_deref(), ty),
    }
}

pub fn print_storage_layout(
    storage_layout: Option<&StorageLayout>,
    namespaced_rows: Vec<(String, String, String)>,
    should_wrap: bool,
) -> Result<()> {
    let Some(storage_layout) = storage_layout else {
        return Err(missing_error("storage layout"));
    };

    if shell::is_json() {
        return print_json(&storage_layout);
    }

    let headers = vec![
        Cell::new("Name"),
        Cell::new("Type"),
        Cell::new("Slot"),
        Cell::new("Offset"),
        Cell::new("Bytes"),
        Cell::new("Contract"),
    ];

    print_table(
        headers,
        |table| {
            for slot in &storage_layout.storage {
                let storage_type = storage_layout.types.get(&slot.storage_type);
                table.add_row([
                    slot.label.as_str(),
                    storage_type.map_or("?", |t| &t.label),
                    &slot.slot,
                    &slot.offset.to_string(),
                    storage_type.map_or("?", |t| &t.number_of_bytes),
                    &slot.contract,
                ]);
            }
            for (_, ns, slot_hex) in &namespaced_rows {
                table.add_row([
                    "",
                    ns.as_str(),
                    slot_hex.as_str(),
                    "0",
                    "32",
                    ns.split('.').last().unwrap_or(ns.as_str()),
                ]);
            }
        },
        should_wrap,
    )
}

fn print_method_identifiers(
    method_identifiers: &Option<BTreeMap<String, String>>,
    should_wrap: bool,
) -> Result<()> {
    let Some(method_identifiers) = method_identifiers else {
        return Err(missing_error("method identifiers"));
    };

    if shell::is_json() {
        return print_json(method_identifiers);
    }

    let headers = vec![Cell::new("Method"), Cell::new("Identifier")];

    print_table(
        headers,
        |table| {
            for (method, identifier) in method_identifiers {
                table.add_row([method, identifier]);
            }
        },
        should_wrap,
    )
}

fn print_errors_events(map: &Map<String, Value>, is_err: bool, should_wrap: bool) -> Result<()> {
    if shell::is_json() {
        return print_json(map);
    }

    let headers = if is_err {
        vec![Cell::new("Error"), Cell::new("Selector")]
    } else {
        vec![Cell::new("Event"), Cell::new("Topic")]
    };
    print_table(
        headers,
        |table| {
            for (method, selector) in map {
                table.add_row([method, selector.as_str().unwrap()]);
            }
        },
        should_wrap,
    )
}

fn print_table(
    headers: Vec<Cell>,
    add_rows: impl FnOnce(&mut Table),
    should_wrap: bool,
) -> Result<()> {
    let mut table = Table::new();
    if shell::is_markdown() {
        table.load_preset(ASCII_MARKDOWN);
    } else {
        table.apply_modifier(UTF8_ROUND_CORNERS);
    }
    table.set_header(headers);
    if should_wrap {
        table.set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
    }
    add_rows(&mut table);
    sh_println!("\n{table}\n")?;
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
    LegacyAssembly,
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
    StandardJson,
    Libraries,
}

macro_rules! impl_value_enum {
    (enum $name:ident { $($field:ident => $main:literal $(| $alias:literal)*),+ $(,)? }) => {
        impl $name {
            /// All the variants of this enum.
            pub const ALL: &'static [Self] = &[$(Self::$field),+];

            /// Returns the string representation of `self`.
            pub const fn as_str(&self) -> &'static str {
                match self {
                    $(
                        Self::$field => $main,
                    )+
                }
            }

            /// Returns all the aliases of `self`.
            pub const fn aliases(&self) -> &'static [&'static str] {
                match self {
                    $(
                        Self::$field => &[$($alias),*],
                    )+
                }
            }
        }

        impl ::clap::ValueEnum for $name {
            fn value_variants<'a>() -> &'a [Self] {
                Self::ALL
            }

            fn to_possible_value(&self) -> Option<::clap::builder::PossibleValue> {
                Some(::clap::builder::PossibleValue::new(Self::as_str(self)).aliases(Self::aliases(self)))
            }

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
        LegacyAssembly    => "legacyAssembly" | "legacyassembly" | "legacy_assembly",
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
        StandardJson      => "standardJson" | "standard-json" | "standard_json",
        Libraries         => "libraries" | "lib" | "libs",
    }
}

impl TryFrom<ContractArtifactField> for ContractOutputSelection {
    type Error = eyre::Error;

    fn try_from(field: ContractArtifactField) -> Result<Self, Self::Error> {
        type Caf = ContractArtifactField;
        match field {
            Caf::Abi => Ok(Self::Abi),
            Caf::Bytecode => {
                Ok(Self::Evm(EvmOutputSelection::ByteCode(BytecodeOutputSelection::All)))
            }
            Caf::DeployedBytecode => Ok(Self::Evm(EvmOutputSelection::DeployedByteCode(
                DeployedBytecodeOutputSelection::All,
            ))),
            Caf::Assembly | Caf::AssemblyOptimized => Ok(Self::Evm(EvmOutputSelection::Assembly)),
            Caf::LegacyAssembly => Ok(Self::Evm(EvmOutputSelection::LegacyAssembly)),
            Caf::MethodIdentifiers => Ok(Self::Evm(EvmOutputSelection::MethodIdentifiers)),
            Caf::GasEstimates => Ok(Self::Evm(EvmOutputSelection::GasEstimates)),
            Caf::StorageLayout => Ok(Self::StorageLayout),
            Caf::DevDoc => Ok(Self::DevDoc),
            Caf::Ir => Ok(Self::Ir),
            Caf::IrOptimized => Ok(Self::IrOptimized),
            Caf::Metadata => Ok(Self::Metadata),
            Caf::UserDoc => Ok(Self::UserDoc),
            Caf::Ewasm => Ok(Self::Ewasm(EwasmOutputSelection::All)),
            Caf::Errors => Ok(Self::Abi),
            Caf::Events => Ok(Self::Abi),
            Caf::StandardJson => {
                Err(eyre!("StandardJson is not supported for ContractOutputSelection"))
            }
            Caf::Libraries => Err(eyre!("Libraries is not supported for ContractOutputSelection")),
        }
    }
}

impl PartialEq<ContractOutputSelection> for ContractArtifactField {
    fn eq(&self, other: &ContractOutputSelection) -> bool {
        type Cos = ContractOutputSelection;
        type Eos = EvmOutputSelection;
        matches!(
            (self, other),
            (Self::Abi | Self::Events, Cos::Abi)
                | (Self::Errors, Cos::Abi)
                | (Self::Bytecode, Cos::Evm(Eos::ByteCode(_)))
                | (Self::DeployedBytecode, Cos::Evm(Eos::DeployedByteCode(_)))
                | (Self::Assembly | Self::AssemblyOptimized, Cos::Evm(Eos::Assembly))
                | (Self::LegacyAssembly, Cos::Evm(Eos::LegacyAssembly))
                | (Self::MethodIdentifiers, Cos::Evm(Eos::MethodIdentifiers))
                | (Self::GasEstimates, Cos::Evm(Eos::GasEstimates))
                | (Self::StorageLayout, Cos::StorageLayout)
                | (Self::DevDoc, Cos::DevDoc)
                | (Self::Ir, Cos::Ir)
                | (Self::IrOptimized, Cos::IrOptimized)
                | (Self::Metadata, Cos::Metadata)
                | (Self::UserDoc, Cos::UserDoc)
                | (Self::Ewasm, Cos::Ewasm(_))
        )
    }
}

impl fmt::Display for ContractArtifactField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl ContractArtifactField {
    /// Returns true if this field does not need to be passed to the compiler.
    pub const fn can_skip_field(&self) -> bool {
        matches!(
            self,
            Self::Bytecode | Self::DeployedBytecode | Self::StandardJson | Self::Libraries
        )
    }
}

fn print_json(obj: &impl serde::Serialize) -> Result<()> {
    sh_println!("{}", serde_json::to_string_pretty(obj)?)?;
    Ok(())
}

fn print_json_str(obj: &impl serde::Serialize, key: Option<&str>) -> Result<()> {
    sh_println!("{}", get_json_str(obj, key)?)?;
    Ok(())
}

fn print_yul(yul: Option<&str>, strip_comments: bool) -> Result<()> {
    let Some(yul) = yul else {
        return Err(missing_error("IR output"));
    };

    static YUL_COMMENTS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(///.*\n\s*)|(\s*/\*\*.*?\*/)").unwrap());

    if strip_comments {
        sh_println!("{}", YUL_COMMENTS.replace_all(yul, ""))?;
    } else {
        sh_println!("{yul}")?;
    }

    Ok(())
}

fn get_json_str(obj: &impl serde::Serialize, key: Option<&str>) -> Result<String> {
    let value = serde_json::to_value(obj)?;
    let value = if let Some(key) = key
        && let Some(value) = value.get(key)
    {
        value
    } else {
        &value
    };
    Ok(match value.as_str() {
        Some(s) => s.to_string(),
        None => format!("{value:#}"),
    })
}

fn missing_error(field: &str) -> eyre::Error {
    eyre!(
        "{field} missing from artifact; \
         this could be a spurious caching issue, consider running `forge clean`"
    )
}

#[inline]
fn compute_erc7201_slot_hex(ns: &str) -> String {
    // Step 1: keccak256(bytes(id))
    let ns_hash = keccak256(ns.as_bytes()); // 32 bytes

    // Step 2: (uint256(keccak256(id)) - 1) as 32-byte big-endian
    let mut u = U256::from_be_slice(ns_hash.as_slice());
    u = u.wrapping_sub(U256::from(1u8));
    let enc = u.to_be_bytes::<32>();

    // Step 3: keccak256(abi.encode(uint256(...)))
    let slot_hash = keccak256(enc);

    // Step 4: & ~0xff (zero out the lowest byte)
    let mut slot_u = U256::from_be_slice(slot_hash.as_slice());
    slot_u &= !U256::from(0xffu8);

    // 0x-prefixed 32-byte hex, optionally shorten with your helper
    let full = hex::encode_prefixed(slot_u.to_be_bytes::<32>());
    short_hex(&full)
}

// Simple “formula registry” so future EIPs can be added without touching the parser.
fn derive_slot_hex(formula: &str, ns: &str) -> Option<String> {
    match formula.to_ascii_lowercase().as_str() {
        "erc7201" => Some(compute_erc7201_slot_hex(ns)),
        // For future EIPs: add "erc1234" => Some(compute_erc1234_slot_hex(ns))
        _ => None,
    }
}

fn strings_from_json(val: &serde_json::Value) -> Vec<String> {
    match val {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(arr) => {
            arr.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()
        }
        _ => vec![],
    }
}

fn get_custom_tag_lines(devdoc: &serde_json::Value, key: &str) -> Vec<String> {
    if let Some(v) = devdoc.get(key) {
        let xs = strings_from_json(v);
        if !xs.is_empty() {
            return xs;
        }
    }
    devdoc
        .get("methods")
        .and_then(|m| m.get("constructor"))
        .and_then(|c| c.as_object())
        .and_then(|obj| obj.get(key))
        .map(strings_from_json)
        .unwrap_or_default()
}

pub fn parse_storage_locations(
    raw_metadata: Option<&String>,
) -> Option<Vec<(String, String, String)>> {
    let raw = raw_metadata?;
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let devdoc = v.get("output")?.get("devdoc")?;
    let loc_lines = get_custom_tag_lines(devdoc, "custom:storage-location");
    if loc_lines.is_empty() {
        return None;
    }
    let mut comments = Comments::default();
    for s in loc_lines {
        comments.push(Comment::new(CommentTag::Custom("storage-location".to_owned()), s));
    }
    let cref = CommentsRef::from(&comments);
    let out: Vec<(String, String, String)> = cref
        .storage_location_pairs()
        .into_iter()
        .filter_map(|(formula, ns)| {
            derive_slot_hex(&formula, &ns)
                .map(|slot_hex| (formula.to_ascii_lowercase(), ns, slot_hex))
        })
        .collect();
    if out.is_empty() { None } else { Some(out) }
}

fn short_hex(h: &str) -> String {
    let s = h.strip_prefix("0x").unwrap_or(h);
    if s.len() > 12 { format!("0x{}…{}", &s[..6], &s[s.len() - 4..]) } else { format!("0x{s}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_output_selection() {
        for &field in ContractArtifactField::ALL {
            if field == ContractArtifactField::StandardJson {
                let selection: Result<ContractOutputSelection, _> = field.try_into();
                assert!(
                    selection
                        .unwrap_err()
                        .to_string()
                        .eq("StandardJson is not supported for ContractOutputSelection")
                );
            } else if field == ContractArtifactField::Libraries {
                let selection: Result<ContractOutputSelection, _> = field.try_into();
                assert!(
                    selection
                        .unwrap_err()
                        .to_string()
                        .eq("Libraries is not supported for ContractOutputSelection")
                );
            } else {
                let selection: ContractOutputSelection = field.try_into().unwrap();
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

    #[test]
    fn parses_eip7201_storage_buckets_from_metadata() {
        let raw_wrapped = r#"
        {
            "metadata": {
                "compiler": { "version": "0.8.30+commit.73712a01" },
                "language": "Solidity",
                "output": {
                    "abi": [],
                    "devdoc": {
                        "kind": "dev",
                        "methods": {
                            "constructor": {
                                "custom:storage-location": "erc7201:openzeppelin.storage.ERC20erc7201:openzeppelin.storage.AccessControlDefaultAdminRules"
                            }
                        },
                        "version": 1
                    },
                    "userdoc": { "kind": "user", "methods": {}, "version": 1 }
                },
                "settings": { "optimizer": { "enabled": false, "runs": 200 } },
                "sources": {},
                "version": 1
            }
        }"#;

        let v: serde_json::Value = serde_json::from_str(raw_wrapped).unwrap();
        let inner_meta_str = v.get("metadata").unwrap().to_string();

        let rows = parse_storage_locations(Some(&inner_meta_str)).expect("parser returned None");
        assert_eq!(rows.len(), 2, "expected two EIP-7201 buckets");

        assert_eq!(rows[0].1, "openzeppelin.storage.ERC20");
        assert_eq!(rows[1].1, "openzeppelin.storage.AccessControlDefaultAdminRules");

        let expect_short = |h: &str| {
            let hex_str = h.trim_start_matches("0x");
            let slot = U256::from_str_radix(hex_str, 16).unwrap();
            let full = alloy_primitives::hex::encode_prefixed(slot.to_be_bytes::<32>());
            short_hex(&full)
        };

        let eip712_slot_hex =
            expect_short("0x52c63247e1f47db19d5ce0460030c497f067ca4cebf71ba98eeadabe20bace00");
        let nonces_slot_hex =
            expect_short("0xeef3dac4538c82c8ace4063ab0acd2d15cdb5883aa1dff7c2673abb3d8698400");

        assert_eq!(rows[0].2, eip712_slot_hex);
        assert_eq!(rows[1].2, nonces_slot_hex);

        assert!(rows[0].2.starts_with("0x") && rows[0].2.contains('…'));
        assert!(rows[1].2.starts_with("0x") && rows[1].2.contains('…'));
    }
}
