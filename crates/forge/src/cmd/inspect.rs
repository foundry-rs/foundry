use alloy_json_abi::{EventParam, InternalType, JsonAbi, Param};
use alloy_primitives::{hex, keccak256};
use clap::Parser;
use comfy_table::{Cell, Table, modifiers::UTF8_ROUND_CORNERS, presets::ASCII_MARKDOWN};
use eyre::{Result, eyre};
use foundry_cli::opts::{BuildOpts, CompilerOpts};
use foundry_common::{
    compile::{PathOrContractInfo, ProjectCompiler},
    find_matching_contract_artifact, find_target_path, shell,
};
use foundry_compilers::artifacts::ast as solast;
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
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    str::FromStr,
    sync::LazyLock,
};

// Regexes for storage bucket annotations (module-level LazyLocks)
static STORAGE_BUCKET_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@custom:storage-bucket\s+(.+)").unwrap());

static STORAGE_BUCKET_SCHEMA_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@custom:storage-bucket-schema\s+(\S+)").unwrap());

static STORAGE_BUCKET_SLOT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@custom:storage-bucket-slot\s+(\S+)(?:\s+(0x[0-9a-fA-F]+))?").unwrap()
});

// Transient (EIP-1153)
static STORAGE_BUCKET_TRANSIENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@custom:storage-bucket-transient\s+(.+)").unwrap());

static STORAGE_BUCKET_TRANSIENT_SLOT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@custom:storage-bucket-transient-slot\s+(\S+)(?:\s+(0x[0-9a-fA-F]+))?").unwrap()
});

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

    /// Show EIP-7201 (bucket-based) storage layout instead of compiler layout
    #[arg(long, help_heading = "Display options")]
    pub eip7201: bool,
}

impl InspectArgs {
    pub fn run(self) -> Result<()> {
        let Self { contract, field, build, strip_yul_comments, eip7201 } = self;

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
            // Ensure AST is included so we can traverse it in this command
            compiler: CompilerOpts {
                ast: eip7201,
                extra_output: cos,
                optimize: optimized,
                ..build.compiler
            },
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
                let abi = artifact
                    .abi
                    .as_ref()
                    .ok_or_else(|| eyre::eyre!("Failed to fetch lossless ABI"))?;
                print_abi(abi)?;
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
                print_method_identifiers(&artifact.method_identifiers)?;
            }
            ContractArtifactField::GasEstimates => {
                print_json(&artifact.gas_estimates)?;
            }
            ContractArtifactField::StorageLayout => {
                if !eip7201 {
                    print_storage_layout(artifact.storage_layout.as_ref())?;
                } else {
                    let cname = contract.name().ok_or_else(|| {
                        eyre!("Contract name is required when using --eip7201")
                    })?;
                    print_storage_layout_from_ast(&artifact, cname, &output)?;
                }
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
                print_errors_events(&out, true)?;
            }
            ContractArtifactField::Events => {
                let out = artifact.abi.as_ref().map_or(Map::new(), parse_events);
                print_errors_events(&out, false)?;
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

// New AST-only pipeline for storage-bucket inspection
fn print_storage_layout_from_ast(
    artifact: &foundry_compilers::artifacts::ConfigurableContractArtifact,
    contract_name: &str,
    output: &foundry_compilers::ProjectCompileOutput,
) -> Result<()> {
    let ast: &solast::Ast =
        artifact.ast.as_ref().ok_or_else(|| eyre!("AST not available; re-run with --ast"))?;

    // 1) Initialize buckets from constructor @custom:storage-bucket
    let mut buckets: Vec<BucketRow> = Vec::new();
    collect_constructor_buckets(ast, contract_name, &mut buckets);

    // 2) For each bucket: fill schema and slot from matching function annotations
    fill_bucket_slot(ast, contract_name, &mut buckets);

    // 3) Cross-artifact doc matching for schema and slot annotations
    fill_bucket_schema_across_artifacts(output, &mut buckets);

    // Build type registry for expansions and constants
    let type_registry = build_type_registry(output);

    // Assign base slots from function bodies via AST referenced ids
    resolve_bucket_slots_from_function_bodies(output, &type_registry, &mut buckets);

    // Print
    if shell::is_json() {
        return print_json(&buckets);
    }

    let headers = vec![
        Cell::new("Name"),
        Cell::new("Type"),
        Cell::new("Slot"),
        Cell::new("Offset"),
        Cell::new("Bytes"),
        Cell::new("Contract"),
    ];
    print_table(headers, |table| {
        // First, add standard compiler-provided storage layout rows (if available)
        if let Some(std_layout) = artifact.storage_layout.as_ref() {
            for slot in &std_layout.storage {
                let storage_type = std_layout.types.get(&slot.storage_type);
                table.add_row([
                    slot.label.as_str(),
                    storage_type.map_or("?", |t| &t.label),
                    &slot.slot,
                    &slot.offset.to_string(),
                    storage_type.map_or("?", |t| &t.number_of_bytes),
                    &slot.contract,
                ]);
            }
        }

        let mut used_aliases: BTreeSet<String> = BTreeSet::new();
        for b in &buckets {
            let raw_typ = b.bucket_type.clone();
            let mut display_typ = if raw_typ.starts_with("singleton(") && raw_typ.ends_with(')') {
                raw_typ.trim_start_matches("singleton(").trim_end_matches(')').trim().to_string()
            } else {
                raw_typ.clone()
            };
            if b.transient {
                display_typ = format!("[transient] {}", display_typ);
            }
            let mut base_slot = b.slot.clone();
            let mut slot_cell = base_slot.clone();
            let mut make_unique_alias = |alias: String| {
                if !used_aliases.contains(&alias) {
                    used_aliases.insert(alias.clone());
                    return alias;
                }
                let mut alias_star = format!("{}*", alias);
                while used_aliases.contains(&alias_star) {
                    alias_star.push('*');
                }
                used_aliases.insert(alias_star.clone());
                alias_star
            };
            // If bucket is a singleton/struct, introduce an alias for the base slot for readability
            if raw_typ.starts_with("singleton(") {
                if let Some(inner) =
                    raw_typ.strip_prefix("singleton(").and_then(|s| s.strip_suffix(')'))
                {
                    let inner = inner.trim();
                    let struct_name = inner.strip_prefix("struct ").unwrap_or(inner);
                    if has_struct_named(&type_registry, struct_name) {
                        let alias = make_unique_alias(get_struct_alias(struct_name));
                        let displayed = if base_slot.is_empty() {
                            "<slot>".to_string()
                        } else {
                            base_slot.clone()
                        };
                        slot_cell = format!("{} = {}", alias, displayed);
                        base_slot = alias.clone();
                    }
                }
            } else if raw_typ.starts_with("struct ") {
                let struct_name = raw_typ.trim_start_matches("struct ").trim();
                if has_struct_named(&type_registry, struct_name) {
                    let alias = make_unique_alias(get_struct_alias(struct_name));
                    let displayed =
                        if b.slot.is_empty() { "<slot>".to_string() } else { b.slot.clone() };
                    slot_cell = format!("{} = {}", alias, displayed);
                    base_slot = alias.clone();
                }
            } else if raw_typ.starts_with("mapping(") {
                // If mapping to struct, prefer aliasing its base slot to something readable
                if let Some(value_part) = raw_typ.split("=>").nth(1) {
                    let v = value_part.trim().trim_end_matches(')');
                    let struct_name = v.strip_prefix("struct ").unwrap_or(v);
                    if has_struct_named(&type_registry, struct_name) {
                        let alias = make_unique_alias(get_struct_alias(struct_name));
                        let displayed = if base_slot.is_empty() {
                            "<slot>".to_string()
                        } else {
                            base_slot.clone()
                        };
                        slot_cell = format!("{} = keccak(key, {})", alias, displayed);
                        base_slot = alias.clone();
                    }
                }
            }

            table.add_row([&b.name, &display_typ, &slot_cell, "0", "32", &b.contract]);

            if raw_typ.starts_with("singleton(") {
                if let Some(inner) =
                    raw_typ.strip_prefix("singleton(").and_then(|s| s.strip_suffix(')'))
                {
                    let inner = inner.trim();
                    let struct_name = inner.strip_prefix("struct ").unwrap_or(inner);
                    if has_struct_named(&type_registry, struct_name) {
                        expand_struct_layout_rows(
                            &type_registry,
                            struct_name,
                            &b.name,
                            &b.contract,
                            &base_slot,
                            table,
                            0,
                            &mut used_aliases,
                        );
                    }
                }
            } else if raw_typ.starts_with("struct ") {
                let struct_name = raw_typ.trim_start_matches("struct ").trim();
                expand_struct_layout_rows(
                    &type_registry,
                    struct_name,
                    &b.name,
                    &b.contract,
                    &base_slot,
                    table,
                    0,
                    &mut used_aliases,
                );
            } else if raw_typ.starts_with("mapping(") {
                if let Some(value_part) = raw_typ.split("=>").nth(1) {
                    let v = value_part.trim().trim_end_matches(')');
                    let struct_name = v.strip_prefix("struct ").unwrap_or(v);
                    if has_struct_named(&type_registry, struct_name) {
                        // base_slot already set to alias above (e.g., M); use it so children render as M, M + 1, ...
                        let base = if base_slot.is_empty() {
                            "<slot>".to_string()
                        } else {
                            base_slot.clone()
                        };
                        expand_struct_layout_rows(
                            &type_registry,
                            struct_name,
                            &b.name,
                            &b.contract,
                            &base,
                            table,
                            0,
                            &mut used_aliases,
                        );
                    }
                }
            }
        }
    })
}

fn fill_bucket_schema_across_artifacts(
    output: &foundry_compilers::ProjectCompileOutput,
    buckets: &mut [BucketRow],
) {
    for (_id, artifact) in output.artifact_ids() {
        if let Some(ast) = &artifact.ast {
            for node in &ast.nodes {
                match node.node_type {
                    // Walk into contracts/libraries and inspect child functions/vars
                    solast::NodeType::ContractDefinition => {
                        for child in &node.nodes {
                            if matches!(child.node_type, solast::NodeType::FunctionDefinition) {
                                let docs: Option<solast::Documentation> =
                                    child.attribute("documentation");
                                let doc_text = documentation_text(docs);
                                for bucket in buckets.iter_mut() {
                                    if let Some(caps) = STORAGE_BUCKET_SCHEMA_RE.captures(&doc_text)
                                    {
                                        let schema_id = caps.get(1).unwrap().as_str();
                                        if schema_id == bucket.name {
                                            bucket.schema_func = Some(child.clone());
                                            bucket.bucket_type =
                                                infer_typed_schema_from_function_ast(child)
                                                    .unwrap_or_else(|| {
                                                        infer_untyped_schema_from_function_ast(
                                                            child,
                                                        )
                                                        .unwrap_or_else(|| "unknown".into())
                                                    });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Some tools may emit free-standing FunctionDefinition/VariableDeclaration
                    solast::NodeType::FunctionDefinition => {
                        let docs: Option<solast::Documentation> = node.attribute("documentation");
                        let doc_text = documentation_text(docs);
                        for bucket in buckets.iter_mut() {
                            if let Some(caps) = STORAGE_BUCKET_SCHEMA_RE.captures(&doc_text) {
                                let schema_id = caps.get(1).unwrap().as_str();
                                if schema_id == bucket.name {
                                    bucket.schema_func = Some(node.clone());
                                    bucket.bucket_type = infer_typed_schema_from_function_ast(node)
                                        .unwrap_or_else(|| {
                                            infer_untyped_schema_from_function_ast(node)
                                                .unwrap_or_else(|| "unknown".into())
                                        });
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct BucketRow {
    name: String,
    bucket_type: String,
    slot: String,
    contract: String,
    transient: bool,
    #[serde(skip_serializing)]
    schema_func: Option<solast::Node>,
}

#[derive(Debug, Clone)]
struct StructInfo {
    id: Option<usize>,
    name: String,
    source_path: String,
    members: Vec<solast::VariableDeclaration>,
}

#[derive(Debug, Clone)]
struct MemberLayout {
    name: String,
    kind: TypeKind,
    slot_offset: usize,
    byte_offset: usize,
    size_bytes: usize,
    type_name: String,
    struct_id: Option<usize>,
    mapping_keys: Option<String>,
}

#[derive(Debug, Clone)]
enum TypeKind {
    Primitive,
    Enum,
    UserDefinedType,
    Struct(usize),
    MappingToStruct { struct_id: usize, key_types: Vec<String> },
    MappingOther { key_types: Vec<String> },
    Array { base_kind: Box<TypeKind>, is_dynamic: bool },
}

#[derive(Debug, Clone)]
struct EnumInfo {
    id: Option<usize>,
    name: String,
    source_path: String,
    num_values: usize,
}

#[derive(Debug, Clone)]
struct UserTypeInfo {
    id: Option<usize>,
    name: String,
    source_path: String,
    underlying_label: String,
}

#[derive(Debug, Default, Clone)]
struct TypeRegistry {
    structs_by_id: BTreeMap<isize, StructInfo>,
    enums_by_id: BTreeMap<isize, EnumInfo>,
    usertypes_by_id: BTreeMap<isize, UserTypeInfo>,
    structs_by_name: BTreeMap<String, StructInfo>,
    enums_by_name: BTreeMap<String, EnumInfo>,
    usertypes_by_name: BTreeMap<String, UserTypeInfo>,
    slot_consts_by_name: BTreeMap<String, String>,
    slot_consts_by_id: BTreeMap<isize, String>,
}

fn build_type_registry(output: &foundry_compilers::ProjectCompileOutput) -> TypeRegistry {
    let mut reg = TypeRegistry::default();
    for (_id, artifact) in output.artifact_ids() {
        if let Some(ast) = &artifact.ast {
            collect_types_from_ast(ast, &mut reg);
        }
    }
    reg
}

fn collect_types_from_ast(ast: &solast::Ast, reg: &mut TypeRegistry) {
    let src_path = ast.absolute_path.clone();
    for node in &ast.nodes {
        collect_types_from_node(node, &src_path, reg);
    }
}

fn collect_types_from_node(node: &solast::Node, src_path: &str, reg: &mut TypeRegistry) {
    match node.node_type {
        solast::NodeType::StructDefinition => {
            let name: Option<String> = node.attribute("name");
            let members: Option<Vec<solast::VariableDeclaration>> = node.attribute("members");
            let info = StructInfo {
                id: node.id,
                name: name.clone().unwrap_or_default(),
                source_path: src_path.to_string(),
                members: members.unwrap_or_default(),
            };
            if let Some(id) = node.id.map(|v| v as isize) {
                reg.structs_by_id.insert(id, info.clone());
            }
            if let Some(n) = name {
                reg.structs_by_name.insert(format!("{}:{}", short_path(src_path), n), info);
            }
        }
        solast::NodeType::EnumDefinition => {
            let name: Option<String> = node.attribute("name");
            let members: Option<Vec<solast::EnumValue>> = node.attribute("members");
            let info = EnumInfo {
                id: node.id,
                name: name.clone().unwrap_or_default(),
                source_path: src_path.to_string(),
                num_values: members.as_ref().map(|v| v.len()).unwrap_or(0),
            };
            if let Some(id) = node.id.map(|v| v as isize) {
                reg.enums_by_id.insert(id, info.clone());
            }
            if let Some(n) = name {
                reg.enums_by_name.insert(n, info);
            }
        }
        solast::NodeType::UserDefinedValueTypeDefinition => {
            let name: Option<String> = node.attribute("name");
            let underlying: Option<solast::TypeName> = node.attribute("underlyingType");
            let info = UserTypeInfo {
                id: node.id,
                name: name.clone().unwrap_or_default(),
                source_path: src_path.to_string(),
                underlying_label: underlying
                    .map(|t| type_string_from_typename(&t))
                    .unwrap_or_else(|| "uint256".to_string()),
            };
            if let Some(id) = node.id.map(|v| v as isize) {
                reg.usertypes_by_id.insert(id, info.clone());
            }
            if let Some(n) = name {
                reg.usertypes_by_name.insert(n, info);
            }
        }
        solast::NodeType::VariableDeclaration => {
            // Capture global and file-level constants
            let is_const: bool = node.attribute::<bool>("constant").unwrap_or(false);
            let mutability: Option<String> = node.attribute("mutability");
            let is_const_mut = matches!(mutability.as_deref(), Some("constant"));
            if is_const || is_const_mut {
                if let Some(name) = node.attribute::<String>("name") {
                    if let Some(expr) = node.attribute::<solast::Expression>("value") {
                        if let Some(val) = literal_hex_or_value(expr) {
                            reg.slot_consts_by_name.entry(name.clone()).or_insert(val.clone());
                            if let Some(id) = node.id {
                                reg.slot_consts_by_id.insert(id as isize, val.clone());
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    for child in &node.nodes {
        collect_types_from_node(child, src_path, reg);
    }
    if let Some(body) = &node.body {
        collect_types_from_node(body, src_path, reg);
    }
}

fn short_path(path: &str) -> String {
    if let Some(pos) = path.rfind('/') { path[pos + 1..].to_string() } else { path.to_string() }
}

fn expand_struct_layout_rows(
    reg: &TypeRegistry,
    struct_name: &str,
    _bucket_display: &str,
    _contract: &str,
    base_slot: &str,
    table: &mut comfy_table::Table,
    indent_level: usize,
    used_aliases: &mut BTreeSet<String>,
) {
    if let Some(si) = find_struct_by_simple_name(reg, struct_name) {
        // Add a struct header row with requested indentation
        let origin_label = format!("{}:{}", short_path(&si.source_path), si.name);
        let struct_bytes = struct_total_slots(si, reg) * 32;
        table.add_row([
            &format!("{}├─ {}", " ".repeat(indent_level * 8), si.name),
            &format!("struct {}", si.name),
            "",
            "0",
            &struct_bytes.to_string(),
            &origin_label,
        ]);

        let layout = compute_typed_struct_layout(si, reg);
        render_member_layouts(
            &layout,
            &si.name,
            base_slot,
            reg,
            table,
            indent_level + 1,
            used_aliases,
        );
    }
}

fn render_member_layouts(
    layout: &[MemberLayout],
    bucket_display: &str,
    base_slot: &str,
    reg: &TypeRegistry,
    table: &mut comfy_table::Table,
    indent_level: usize,
    used_aliases: &mut BTreeSet<String>,
) {
    for member in layout {
        let slot_formula = combine_slot_offset(base_slot, member.slot_offset);
        let indent = " ".repeat(indent_level * 8);

        match &member.kind {
            TypeKind::Struct(struct_id) => {
                // Struct header row + expand members using simple struct name
                if let Some(si) = reg.structs_by_id.get(&(*struct_id as isize)) {
                    let origin_label = format!("{}:{}", short_path(&si.source_path), si.name);
                    table.add_row([
                        &format!("{}├─ {}", indent, si.name),
                        &format!("struct {}", si.name),
                        "",
                        "0",
                        &(struct_total_slots(si, reg) * 32).to_string(),
                        &origin_label,
                    ]);

                    let nested_layout = compute_typed_struct_layout(si, reg);
                    render_member_layouts(
                        &nested_layout,
                        &si.name,
                        &slot_formula,
                        reg,
                        table,
                        indent_level + 1,
                        used_aliases,
                    );
                }
            }
            TypeKind::MappingToStruct { struct_id, key_types } => {
                // Mapping to struct: print alias header, then expand using alias
                if let Some(si) = reg.structs_by_id.get(&(*struct_id as isize)) {
                    let origin_label = format!("{}:{}", short_path(&si.source_path), si.name);
                    let keys = key_types.join(", ");
                    // Ensure unique alias across the whole table
                    let mut alias = get_struct_alias(&si.name);
                    if used_aliases.contains(&alias) {
                        let mut tmp = format!("{}*", alias);
                        while used_aliases.contains(&tmp) {
                            tmp.push('*');
                        }
                        alias = tmp;
                    }
                    used_aliases.insert(alias.clone());
                    let map_base = format!("keccak({}, {})", keys, slot_formula);

                    table.add_row([
                        &format!("{}├─ {}.{}", indent, bucket_display, member.name),
                        &member.type_name,
                        &format!("{} = {}", alias, map_base),
                        "0",
                        &member.size_bytes.to_string(),
                        &origin_label,
                    ]);
                    // Insert a struct header row under the mapping row
                    let struct_bytes = struct_total_slots(si, reg) * 32;
                    table.add_row([
                        &format!("{}├─ {}", " ".repeat((indent_level + 1) * 8), si.name),
                        &format!("struct {}", si.name),
                        "",
                        "0",
                        &struct_bytes.to_string(),
                        &origin_label,
                    ]);

                    let nested_layout = compute_typed_struct_layout(si, reg);
                    render_member_layouts(
                        &nested_layout,
                        &si.name,
                        &alias,
                        reg,
                        table,
                        indent_level + 1,
                        used_aliases,
                    );
                }
            }
            _ => {
                // Primitive/enum/UDT/array/mapping-other: leaf row
                let origin = if let Some(struct_id) = member.struct_id {
                    if let Some(si) = reg.structs_by_id.get(&(struct_id as isize)) {
                        format!("{}:{}", short_path(&si.source_path), si.name)
                    } else {
                        "Unknown".to_string()
                    }
                } else {
                    "Built-in".to_string()
                };

                table.add_row([
                    &format!("{}├─ {}.{}", indent, bucket_display, member.name),
                    &member.type_name,
                    &slot_formula,
                    &member.byte_offset.to_string(),
                    &member.size_bytes.to_string(),
                    &origin,
                ]);
            }
        }
    }
}

fn get_struct_alias(struct_name: &str) -> String {
    struct_name.chars().next().unwrap_or('S').to_uppercase().collect()
}

fn combine_slot_offset(base: &str, offset: usize) -> String {
    if offset == 0 {
        return base.to_string();
    }
    // Try to fold existing "+ N" at the end of base
    // Pattern: "<any> + <number>"
    if let Some(pos) = base.rfind('+') {
        let (head, tail) = base.split_at(pos);
        let n_str = tail.trim_start_matches('+').trim();
        if let Ok(n) = n_str.parse::<usize>() {
            return format!("{} + {}", head.trim_end(), n + offset);
        }
    }
    format!("{} + {}", base, offset)
}

fn struct_total_slots(si: &StructInfo, reg: &TypeRegistry) -> usize {
    let layout = compute_typed_struct_layout(si, reg);
    if layout.is_empty() {
        return 1;
    }
    let last = layout.last().unwrap();
    if last.size_bytes >= 32 {
        last.slot_offset + ((last.size_bytes + 31) / 32)
    } else {
        last.slot_offset + 1
    }
}

fn compute_typed_struct_layout(si: &StructInfo, reg: &TypeRegistry) -> Vec<MemberLayout> {
    let mut layout = Vec::new();
    let mut slot_index: usize = 0;
    let mut used_in_slot: usize = 0;

    for m in &si.members {
        let kind = classify_type_from_var(m, reg);
        let type_name = type_string_from_var(m);
        let size = member_size_bytes(&kind, &type_name, reg);

        // Check if we need to advance to next slot
        let is_full_slot = matches!(
            kind,
            TypeKind::MappingToStruct { .. }
                | TypeKind::MappingOther { .. }
                | TypeKind::Array { .. }
        ) || size >= 32;
        let is_struct = matches!(kind, TypeKind::Struct(_));

        if is_full_slot || is_struct || used_in_slot + size > 32 {
            if used_in_slot > 0 {
                slot_index += 1;
                used_in_slot = 0;
            }
        }

        let byte_offset = if is_full_slot || is_struct { 0 } else { used_in_slot };

        layout.push(MemberLayout {
            name: m.name.clone(),
            kind: kind.clone(),
            slot_offset: slot_index,
            byte_offset,
            size_bytes: size,
            type_name,
            struct_id: si.id,
            mapping_keys: extract_mapping_keys(&kind),
        });

        if is_full_slot || is_struct {
            let slots_consumed = match &kind {
                TypeKind::Struct(struct_id) => reg
                    .structs_by_id
                    .get(&(*struct_id as isize))
                    .map(|s| struct_total_slots(s, reg))
                    .unwrap_or(1),
                _ => (size + 31) / 32,
            };
            slot_index += slots_consumed;
            used_in_slot = 0;
        } else {
            used_in_slot += size;
            if used_in_slot == 32 {
                slot_index += 1;
                used_in_slot = 0;
            }
        }
    }

    layout
}

fn classify_type_from_var(var: &solast::VariableDeclaration, reg: &TypeRegistry) -> TypeKind {
    if let Some(type_name) = &var.type_name {
        classify_type_from_typename(type_name, reg)
    } else {
        TypeKind::Primitive
    }
}

fn classify_type_from_typename(type_name: &solast::TypeName, reg: &TypeRegistry) -> TypeKind {
    match type_name {
        solast::TypeName::ElementaryTypeName(_) => TypeKind::Primitive,
        solast::TypeName::UserDefinedTypeName(udt) => {
            // Prefer resolving by type string/name to avoid version-specific referenced_declaration shapes
            let name = if let Some(n) = &udt.name {
                clean_ast_type_str(n)
            } else if let Some(path) = &udt.path_node {
                clean_ast_type_str(&path.name)
            } else if let Some(ts) = &udt.type_descriptions.type_string {
                clean_ast_type_str(ts)
            } else {
                String::new()
            };

            if !name.is_empty() {
                if let Some(si) = reg.structs_by_name.iter().find_map(|(k, v)| {
                    if k.ends_with(&format!(":{}", name)) { Some(v) } else { None }
                }) {
                    if let Some(id) = si.id {
                        return TypeKind::Struct(id);
                    }
                }
                if reg.enums_by_name.contains_key(&name) {
                    return TypeKind::Enum;
                }
                if reg.usertypes_by_name.contains_key(&name) {
                    return TypeKind::UserDefinedType;
                }
            }
            TypeKind::Primitive
        }
        solast::TypeName::Mapping(mapping) => {
            let key_types = vec![type_string_from_typename(&mapping.key_type)];
            match classify_type_from_typename(&mapping.value_type, reg) {
                TypeKind::Struct(struct_id) => TypeKind::MappingToStruct { struct_id, key_types },
                _ => TypeKind::MappingOther { key_types },
            }
        }
        solast::TypeName::ArrayTypeName(array) => {
            let base_kind = Box::new(classify_type_from_typename(&array.base_type, reg));
            let is_dynamic = array.length.is_none();
            TypeKind::Array { base_kind, is_dynamic }
        }
        solast::TypeName::FunctionTypeName(_) => TypeKind::Primitive,
    }
}

fn member_size_bytes(kind: &TypeKind, type_label: &str, reg: &TypeRegistry) -> usize {
    match kind {
        TypeKind::Primitive => elementary_size_bytes(type_label).unwrap_or(32),
        TypeKind::Enum => 1,
        TypeKind::UserDefinedType => {
            if let Some(ut) = reg.usertypes_by_name.get(type_label) {
                elementary_size_bytes(&ut.underlying_label).unwrap_or(32)
            } else {
                32
            }
        }
        TypeKind::Struct(struct_id) => {
            if let Some(si) = reg.structs_by_id.get(&(*struct_id as isize)) {
                struct_total_slots(si, reg) * 32
            } else {
                32
            }
        }
        TypeKind::MappingToStruct { .. } | TypeKind::MappingOther { .. } => 32,
        TypeKind::Array { .. } => 32,
    }
}

fn extract_mapping_keys(kind: &TypeKind) -> Option<String> {
    match kind {
        TypeKind::MappingToStruct { key_types, .. } | TypeKind::MappingOther { key_types } => {
            if key_types.len() == 1 {
                Some("key".to_string())
            } else {
                Some(format!(
                    "key{}",
                    (1..=key_types.len()).map(|i| i.to_string()).collect::<Vec<_>>().join(", key")
                ))
            }
        }
        _ => None,
    }
}

fn elementary_size_bytes(t: &str) -> Option<usize> {
    if t.starts_with("uint") || t.starts_with("int") {
        let bits: usize = t.trim_start_matches(|c: char| c.is_alphabetic()).parse().unwrap_or(256);
        return Some((bits + 7) / 8);
    }
    if t == "bool" {
        return Some(1);
    }
    if t == "address" || t == "address payable" {
        return Some(20);
    }
    if t == "bytes32" {
        return Some(32);
    }
    if t.starts_with("bytes") {
        let n: usize = t[5..].parse().unwrap_or(32);
        return Some(n);
    }
    None
}

fn literal_hex_or_value(expr: solast::Expression) -> Option<String> {
    match expr {
        solast::Expression::Literal(lit) => {
            if let Some(v) = lit.value {
                return Some(v);
            }
            if !lit.hex_value.is_empty() {
                return Some(format!("0x{}", lit.hex_value));
            }
            None
        }
        solast::Expression::FunctionCall(fc) => {
            if let solast::Expression::Identifier(id) = fc.expression {
                if id.name == "keccak256" {
                    return Some("keccak256(...)".to_string());
                }
            }
            None
        }
        _ => None,
    }
}

// removed unused is_dynamic_type helper

fn has_struct_named(reg: &TypeRegistry, simple_name: &str) -> bool {
    reg.structs_by_name.keys().any(|k| k.ends_with(&format!(":{}", simple_name)))
}

fn find_struct_by_simple_name<'a>(
    reg: &'a TypeRegistry,
    simple_name: &str,
) -> Option<&'a StructInfo> {
    reg.structs_by_name
        .iter()
        .find(|(k, _)| k.ends_with(&format!(":{}", simple_name)))
        .map(|(_, v)| v)
}

fn collect_constructor_buckets(ast: &solast::Ast, contract_name: &str, out: &mut Vec<BucketRow>) {
    for node in &ast.nodes {
        if matches!(node.node_type, solast::NodeType::ContractDefinition) {
            // get contract name
            let name: Option<String> = node.attribute("name");
            if name.as_deref() != Some(contract_name) {
                continue;
            }

            // iterate members
            for child in &node.nodes {
                if !matches!(child.node_type, solast::NodeType::FunctionDefinition) {
                    continue;
                }
                let kind: Option<String> = child.attribute("kind");
                if kind.as_deref() != Some("constructor") {
                    continue;
                }
                // documentation
                let docs: Option<solast::Documentation> = child.attribute("documentation");
                let doc_text = documentation_text(docs);
                if !doc_text.is_empty() {
                    for caps in STORAGE_BUCKET_RE.captures_iter(doc_text.trim()) {
                        let name = caps.get(1).unwrap().as_str().trim().to_string();
                        out.push(BucketRow {
                            name,
                            bucket_type: "unknown".into(),
                            slot: "".into(),
                            contract: contract_name.into(),
                            transient: false,
                            schema_func: None,
                        });
                    }
                    for caps in STORAGE_BUCKET_TRANSIENT_RE.captures_iter(doc_text.trim()) {
                        let name = caps.get(1).unwrap().as_str().trim().to_string();
                        out.push(BucketRow {
                            name,
                            bucket_type: "unknown".into(),
                            slot: "".into(),
                            contract: contract_name.into(),
                            transient: true,
                            schema_func: None,
                        });
                    }
                }
            }
        }
    }
}

fn fill_bucket_slot(ast: &solast::Ast, contract_name: &str, buckets: &mut [BucketRow]) {
    for node in &ast.nodes {
        if !matches!(node.node_type, solast::NodeType::ContractDefinition) {
            continue;
        }
        let name: Option<String> = node.attribute("name");
        if name.as_deref() != Some(contract_name) {
            continue;
        }

        for child in &node.nodes {
            if !matches!(child.node_type, solast::NodeType::FunctionDefinition) {
                continue;
            }
            let docs: Option<solast::Documentation> = child.attribute("documentation");
            let doc_text = documentation_text(docs);

            // Match by SCHEMA_ID in annotations (schema/slot lines contain schema id)
            for bucket in buckets.iter_mut() {
                // Fill slot
                if let Some(caps) = STORAGE_BUCKET_SLOT_RE.captures(&doc_text) {
                    let schema_id = caps.get(1).unwrap().as_str();
                    let slot_hex = caps.get(2).map(|m| m.as_str()).unwrap_or("0x0");
                    if schema_id == bucket.name {
                        bucket.slot = slot_hex.to_string();
                    }
                }

                // Fill transient slot
                if let Some(caps) = STORAGE_BUCKET_TRANSIENT_SLOT_RE.captures(&doc_text) {
                    let schema_id = caps.get(1).unwrap().as_str();
                    let slot_hex = caps.get(2).map(|m| m.as_str()).unwrap_or("0x0");
                    if schema_id == bucket.name {
                        bucket.slot = slot_hex.to_string();
                    }
                }
            }
        }
    }
}

fn infer_untyped_schema_from_function_ast(func: &solast::Node) -> Option<String> {
    let params: Option<solast::ParameterList> = func.attribute("parameters");
    let returns: Option<solast::ParameterList> = func.attribute("returnParameters");
    let param_count = params.map(|p| p.parameters.len()).unwrap_or(0);
    let ret_count = returns.map(|r| r.parameters.len()).unwrap_or(0);
    match (param_count, ret_count) {
        (0, 1) => Some("singleton".into()),
        (1, 1) => Some("mapping(K => V)".into()),
        (n, 1) if n > 1 => Some("mapping(K1, K2, ... => V)".into()),
        _ => None,
    }
}

fn infer_typed_schema_from_function_ast(func: &solast::Node) -> Option<String> {
    let param_types = extract_param_types_from_node(func);
    let return_types = extract_return_types_from_node(func);
    match (param_types.len(), return_types.get(0)) {
        (0, Some(v)) => Some(format!("singleton({})", v)),
        (1, Some(v)) => Some(format!("mapping({} => {})", param_types[0], v)),
        (n, Some(v)) if n > 1 => Some(format!("mapping({} => {})", param_types.join(", "), v)),
        _ => None,
    }
}

fn extract_param_types_from_node(func: &solast::Node) -> Vec<String> {
    if let Some(list) = func.attribute::<solast::ParameterList>("parameters") {
        return type_strings_from_parameter_list(&list);
    }
    Vec::new()
}

fn extract_return_types_from_node(func: &solast::Node) -> Vec<String> {
    if let Some(list) = func.attribute::<solast::ParameterList>("returnParameters") {
        return type_strings_from_parameter_list(&list);
    }
    Vec::new()
}

fn type_strings_from_parameter_list(list: &solast::ParameterList) -> Vec<String> {
    list.parameters.iter().map(type_string_from_var).collect()
}

fn type_string_from_var(v: &solast::VariableDeclaration) -> String {
    if let Some(ts) = &v.type_descriptions.type_string {
        return clean_ast_type_str(ts);
    }
    if let Some(ty) = &v.type_name {
        return type_string_from_typename(ty);
    }
    "unknown".to_string()
}

fn type_string_from_typename(ty: &solast::TypeName) -> String {
    match ty {
        solast::TypeName::ElementaryTypeName(t) => t.name.clone(),
        solast::TypeName::UserDefinedTypeName(u) => {
            if let Some(name) = &u.name {
                clean_ast_type_str(name)
            } else if let Some(path) = &u.path_node {
                clean_ast_type_str(&path.name)
            } else if let Some(ts) = &u.type_descriptions.type_string {
                clean_ast_type_str(ts)
            } else {
                "userdefined".to_string()
            }
        }
        solast::TypeName::Mapping(m) => {
            let k = type_string_from_typename(&m.key_type);
            let v = type_string_from_typename(&m.value_type);
            format!("mapping({} => {})", k, v)
        }
        solast::TypeName::ArrayTypeName(a) => {
            let base = type_string_from_typename(&a.base_type);
            format!("{}[]", base)
        }
        solast::TypeName::FunctionTypeName(_) => "function".to_string(),
    }
}

fn clean_ast_type_str(s: &str) -> String {
    let mut out = s.to_string();
    if let Some(rest) = out.strip_prefix("struct ") {
        out = rest.to_string();
    }
    if let Some(idx) = out.find(" storage") {
        out.truncate(idx);
    }
    if let Some(idx) = out.find(" ref") {
        out.truncate(idx);
    }
    out
}

fn extract_bucket_function_name(bucket_name: &str) -> String {
    // Accept forms like "Namespace.func" or "Namespace:func" and extract the function part
    if let Some(idx) = bucket_name.rfind(['.', ':']) {
        bucket_name[idx + 1..].to_string()
    } else {
        bucket_name.to_string()
    }
}

fn resolve_bucket_slots_from_function_bodies(
    output: &foundry_compilers::ProjectCompileOutput,
    reg: &TypeRegistry,
    buckets: &mut [BucketRow],
) {
    // Map function simple name -> (idents, ref ids) found in body across all artifacts
    let mut func_body_refs: BTreeMap<String, (Vec<String>, Vec<isize>)> = BTreeMap::new();
    // Map schema id (from @custom:storage-bucket-schema <ID>) -> (idents, ref ids)
    let mut schema_body_refs: BTreeMap<String, (Vec<String>, Vec<isize>)> = BTreeMap::new();
    for (_id, artifact) in output.artifact_ids() {
        if let Some(ast) = &artifact.ast {
            for node in &ast.nodes {
                match node.node_type {
                    // Libraries are represented as ContractDefinition with kind==Library
                    solast::NodeType::ContractDefinition => {
                        for child in &node.nodes {
                            if !matches!(child.node_type, solast::NodeType::FunctionDefinition) {
                                continue;
                            }
                            let func_name: Option<String> = child.attribute("name");
                            let Some(fname) = func_name else {
                                continue;
                            };
                            if let Ok(json) = serde_json::to_value(child) {
                                let mut ids = Vec::new();
                                let mut ref_ids = Vec::new();

                                collect_idents_and_refs_in_node_json(&json, &mut ids, &mut ref_ids);
                                let entry = func_body_refs.entry(fname).or_default();
                                entry.0.extend(ids);
                                entry.1.extend(ref_ids);
                                // Also index by schema id if present in docs
                                let docs: Option<solast::Documentation> =
                                    child.attribute("documentation");
                                let doc_text = documentation_text(docs);
                                if let Some(caps) = STORAGE_BUCKET_SCHEMA_RE.captures(&doc_text) {
                                    let schema_id = caps.get(1).unwrap().as_str().to_string();
                                    let entry = schema_body_refs.entry(schema_id).or_default();
                                    // recompute to avoid moving previous vectors
                                    let mut ids2 = Vec::new();
                                    let mut ref_ids2 = Vec::new();
                                    collect_idents_and_refs_in_node_json(
                                        &json,
                                        &mut ids2,
                                        &mut ref_ids2,
                                    );
                                    entry.0.extend(ids2);
                                    entry.1.extend(ref_ids2);
                                }
                            }
                        }
                    }
                    solast::NodeType::FunctionDefinition => {
                        let func_name: Option<String> = node.attribute("name");
                        if let Some(fname) = func_name {
                            if let Ok(json) = serde_json::to_value(node) {
                                let mut ids = Vec::new();
                                let mut ref_ids = Vec::new();

                                collect_idents_and_refs_in_node_json(&json, &mut ids, &mut ref_ids);
                                let entry = func_body_refs.entry(fname).or_default();
                                entry.0.extend(ids);
                                entry.1.extend(ref_ids);
                                // Index by schema id if present on this function
                                let docs: Option<solast::Documentation> =
                                    node.attribute("documentation");
                                let doc_text = documentation_text(docs);
                                if let Some(caps) = STORAGE_BUCKET_SCHEMA_RE.captures(&doc_text) {
                                    let schema_id = caps.get(1).unwrap().as_str().to_string();
                                    let entry = schema_body_refs.entry(schema_id).or_default();
                                    let mut ids2 = Vec::new();
                                    let mut ref_ids2 = Vec::new();
                                    collect_idents_and_refs_in_node_json(
                                        &json,
                                        &mut ids2,
                                        &mut ref_ids2,
                                    );
                                    entry.0.extend(ids2);
                                    entry.1.extend(ref_ids2);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    for b in buckets.iter_mut() {
        if !b.slot.is_empty() {
            continue;
        }
        let fname = extract_bucket_function_name(&b.name);
        if let Some((idents, ref_ids)) = func_body_refs.get(&fname).cloned() {
            // Try ref ids first
            for rid in ref_ids {
                if let Some(hex) = reg.slot_consts_by_id.get(&rid) {
                    b.slot = short_hex_str(hex);
                    break;
                }
            }
            if b.slot.is_empty() {
                for ident in idents {
                    if let Some(hex) = reg.slot_consts_by_name.get(&ident) {
                        b.slot = short_hex_str(hex);
                        break;
                    }
                }
            }
        }
        // Fallback to schema-id based matching
        if b.slot.is_empty() {
            if let Some((ids, ref_ids)) = schema_body_refs.get(&b.name).cloned() {
                for rid in ref_ids {
                    if let Some(hex) = reg.slot_consts_by_id.get(&rid) {
                        b.slot = short_hex_str(hex);
                        break;
                    }
                }
                if b.slot.is_empty() {
                    for ident in ids {
                        if let Some(hex) = reg.slot_consts_by_name.get(&ident) {
                            b.slot = short_hex_str(hex);
                            break;
                        }
                    }
                }
            }
        }
    }
}

fn collect_idents_and_refs_in_node_json(
    node: &serde_json::Value,
    ids: &mut Vec<String>,
    ref_ids: &mut Vec<isize>,
) {
    if let Some(obj) = node.as_object() {
        if let Some(nt) = obj.get("nodeType").and_then(|v| v.as_str()) {
            if nt == "Identifier" {
                if let Some(name) = obj.get("name").and_then(|v| v.as_str()) {
                    ids.push(name.to_string());
                }
                if let Some(rd) = obj.get("referencedDeclaration").and_then(|v| v.as_i64()) {
                    ref_ids.push(rd as isize);
                }
            }
            // Inline assembly externalReferences entries carry { declaration: <id>, slot: bool, offset: bool }
            if nt == "InlineAssembly" {
                if let Some(ext) = obj.get("externalReferences").and_then(|v| v.as_array()) {
                    for e in ext {
                        if let Some(decl) = e.get("declaration").and_then(|v| v.as_i64()) {
                            ref_ids.push(decl as isize);
                        }
                    }
                }
            }
        }
        for (_k, v) in obj {
            collect_idents_and_refs_in_node_json(v, ids, ref_ids);
        }
    } else if let Some(arr) = node.as_array() {
        for v in arr {
            collect_idents_and_refs_in_node_json(v, ids, ref_ids);
        }
    }
}

fn short_hex_str(hex_in: &str) -> String {
    let s = hex_in.strip_prefix("0x").unwrap_or(hex_in);
    if s.len() > 12 { format!("0x{}…{}", &s[..6], &s[s.len() - 4..]) } else { format!("0x{}", s) }
}

fn documentation_text(docs: Option<solast::Documentation>) -> String {
    match docs {
        Some(solast::Documentation::Structured(sd)) => sd.text.trim().to_string(),
        Some(solast::Documentation::Raw(s)) => s.trim().to_string(),
        None => String::new(),
    }
}

// Removed JSON-based inference helpers in favor of typed AST traversal
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

fn print_abi(abi: &JsonAbi) -> Result<()> {
    if shell::is_json() {
        return print_json(abi);
    }

    let headers = vec![Cell::new("Type"), Cell::new("Signature"), Cell::new("Selector")];
    print_table(headers, |table| {
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
    })
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

pub fn print_storage_layout(storage_layout: Option<&StorageLayout>) -> Result<()> {
    let Some(storage_layout) = storage_layout else {
        eyre::bail!("Could not get storage layout");
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

    print_table(headers, |table| {
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
    })
}

fn print_method_identifiers(method_identifiers: &Option<BTreeMap<String, String>>) -> Result<()> {
    let Some(method_identifiers) = method_identifiers else {
        eyre::bail!("Could not get method identifiers");
    };

    if shell::is_json() {
        return print_json(method_identifiers);
    }

    let headers = vec![Cell::new("Method"), Cell::new("Identifier")];

    print_table(headers, |table| {
        for (method, identifier) in method_identifiers {
            table.add_row([method, identifier]);
        }
    })
}

fn print_errors_events(map: &Map<String, Value>, is_err: bool) -> Result<()> {
    if shell::is_json() {
        return print_json(map);
    }

    let headers = if is_err {
        vec![Cell::new("Error"), Cell::new("Selector")]
    } else {
        vec![Cell::new("Event"), Cell::new("Topic")]
    };
    print_table(headers, |table| {
        for (method, selector) in map {
            table.add_row([method, selector.as_str().unwrap()]);
        }
    })
}

fn print_table(headers: Vec<Cell>, add_rows: impl FnOnce(&mut Table)) -> Result<()> {
    let mut table = Table::new();
    if shell::is_markdown() {
        table.load_preset(ASCII_MARKDOWN);
    } else {
        table.apply_modifier(UTF8_ROUND_CORNERS);
    }
    table.set_header(headers);
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
        eyre::bail!("Could not get IR output");
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
    let mut value_ref = &value;
    if let Some(key) = key
        && let Some(value2) = value.get(key)
    {
        value_ref = value2;
    }
    let s = match value_ref.as_str() {
        Some(s) => s.to_string(),
        None => format!("{value_ref:#}"),
    };
    Ok(s)
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
}
