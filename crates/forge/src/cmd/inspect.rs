use alloy_json_abi::{Event, EventParam, InternalType, JsonAbi, Param};
use alloy_primitives::U256;
use clap::Parser;
use comfy_table::{Cell, Table, modifiers::UTF8_ROUND_CORNERS, presets::ASCII_MARKDOWN};
use eyre::{Result, eyre};
use foundry_cli::opts::{BuildOpts, CompilerOpts};
use foundry_common::{
    compile::{PathOrContractInfo, ProjectCompiler},
    erc7201, find_matching_contract_artifact, find_target_path, shell,
};
use foundry_compilers::{
    ProjectCompileOutput,
    artifacts::{
        Storage, StorageLayout, StorageType,
        output_selection::{
            BytecodeOutputSelection, ContractOutputSelection, DeployedBytecodeOutputSelection,
            EvmOutputSelection, EwasmOutputSelection,
        },
    },
    solc::SolcLanguage,
};
use path_slash::PathExt;
use regex::Regex;
use serde_json::{Map, Value};
use solar::{
    ast::LitKind,
    sema::{
        hir::{ElementaryType, ExprKind, Hir, ItemId, NatSpecKind, TypeKind},
        interface::source_map::FileName,
    },
};
use std::{collections::BTreeMap, fmt, ops::ControlFlow, path::Path, str::FromStr, sync::LazyLock};

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
        let target_path = find_target_path(&project, &contract)?;
        if field == ContractArtifactField::Linearization && !is_solidity_source(&target_path) {
            eyre::bail!(
                "linearization inspection is only supported for Solidity contracts (.sol targets)"
            );
        }
        let compiler = ProjectCompiler::new().quiet(true);
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
                let mut layout =
                    artifact.storage_layout.ok_or_else(|| missing_error("storage layout"))?;
                if is_solidity_source(&target_path) {
                    let (entries, types) =
                        collect_erc7201_entries(&mut output, &target_path, contract.name())?;
                    layout.storage.extend(entries);
                    layout.types.extend(types);
                }
                print_storage_layout(Some(&layout), wrap)?;
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
                }
                sh_status!("Dynamically linked libraries:")?;
                for lib in &all_libs {
                    sh_println!("{lib}")?;
                }
            }
            ContractArtifactField::Linearization => {
                print_linearization(
                    &mut output,
                    project.root(),
                    &target_path,
                    contract.name(),
                    wrap,
                )?;
            }
        };

        Ok(())
    }
}

fn parse_errors(abi: &JsonAbi) -> Map<String, Value> {
    let mut out = serde_json::Map::new();
    for er in abi.errors.values().flatten() {
        let types = get_ty_sig(&er.inputs);
        let sig = format!("{:x}", er.selector());
        let sig_trimmed = &sig[0..8];
        out.insert(format!("{}({})", er.name, types), sig_trimmed.to_string().into());
    }
    out
}

fn parse_events(abi: &JsonAbi) -> Map<String, Value> {
    let mut out = serde_json::Map::new();
    for ev in abi.events.values().flatten() {
        let types = parse_event_params(&ev.inputs);
        let topic = event_topic(ev).map_or(Value::Null, Into::into);
        out.insert(format!("{}({})", ev.name, types), topic);
    }
    out
}

/// Returns topic0 for non-anonymous events. Anonymous events have no signature topic.
fn event_topic(ev: &Event) -> Option<String> {
    (!ev.anonymous).then(|| ev.selector().to_string())
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
            for ev in abi.events.values().flatten() {
                let types = parse_event_params(&ev.inputs);
                let signature = if ev.anonymous {
                    format!("{}({}) anonymous", ev.name, types)
                } else {
                    format!("{}({})", ev.name, types)
                };
                let selector = event_topic(ev).unwrap_or_default();
                table.add_row(["event", &signature, &selector]);
            }

            // Print errors
            for er in abi.errors.values().flatten() {
                let selector = er.selector().to_string();
                table.add_row([
                    "error",
                    &format!("{}({})", er.name, get_ty_sig(&er.inputs)),
                    &selector,
                ]);
            }

            // Print functions
            for func in abi.functions.values().flatten() {
                let selector = func.selector().to_string();
                let state_mut = func.state_mutability.as_json_str();
                let func_sig = if func.outputs.is_empty() {
                    format!("{}({}) {state_mut}", func.name, get_ty_sig(&func.inputs))
                } else {
                    format!(
                        "{}({}) {state_mut} returns ({})",
                        func.name,
                        get_ty_sig(&func.inputs),
                        get_ty_sig(&func.outputs)
                    )
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
                table.add_row([method.as_str(), identifier.as_str()]);
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
                table.add_row([method.as_str(), selector.as_str().unwrap_or("")]);
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

/// Returns `(label, number_of_bytes, slot_count, encoding)` for a HIR type in storage context.
///
/// - `number_of_bytes`: actual data bytes (used in `StorageType.numberOfBytes`)
/// - `slot_count`: 0 for packable value types; ≥ 1 for slot-boundary types (arrays, structs,
///   mappings, string, bytes). The packing algorithm advances `current_slot` by this amount.
fn hir_type_storage_info<'hir>(
    hir: &Hir<'hir>,
    kind: &TypeKind<'hir>,
) -> (String, u64, u64, &'static str) {
    match kind {
        TypeKind::Elementary(et) => match et {
            ElementaryType::Address(_) => ("address".to_string(), 20, 0, "inplace"),
            ElementaryType::Bool => ("bool".to_string(), 1, 0, "inplace"),
            ElementaryType::String => ("string".to_string(), 32, 1, "bytes"),
            ElementaryType::Bytes => ("bytes".to_string(), 32, 1, "bytes"),
            ElementaryType::Int(size) => {
                (format!("int{}", size.bits()), size.bytes() as u64, 0, "inplace")
            }
            ElementaryType::UInt(size) => {
                (format!("uint{}", size.bits()), size.bytes() as u64, 0, "inplace")
            }
            ElementaryType::FixedBytes(size) => {
                (format!("bytes{}", size.bytes()), size.bytes() as u64, 0, "inplace")
            }
            ElementaryType::Fixed(m, n) => {
                (format!("fixed{}x{}", m.bits(), n.get()), m.bytes() as u64, 0, "inplace")
            }
            ElementaryType::UFixed(m, n) => {
                (format!("ufixed{}x{}", m.bits(), n.get()), m.bytes() as u64, 0, "inplace")
            }
        },
        TypeKind::Array(arr) => {
            let (elem_label, elem_bytes, elem_slots, _) =
                hir_type_storage_info(hir, &arr.element.kind);
            // Try to evaluate a literal array size for precise slot counting.
            let fixed_len: Option<u64> = arr.size.and_then(|size_expr| {
                if let ExprKind::Lit(lit) = size_expr.kind {
                    if let LitKind::Number(n) = lit.kind { n.try_into().ok() } else { None }
                } else {
                    None
                }
            });
            match fixed_len {
                Some(n) if n > 0 => {
                    let label = format!("{elem_label}[{n}]");
                    let (number_of_bytes, slot_count) = if elem_slots == 0 {
                        // Packable element: compute tight packing.
                        // elements_per_slot = floor(32 / elem_bytes), minimum 1.
                        let per_slot = (32u64 / elem_bytes).max(1);
                        let slots = n.div_ceil(per_slot);
                        (n * elem_bytes, slots)
                    } else {
                        // Slot-boundary element (e.g. T is itself an array or struct).
                        let slots = n * elem_slots;
                        (slots * 32, slots)
                    };
                    (label, number_of_bytes, slot_count, "inplace")
                }
                // Dynamic array or unresolvable size: 1 slot base.
                _ => (format!("{elem_label}[]"), 32, 1, "dynamic_array"),
            }
        }
        TypeKind::Mapping(m) => {
            let (key_label, ..) = hir_type_storage_info(hir, &m.key.kind);
            let (val_label, ..) = hir_type_storage_info(hir, &m.value.kind);
            (format!("mapping({key_label} => {val_label})"), 32, 1, "mapping")
        }
        TypeKind::Custom(ItemId::Struct(id)) => {
            let s = hir.strukt(*id);
            let label = if let Some(cid) = s.contract {
                format!("struct {}.{}", hir.contract(cid).name.as_str(), s.name.as_str())
            } else {
                format!("struct {}", s.name.as_str())
            };
            // Recursively compute the struct's slot count via the same packing rules.
            let slot_count = struct_slot_count(hir, s.fields);
            (label, slot_count * 32, slot_count, "inplace")
        }
        TypeKind::Custom(ItemId::Enum(id)) => {
            let e = hir.enumm(*id);
            let label = if let Some(cid) = e.contract {
                format!("enum {}.{}", hir.contract(cid).name.as_str(), e.name.as_str())
            } else {
                format!("enum {}", e.name.as_str())
            };
            (label, 1, 0, "inplace")
        }
        TypeKind::Custom(ItemId::Udvt(id)) => {
            let u = hir.udvt(*id);
            let (_, bytes, slots, encoding) = hir_type_storage_info(hir, &u.ty.kind);
            let label = if let Some(cid) = u.contract {
                format!("{}.{}", hir.contract(cid).name.as_str(), u.name.as_str())
            } else {
                u.name.as_str().to_string()
            };
            (label, bytes, slots, encoding)
        }
        TypeKind::Function(_) => ("function".to_string(), 24, 0, "inplace"),
        TypeKind::Custom(_) | TypeKind::Err(_) => ("unknown".to_string(), 32, 1, "inplace"),
    }
}

/// Computes the number of 32-byte slots consumed by a sequence of struct fields.
fn struct_slot_count<'hir>(hir: &Hir<'hir>, fields: &[solar::sema::hir::VariableId]) -> u64 {
    let mut current_slot: u64 = 0;
    let mut current_offset: u64 = 0;
    for &var_id in fields {
        let var = hir.variable(var_id);
        let (_, byte_size, slot_count, _) = hir_type_storage_info(hir, &var.ty.kind);
        if slot_count > 0 {
            if current_offset > 0 {
                current_slot += 1;
                current_offset = 0;
            }
            current_slot += slot_count;
        } else {
            if current_offset + byte_size > 32 {
                current_slot += 1;
                current_offset = 0;
            }
            current_offset += byte_size;
        }
    }
    // Any partially-filled final slot counts as a full slot.
    if current_offset > 0 { current_slot + 1 } else { current_slot }
}

/// Collects ERC-7201 namespaced storage entries for the target contract using Solar HIR.
///
/// Scans all structs annotated with `@custom:storage-location erc7201:<namespace>` in the
/// target contract's linearization chain, computes their base slot via [`erc7201`], and
/// synthesises [`Storage`] and [`StorageType`] entries using Solidity's packing rules.
fn collect_erc7201_entries(
    output: &mut ProjectCompileOutput,
    target_path: &Path,
    target_name: Option<&str>,
) -> Result<(Vec<Storage>, BTreeMap<String, StorageType>)> {
    let mut entries: Vec<Storage> = Vec::new();
    let mut types: BTreeMap<String, StorageType> = BTreeMap::new();

    let mut lowered = false;
    let compiler = output.parser_mut().solc_mut().compiler_mut();
    compiler.enter_mut(|compiler| -> Result<()> {
        let Ok(ControlFlow::Continue(())) = compiler.lower_asts() else { return Ok(()) };
        lowered = true;

        let gcx = compiler.gcx();
        let hir = &gcx.hir;

        // Locate the target contract.
        let matching: Vec<_> = hir
            .contract_ids()
            .filter(|id| {
                let c = hir.contract(*id);
                if let Some(name) = target_name
                    && c.name.as_str() != name
                {
                    return false;
                }
                matches!(&hir.source(c.source).file.name, FileName::Real(p) if p == target_path)
            })
            .collect();

        let target_id = match matching.as_slice() {
            [id] => *id,
            _ => return Ok(()),
        };

        let linearized_bases: Vec<_> = hir.contract(target_id).linearized_bases.to_vec();
        let target_contract_name = hir.contract(target_id).name.as_str().to_string();

        // Walk every struct in the HIR; keep those that belong to a contract in the
        // linearization chain and carry an @custom:storage-location erc7201:<ns> annotation.
        for struct_id in hir.strukt_ids() {
            let strukt = hir.strukt(struct_id);

            let Some(struct_contract_id) = strukt.contract else { continue };
            if !linearized_bases.contains(&struct_contract_id) {
                continue;
            }
            if strukt.doc.is_empty() {
                continue;
            }

            let docs = gcx.natspec_doc_comments(strukt.doc);
            let namespace = docs.iter().find_map(|item| {
                if let NatSpecKind::Custom { name } = item.kind
                    && name.name.as_str() == "storage-location"
                {
                    item.content().trim().strip_prefix("erc7201:")
                } else {
                    None
                }
            });

            let Some(namespace) = namespace else { continue };

            let base_slot = U256::from_be_bytes(erc7201(namespace).0);
            let contract_label = format!("{target_contract_name} [erc7201:{namespace}]");

            // Assign slots using Solidity's packing rules.
            let mut current_slot: u64 = 0;
            let mut current_offset: u64 = 0; // bytes used in current slot (low-order)

            for &var_id in strukt.fields {
                let var = hir.variable(var_id);
                let field_name = var.name.map(|n| n.name.as_str().to_string()).unwrap_or_default();
                let (type_label, byte_size, slot_count, encoding) =
                    hir_type_storage_info(hir, &var.ty.kind);

                let (field_slot, field_offset) = if slot_count > 0 {
                    // Slot-boundary type: align to a fresh slot, then consume slot_count slots.
                    if current_offset > 0 {
                        current_slot += 1;
                        current_offset = 0;
                    }
                    let s = current_slot;
                    current_slot += slot_count;
                    (s, 0u64)
                } else {
                    // Packable value type: fit into current slot or advance.
                    if current_offset + byte_size > 32 {
                        current_slot += 1;
                        current_offset = 0;
                    }
                    let s = current_slot;
                    let o = current_offset;
                    current_offset += byte_size;
                    (s, o)
                };

                let slot_value = base_slot + U256::from(field_slot);
                let slot_str = format!("{slot_value:#066x}");

                entries.push(Storage {
                    ast_id: 0,
                    contract: contract_label.clone(),
                    label: field_name,
                    offset: field_offset as i64,
                    slot: slot_str,
                    storage_type: type_label.clone(),
                });

                types.entry(type_label.clone()).or_insert_with(|| StorageType {
                    encoding: encoding.to_string(),
                    key: None,
                    label: type_label,
                    number_of_bytes: byte_size.to_string(),
                    value: None,
                    other: BTreeMap::new(),
                });
            }
        }

        Ok(())
    })?;

    let _ = lowered;
    Ok((entries, types))
}

fn print_linearization(
    output: &mut ProjectCompileOutput,
    root: &Path,
    target_path: &Path,
    target_name: Option<&str>,
    should_wrap: bool,
) -> Result<()> {
    let mut chain = Vec::new();
    let mut lowered = false;
    let compiler = output.parser_mut().solc_mut().compiler_mut();
    compiler.enter_mut(|compiler| -> Result<()> {
        let Ok(ControlFlow::Continue(())) = compiler.lower_asts() else { return Ok(()) };
        lowered = true;

        let hir = &compiler.gcx().hir;
        let matching_contracts = hir
            .contract_ids()
            .filter(|id| {
                let contract = hir.contract(*id);
                if let Some(target_name) = target_name
                    && contract.name.as_str() != target_name
                {
                    return false;
                }

                matches!(
                    &hir.source(contract.source).file.name,
                    FileName::Real(path) if path == target_path
                )
            })
            .collect::<Vec<_>>();

        let target_contract = match matching_contracts.as_slice() {
            [id] => *id,
            [] => {
                if let Some(target_name) = target_name {
                    eyre::bail!(
                        "Could not find contract `{target_name}` in `{}`",
                        target_path.display()
                    );
                }
                eyre::bail!("Could not find contract in `{}`", target_path.display());
            }
            _ => {
                eyre::bail!(
                    "Multiple contracts found in the same file, please specify the target <path>:<contract> or <contract>"
                );
            }
        };

        for (order, base_id) in hir.contract(target_contract).linearized_bases.iter().enumerate() {
            let contract = hir.contract(*base_id);
            let source = hir.source(contract.source);
            let FileName::Real(path) = &source.file.name else { continue };
            let path = path.strip_prefix(root).unwrap_or(path);
            chain.push((
                order,
                path.to_slash_lossy().into_owned(),
                contract.name.as_str().to_string(),
            ));
        }

        Ok(())
    })?;

    // `compiler.sess()` inside of `ProjectCompileOutput` is built with `with_buffer_emitter`.
    let diags = compiler.sess().dcx.emitted_diagnostics().unwrap();
    if compiler.sess().dcx.has_errors().is_err() {
        eyre::bail!("{diags}");
    } else {
        let _ = sh_eprint!("{diags}");
    }
    if !lowered {
        eyre::bail!(
            "unable to inspect linearization: failed to lower Solidity ASTs for `{}`",
            target_path.display()
        );
    }

    if shell::is_json() {
        let contracts = chain
            .into_iter()
            .map(|(order, source, contract)| {
                serde_json::json!({
                    "order": order,
                    "source": source,
                    "contract": contract,
                })
            })
            .collect::<Vec<_>>();
        return print_json(&contracts);
    }

    let headers = vec![Cell::new("Order"), Cell::new("Source"), Cell::new("Contract")];
    print_table(
        headers,
        |table| {
            for (order, source, contract) in &chain {
                table.add_row([order.to_string(), source.clone(), contract.clone()]);
            }
        },
        should_wrap,
    )
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
    Linearization,
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
        Linearization     => "linearization" | "linearizedInheritance"
                             | "linearized-inheritance" | "linearized_inheritance"
                             | "linearizedBases" | "linearized-bases" | "linearized_bases",
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
            Caf::Linearization => {
                Err(eyre!("Linearization is not supported for ContractOutputSelection"))
            }
        }
    }
}

impl PartialEq<ContractOutputSelection> for ContractArtifactField {
    fn eq(&self, other: &ContractOutputSelection) -> bool {
        type Cos = ContractOutputSelection;
        type Eos = EvmOutputSelection;
        matches!(
            (self, other),
            (Self::Abi | Self::Events | Self::Errors, Cos::Abi)
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
            Self::Bytecode
                | Self::DeployedBytecode
                | Self::StandardJson
                | Self::Libraries
                | Self::Linearization
        )
    }
}

fn print_json(obj: &impl serde::Serialize) -> Result<()> {
    sh_println!("{}", serde_json::to_string_pretty(obj)?)?;
    Ok(())
}

fn print_json_str(obj: &impl serde::Serialize, key: Option<&str>) -> Result<()> {
    let value = serde_json::to_value(obj)?;
    let value = key.and_then(|k| value.get(k)).unwrap_or(&value);
    if shell::is_json() {
        sh_println!("{}", serde_json::to_string_pretty(value)?)?;
    } else {
        let s = match value.as_str() {
            Some(s) => s.to_string(),
            None => format!("{value:#}"),
        };
        sh_println!("{s}")?;
    }
    Ok(())
}

fn print_yul(yul: Option<&str>, strip_comments: bool) -> Result<()> {
    let Some(yul) = yul else {
        return Err(missing_error("IR output"));
    };

    static YUL_COMMENTS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(///.*\n\s*)|(\s*/\*\*.*?\*/)").unwrap());

    let out = if strip_comments {
        YUL_COMMENTS.replace_all(yul, "").into_owned()
    } else {
        yul.to_string()
    };

    if shell::is_json() {
        sh_println!("{}", serde_json::to_string(&out)?)?;
    } else {
        sh_println!("{out}")?;
    }

    Ok(())
}

fn is_solidity_source(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("sol"))
}

fn missing_error(field: &str) -> eyre::Error {
    eyre!(
        "{field} missing from artifact; \
         this could be a spurious caching issue, consider running `forge clean`"
    )
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
            } else if field == ContractArtifactField::Linearization {
                let selection: Result<ContractOutputSelection, _> = field.try_into();
                assert!(
                    selection
                        .unwrap_err()
                        .to_string()
                        .eq("Linearization is not supported for ContractOutputSelection")
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
}
