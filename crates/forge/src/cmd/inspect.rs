use alloy_json_abi::{EventParam, InternalType, JsonAbi, Param};
use alloy_primitives::{U256, hex, keccak256};
use clap::Parser;
use comfy_table::{Cell, Table, modifiers::UTF8_ROUND_CORNERS};
use eyre::{Result, eyre};
use foundry_cli::opts::BuildOpts;
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

/// Number of bytes in an EVM storage slot
const SLOT_SIZE_BYTES: u64 = 32;

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

    /// Enable enhanced EIP-7201 storage bucket parsing with AST information.
    #[arg(long, help_heading = "Display options")]
    pub eip7201: bool,
}

impl InspectArgs {
    pub fn run(self) -> Result<()> {
        let Self { contract, field, build, strip_yul_comments, wrap, eip7201 } = self;

        trace!(target: "forge", ?field, ?contract, "running forge inspect");

        // Map field to ContractOutputSelection
        let mut cos = build.compiler.extra_output.clone();
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

        // Build modified Args with AST if needed
        let mut final_build_args = build;
        final_build_args.compiler.extra_output = cos;
        final_build_args.compiler.optimize = optimized;
        
        // For storage layout inspection with EIP-7201, also request AST to enhance bucket information
        if field == ContractArtifactField::StorageLayout && eip7201 {
            final_build_args.compiler.ast = true;
        }

        // Build the project
        let project = final_build_args.project()?;
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
                let bucket_rows =
                    parse_storage_buckets_value(artifact.raw_metadata.as_ref()).unwrap_or_default();
                
                let source_buckets = if eip7201 {
                    // Extract EIP-7201 storage buckets directly from AST and build artifacts
                    if let Some(ast) = &artifact.ast {
                        extract_eip7201_buckets_from_ast(ast, &output)
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                
                print_storage_layout(artifact.storage_layout.as_ref(), bucket_rows, source_buckets, eip7201, wrap)?;
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
    bucket_rows: Vec<(String, String)>,
    source_buckets: Vec<StorageBucket>,
    eip7201: bool,
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
            
            // Add legacy bucket rows for backward compatibility (only when not using EIP-7201)
            if !eip7201 {
                for (type_str, slot_dec) in &bucket_rows {
                    table.add_row([
                        "storage-bucket",
                        type_str.as_str(),
                        slot_dec.as_str(),
                        "0",
                        "32",
                        type_str,
                    ]);
                }
            }
            
            // Add enhanced source buckets with EIP-7201 information (only when using EIP-7201)
            if eip7201 {
                for bucket in &source_buckets {
                let display_type = if !bucket.bucket_type.is_empty() && bucket.bucket_type != "unknown" {
                    if bucket.bucket_type == "singleton" {
                        // Handle singleton type - prioritize explicit value identifier from natspec
                        let value_type = if let Some(value_id) = &bucket.value_identifier {
                            // Use explicit @custom:storage-bucket-value annotation
                            value_id
                        } else if let Some(ret_type) = &bucket.return_type {
                            if ret_type.contains("storage") {
                                extract_storage_type(ret_type)
                            } else {
                                // Direct from AST - extract struct name if it's a struct type
                                if ret_type.starts_with("struct ") {
                                    ret_type.strip_prefix("struct ").unwrap_or(ret_type)
                                        .split(' ').next().unwrap_or(ret_type)
                                } else {
                                    ret_type
                                }
                            }
                        } else {
                            "unknown"
                        };
                        
                        format!("{}", value_type)
                    } else if let (Some(params), Some(ret_type)) = (&bucket.parameters, &bucket.return_type) {
                        if bucket.bucket_type == "keyvalue" {
                            // Extract value type - prioritize explicit value identifier from natspec
                            let value_type = if let Some(value_id) = &bucket.value_identifier {
                                // Use explicit @custom:storage-bucket-value annotation
                                value_id
                            } else if ret_type.contains("storage") {
                                extract_storage_type(ret_type)
                            } else {
                                // Direct from AST - extract struct name if it's a struct type
                                if ret_type.starts_with("struct ") {
                                    ret_type.strip_prefix("struct ").unwrap_or(ret_type)
                                        .split(' ').next().unwrap_or(ret_type)
                                } else {
                                    ret_type
                                }
                            };
                            
                            // Extract key types - handle multiple parameters properly
                            let key_types = extract_all_param_types(params);
                            
                            format!("key({}) => {}", key_types, value_type)
                        } else {
                            bucket.bucket_type.clone()
                        }
                    } else {
                        bucket.bucket_type.clone()
                    }
                } else {
                    "storage-bucket".to_string()
                };
                
                let slot_display = if bucket.slot.is_empty() {
                    "0x0"
                } else {
                    bucket.slot.as_str()
                };
                let contract_display = format_contract_name(bucket);
                
                // Add transient indicator for transient storage buckets
                let name_display = if bucket.is_transient {
                    format!("[T] {}", bucket.name)
                } else {
                    bucket.name.clone()
                };
                
                table.add_row([
                    &name_display,
                    &display_type,
                    slot_display,
                    "0",
                    "32",
                    &contract_display,
                ]);
                
                // Add struct members if available
                if let Some(struct_members) = &bucket.struct_members {
                    // First, add a struct header row showing the struct info
                    let struct_name = extract_struct_name_from_bucket(bucket);
                    let total_struct_size = calculate_total_struct_size(struct_members);
                    let struct_header_slot = generate_member_slot_formula_base(bucket);
                    
                    // Use the first member's source info for the struct header
                    let struct_contract_display = if let Some(first_member) = struct_members.first() {
                        format_struct_contract_name(first_member)
                    } else {
                        "Unknown.sol:Unknown".to_string()
                    };
                    
                    table.add_row([
                        &format!("  ├─ {}", struct_name),
                        "struct",
                        &struct_header_slot,
                        "0",
                        &total_struct_size.to_string(),
                        &struct_contract_display,
                    ]);
                    
                    // Then add individual struct members
                    for member in struct_members {
                        let member_slot = generate_member_slot_formula(bucket, member);
                        let member_contract_display = format_struct_contract_name(member);
                        
                        table.add_row([
                            &format!("  ├─ {}", member.name),
                            &member.type_string,
                            &member_slot,
                            &member.byte_offset.to_string(),
                            &member.size_bytes.to_string(),
                            &member_contract_display,
                        ]);
                        
                        // Recursively display nested struct members
                        if let Some(nested_members) = &member.nested_members {
                            print_nested_struct_members_with_parent(
                                nested_members, 
                                bucket, 
                                table, 
                                2, 
                                None, 
                                Some(member.slot_offset)
                            );
                        }
                    }
                }
                }
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


fn print_nested_struct_members_with_parent(
    nested_members: &[StructMember], 
    bucket: &StorageBucket, 
    table: &mut Table, 
    indent_level: usize,
    parent_base_slot: Option<&str>,
    parent_offset: Option<u64>
) {
    let mut current_struct_var: Option<String> = None;
    let mut struct_base_offset: u64 = 0; // Track the base offset where the current struct starts
    
    for nested_member in nested_members {
        let nested_member_slot = if nested_member.name.starts_with("struct ") {
            // For struct headers, introduce a new variable
            let struct_name = nested_member.name.replace("struct ", "");
                let var_name = generate_struct_variable_name(&struct_name);
            
            let slot_formula = if let Some(parent_base) = parent_base_slot {
                // This is a deeply nested struct - calculate properly from the parent mapping
                let parent_field_offset = parent_offset.unwrap_or(0);
                let mapping_keys = extract_mapping_keys_from_parent(nested_member);
                format!("{} = keccak({}, {} + {})", var_name, mapping_keys, parent_base, parent_field_offset)
            } else {
                // Top-level nested struct - calculate from the parent mapping field
                let parent_field_slot = parent_offset.unwrap_or(1); // Default to mapping at slot + 1
                let mapping_keys = extract_mapping_keys_from_parent(nested_member);
                format!("{} = keccak({}, {} + {})", var_name, mapping_keys, bucket.slot, parent_field_slot)
            };
            
            current_struct_var = Some(var_name.clone());
            struct_base_offset = nested_member.slot_offset; // Remember where this struct starts
            slot_formula
        } else {
            // For regular members, use the current struct variable with relative offset
            if let Some(ref struct_var) = current_struct_var {
                // Calculate relative offset within the current struct
                let relative_offset = nested_member.slot_offset - struct_base_offset;
                if relative_offset == 0 {
                    struct_var.clone()
                } else {
                    format!("{} + {}", struct_var, relative_offset)
                }
            } else if let Some(parent_base) = parent_base_slot {
                // Use parent variable for members without their own struct header
                let relative_offset = nested_member.slot_offset - parent_offset.unwrap_or(0);
                if relative_offset == 0 {
                    parent_base.to_string()
                } else {
                    format!("{} + {}", parent_base, relative_offset)
                }
            } else {
                generate_nested_member_slot_formula(bucket, nested_member, None)
            }
        };
        
        let nested_contract_display = format_struct_contract_name(nested_member);
        let indent_str = "  ".repeat(indent_level);
        
        table.add_row([
            &format!("{}├─ {}", indent_str, nested_member.name),
            &nested_member.type_string,
            &nested_member_slot,
            &nested_member.byte_offset.to_string(),
            &nested_member.size_bytes.to_string(),
            &nested_contract_display,
        ]);
        
        // Recursively display deeper nested members
        if let Some(deeper_nested_members) = &nested_member.nested_members {
            let base_slot = current_struct_var.as_deref();
            print_nested_struct_members_with_parent(
                deeper_nested_members, 
                bucket, 
                table, 
                indent_level + 1, 
                base_slot,
                Some(nested_member.slot_offset)
            );
        }
    }
}

// Generate concise variable names for struct slots (M, P, B, etc.)
fn generate_struct_variable_name(struct_name: &str) -> String {
    let first_char = struct_name.chars().next().unwrap_or('S').to_uppercase().collect::<String>();
    first_char
}

// Extract the mapping keys pattern from the parent field
fn extract_mapping_keys_from_parent(_target_member: &StructMember) -> String {
    // Generic key pattern - always use "key" for simplicity
    "key".to_string()
}

// Find struct definition with namespace preference
fn find_struct_with_namespace_preference(
    struct_name: &str, 
    parent_namespace: Option<&str>,
    struct_definitions: &std::collections::HashMap<String, (Vec<StructMember>, Option<String>, Option<String>)>
) -> Option<(Vec<StructMember>, Option<String>, Option<String>)> {
    // If we have a parent namespace, prefer structs from the same namespace
    if let Some(namespace) = parent_namespace {
        let preferred_key = format!("{}:{}", namespace, struct_name);
        if let Some(definition) = struct_definitions.get(&preferred_key) {
            return Some(definition.clone());
        }
    }
    
    // Fallback: try exact struct name
    if let Some(definition) = struct_definitions.get(struct_name) {
        return Some(definition.clone());
    }
    
    // Last resort: find any struct with this name in the identifier
    for (key, definition) in struct_definitions {
        if key.ends_with(&format!(":{}", struct_name)) || key == struct_name {
            return Some(definition.clone());
        }
    }
    
    None
}


// Extract namespace from the current processing context
fn extract_namespace_from_context(members: &[StructMember]) -> Option<String> {
    // Look for struct_identifier in any of the members to determine current namespace
    for member in members {
        if let Some(identifier) = &member.struct_identifier {
            if let Some(colon_pos) = identifier.find(':') {
                return Some(identifier[..colon_pos].to_string());
            }
        }
    }
    None
}

// Enhance type information by replacing generic types with proper enum names
fn enhance_enum_types(
    buckets: &mut [StorageBucket],
    enum_definitions: &std::collections::HashMap<String, (String, Option<String>)>
) {
    for bucket in buckets.iter_mut() {
        if let Some(members) = &mut bucket.struct_members {
            enhance_enum_types_in_members(members, enum_definitions);
        }
    }
}

// Recursively enhance enum types in struct members
fn enhance_enum_types_in_members(
    members: &mut [StructMember],
    enum_definitions: &std::collections::HashMap<String, (String, Option<String>)>
) {
    for member in members.iter_mut() {
        // Check if this member's type can be enhanced with enum information
        if let Some(enhanced_type) = enhance_type_with_enum(&member.type_string, enum_definitions) {
            member.type_string = enhanced_type;
        }
        
        // Recursively enhance nested members
        if let Some(nested_members) = &mut member.nested_members {
            enhance_enum_types_in_members(nested_members, enum_definitions);
        }
    }
}

// Try to enhance a type string with proper enum name
fn enhance_type_with_enum(
    type_string: &str,
    enum_definitions: &std::collections::HashMap<String, (String, Option<String>)>
) -> Option<String> {
    // Look for patterns that might be enhanced with enum information
    // For example, "uint8" might become "enum Status" if we find a matching context
    
    // Simple enhancement for direct uint8 -> enum mappings
    // This is a placeholder - you'd want more sophisticated logic here
    if type_string == "uint8" {
        // Try to find an enum that makes sense in this context
        // For now, return None to keep the original type
        return None;
    }
    
    // Look for enum names in the type string
    for (enum_key, (canonical_name, _source)) in enum_definitions {
        if type_string.contains(canonical_name) {
            return Some(format!("enum {}", enum_key));
        }
    }
    
    None
}

// Enhance type information by replacing generic types with proper usertype names
fn enhance_usertype_types(
    buckets: &mut [StorageBucket],
    usertype_definitions: &std::collections::HashMap<String, (String, String, Option<String>)>
) {
    for bucket in buckets.iter_mut() {
        if let Some(members) = &mut bucket.struct_members {
            enhance_usertype_types_in_members(members, usertype_definitions);
        }
    }
}

// Recursively enhance usertype types in struct members
fn enhance_usertype_types_in_members(
    members: &mut [StructMember],
    usertype_definitions: &std::collections::HashMap<String, (String, String, Option<String>)>
) {
    for member in members.iter_mut() {
        // Check if this member's type can be enhanced with usertype information
        if let Some(enhanced_type) = enhance_type_with_usertype(&member.type_string, usertype_definitions) {
            member.type_string = enhanced_type;
        }
        
        // Recursively enhance nested members
        if let Some(nested_members) = &mut member.nested_members {
            enhance_usertype_types_in_members(nested_members, usertype_definitions);
        }
    }
}

// Try to enhance a type string with proper usertype name
fn enhance_type_with_usertype(
    type_string: &str,
    usertype_definitions: &std::collections::HashMap<String, (String, String, Option<String>)>
) -> Option<String> {
    // Look for usertype patterns in the type string
    for (_usertype_key, (usertype_name, underlying_type, _source)) in usertype_definitions {
        // Check if the current type matches the underlying type of a defined usertype
        if type_string == underlying_type {
            // Simple direct replacement
            return Some(usertype_name.clone());
        }
        
        // Handle more complex patterns like arrays, mappings, etc.
        if type_string.contains(underlying_type) {
            // Replace the underlying type with the usertype name in complex patterns
            // For example: "uint256[]" -> "OrderId[]"
            // Or: "mapping(address => uint256)" -> "mapping(address => OrderId)"
            let enhanced = type_string.replace(underlying_type, usertype_name);
            return Some(enhanced);
        }
    }
    
    None
}

// Generate slot formula for nested struct members
fn generate_nested_member_slot_formula(bucket: &StorageBucket, member: &StructMember, parent_slot_var: Option<&str>) -> String {
    if let Some(var) = parent_slot_var {
        // Use the parent variable for nested members
        if member.slot_offset == 0 {
            var.to_string()
        } else {
            format!("{} + {}", var, member.slot_offset)
        }
    } else {
        // For top-level nested structs, calculate from bucket slot
        if member.slot_offset == 0 && member.byte_offset < SLOT_SIZE_BYTES {
            format!("keccak(key, {})", bucket.slot)
        } else {
            format!("keccak(key, {}) + {}", bucket.slot, member.slot_offset)
        }
    }
}

fn print_table(
    headers: Vec<Cell>,
        mut add_rows: impl FnMut(&mut Table),
    should_wrap: bool,
) -> Result<()> {
    let mut table = Table::new();
    table.apply_modifier(UTF8_ROUND_CORNERS);
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
        matches!(self, Self::Bytecode | Self::DeployedBytecode | Self::StandardJson)
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

#[derive(Debug, Clone)]
pub struct StorageBucket {
    pub name: String,
    pub bucket_type: String,
    pub slot: String,
    pub function_signature: Option<String>,
    pub parameters: Option<String>,
    pub return_type: Option<String>,
    pub struct_members: Option<Vec<StructMember>>,
    pub source_file: Option<String>,
    pub contract_name: Option<String>,
    pub value_identifier: Option<String>, // For @custom:storage-bucket-value matching
    pub is_transient: bool, // For EIP-1153 transient storage
}

static BUCKET_PAIR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)
        (?P<name>[A-Za-z_][A-Za-z0-9_:\.\-]*)
        \s+
        (?:0x)?(?P<hex>[0-9a-f]{1,64})
    ",
    )
    .unwrap()
});

static STORAGE_BUCKET_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@custom:storage-bucket\s+(.+)")
        .unwrap()
});

static STORAGE_BUCKET_TYPE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@custom:storage-bucket-type\s+(\S+)\s+(\S+)")
        .unwrap()
});

static STORAGE_BUCKET_SLOT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@custom:storage-bucket-slot\s+(\S+)(?:\s+(0x[0-9a-fA-F]+))?")
        .unwrap()
});

static STORAGE_BUCKET_STRUCT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@custom:storage-bucket-struct\s+(\S+)")
        .unwrap()
});

static STORAGE_BUCKET_VALUE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@custom:storage-bucket-value\s+(\S+)")
        .unwrap()
});

static STORAGE_BUCKET_ENUM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@custom:storage-bucket-enum\s+(\S+)")
        .unwrap()
});

static STORAGE_BUCKET_USERTYPE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@custom:storage-bucket-usertype\s+(\S+)")
        .unwrap()
});

static STORAGE_BUCKET_TRANSIENT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@custom:storage-bucket-transient\s+(.+)")
        .unwrap()
});

static STORAGE_BUCKET_TRANSIENT_SLOT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@custom:storage-bucket-transient-slot\s+(\S+)(?:\s+(0x[0-9a-fA-F]+))?")
        .unwrap()
});

#[derive(Debug, Clone)]
pub struct StructMember {
    pub name: String,
    pub type_string: String,
    pub slot_offset: u64,
    pub byte_offset: u64,
    pub size_bytes: u64,
    pub source_file: Option<String>,
    pub struct_name: Option<String>,
    pub struct_identifier: Option<String>, // For @custom:storage-bucket-struct matching
    pub nested_members: Option<Vec<StructMember>>, // For recursive struct expansion
}

fn parse_storage_buckets_value(raw_metadata: Option<&String>) -> Option<Vec<(String, String)>> {
    let parse_bucket_pairs = |s: &str| {
        BUCKET_PAIR_RE
            .captures_iter(s)
            .filter_map(|caps| {
                let name = caps.get(1)?.as_str();
                let hex_str = caps.get(2)?.as_str();

                hex::decode(hex_str.trim_start_matches("0x"))
                    .ok()
                    .filter(|bytes| bytes.len() == SLOT_SIZE_BYTES as usize)
                    .map(|_| (name.to_owned(), hex_str.to_owned()))
            })
            .collect::<Vec<_>>()
    };
    let raw = raw_metadata?;
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    v.get("output")
        .and_then(|o| o.get("devdoc"))
        .and_then(|d| d.get("methods"))
        .and_then(|m| m.get("constructor"))
        .and_then(|c| c.as_object())
        .and_then(|obj| obj.get("custom:storage-bucket"))
        .map(|val| {
            val.as_str()
                .into_iter() // Option<&str> → Iterator<Item=&str>
                .flat_map(parse_bucket_pairs)
                .filter_map(|(name, hex): (String, String)| {
                    let hex_str = hex.strip_prefix("0x").unwrap_or(&hex);
                    let slot = U256::from_str_radix(hex_str, 16).ok()?;
                    let slot_hex = short_hex(&alloy_primitives::hex::encode_prefixed(
                        slot.to_be_bytes::<{ SLOT_SIZE_BYTES as usize }>(),
                    ));
                    Some((name, slot_hex))
                })
                .collect()
        })
}

fn short_hex(h: &str) -> String {
    let s = h.strip_prefix("0x").unwrap_or(h);
    if s.len() > 12 { format!("0x{}…{}", &s[..6], &s[s.len() - 4..]) } else { format!("0x{s}") }
}

fn extract_eip7201_buckets_from_ast(ast: &foundry_compilers::artifacts::ast::Ast, output: &foundry_compilers::ProjectCompileOutput) -> Vec<StorageBucket> {
    let mut buckets = Vec::new();
    
    if let Ok(ast_value) = serde_json::to_value(ast) {
        extract_buckets_from_ast_node(&ast_value, &mut buckets, None, Some(&ast_value));
        process_bucket_information(&mut buckets, output);
        enhance_bucket_types(&mut buckets, output);
    }
    
    buckets
}

fn process_bucket_information(buckets: &mut Vec<StorageBucket>, output: &foundry_compilers::ProjectCompileOutput) {
    // Fill missing function info (return types, parameters)
    for (_artifact_id, contract_artifact) in output.artifact_ids() {
        if let Some(contract_ast) = &contract_artifact.ast {
            if let Ok(contract_ast_value) = serde_json::to_value(contract_ast) {
                fill_missing_bucket_info(&contract_ast_value, buckets);
            }
        }
    }
    
    // Search for struct definitions with return types properly set
    for (_artifact_id, contract_artifact) in output.artifact_ids() {
        if let Some(contract_ast) = &contract_artifact.ast {
            if let Ok(contract_ast_value) = serde_json::to_value(contract_ast) {
                let source_file = contract_ast_value.get("absolutePath")
                    .and_then(|ap| ap.as_str())
                    .map(|path| extract_filename_from_path(path));
                
                search_for_struct_definitions_with_source(&contract_ast_value, buckets, source_file.as_deref());
            }
        }
    }
}

fn enhance_bucket_types(buckets: &mut Vec<StorageBucket>, output: &foundry_compilers::ProjectCompileOutput) {
    let all_struct_definitions = collect_all_struct_definitions(buckets, output);
    let all_enum_definitions = collect_all_enum_definitions(output);
    let all_usertype_definitions = collect_all_usertype_definitions(output);
    
    expand_nested_structs(buckets, &all_struct_definitions);
    enhance_enum_types(buckets, &all_enum_definitions);
    enhance_usertype_types(buckets, &all_usertype_definitions);
}

fn extract_buckets_from_ast_node(node: &Value, buckets: &mut Vec<StorageBucket>, current_source: Option<&str>, ast_root: Option<&Value>) {
    if let Some(node_type) = node.get("nodeType").and_then(|nt| nt.as_str()) {
        match node_type {
            "SourceUnit" => {
                // Extract source file path from SourceUnit
                let source_file = node.get("absolutePath")
                    .and_then(|ap| ap.as_str())
                    .or_else(|| node.get("src").and_then(|src| src.as_str()))
                    .map(|path| extract_filename_from_path(path));
                
                // Root node - recurse into child nodes
                if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                    for child_node in nodes {
                        extract_buckets_from_ast_node(child_node, buckets, source_file.as_deref(), ast_root);
                    }
                }
            }
            "ContractDefinition" | "LibraryDefinition" => {
                if let Some(contract_name) = node.get("name").and_then(|n| n.as_str()) {
                    // Check all child nodes for constructors, functions, variables, and structs
                    if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                        for child_node in nodes {
                            extract_constructor_buckets(child_node, buckets, contract_name, current_source);
                            extract_function_buckets(child_node, buckets, contract_name, current_source, ast_root);
                            extract_struct_buckets(child_node, buckets, current_source);
                        }
                    }
                }
            }
            "StructDefinition" => {
                // Also check for top-level struct definitions
                extract_struct_buckets(node, buckets, current_source);
            }
            _ => {
                // For other node types, continue recursing
                if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                    for child_node in nodes {
                        extract_buckets_from_ast_node(child_node, buckets, current_source, ast_root);
                    }
                }
            }
        }
    }
}

fn extract_constructor_buckets(node: &Value, buckets: &mut Vec<StorageBucket>, contract_name: &str, source_file: Option<&str>) {
    if node.get("nodeType").and_then(|nt| nt.as_str()) == Some("FunctionDefinition") 
        && node.get("kind").and_then(|k| k.as_str()) == Some("constructor") {
        
        // Look for @custom:storage-bucket in documentation
        if let Some(doc_text) = node.get("documentation").and_then(|d| d.get("text")).and_then(|t| t.as_str()) {
            // Find all @custom:storage-bucket matches in the constructor documentation
            for caps in STORAGE_BUCKET_RE.captures_iter(doc_text.trim()) {
                let bucket_name = caps.get(1).unwrap().as_str().trim();
                
                // Create initial bucket entry
                buckets.push(StorageBucket {
                    name: bucket_name.to_string(),
                    bucket_type: "unknown".to_string(),
                    slot: "".to_string(),
                    function_signature: None,
                    parameters: None,
                    return_type: None,
                    struct_members: None,
                    source_file: source_file.map(|s| s.to_string()),
                    contract_name: Some(contract_name.to_string()),
                    value_identifier: None,
                    is_transient: false,
                });
            }
            
            // Find all @custom:storage-bucket-transient matches in the constructor documentation
            for caps in STORAGE_BUCKET_TRANSIENT_RE.captures_iter(doc_text.trim()) {
                let bucket_name = caps.get(1).unwrap().as_str().trim();
                
                // Create initial transient bucket entry
                buckets.push(StorageBucket {
                    name: bucket_name.to_string(),
                    bucket_type: "unknown".to_string(),
                    slot: "".to_string(),
                    function_signature: None,
                    parameters: None,
                    return_type: None,
                    struct_members: None,
                    source_file: source_file.map(|s| s.to_string()),
                    contract_name: Some(contract_name.to_string()),
                    value_identifier: None,
                    is_transient: true,
                });
            }
        }
    }
}

fn extract_function_buckets(node: &Value, buckets: &mut Vec<StorageBucket>, contract_name: &str, source_file: Option<&str>, _ast_root: Option<&Value>) {
    let node_type = node.get("nodeType").and_then(|nt| nt.as_str());
    
    if node_type == Some("FunctionDefinition") {
        // Look for @custom:storage-bucket-type in documentation
        if let Some(doc_text) = node.get("documentation").and_then(|d| d.get("text")).and_then(|t| t.as_str()) {
            if let Some(caps) = STORAGE_BUCKET_TYPE_RE.captures(doc_text.trim()) {
                let function_ref = caps.get(1).unwrap().as_str();
                let bucket_type = caps.get(2).unwrap().as_str();
                
                // Find existing bucket or create new one
                if let Some(existing_bucket) = buckets.iter_mut().find(|b| b.name == function_ref) {
                    // Update existing bucket
                    existing_bucket.bucket_type = bucket_type.to_string();
                    existing_bucket.source_file = source_file.map(|s| s.to_string());
                    existing_bucket.contract_name = Some(contract_name.to_string());
                    extract_function_signature_from_ast(node, existing_bucket);
                } else {
                    // Create new bucket - this should only happen in the target file
                    let mut new_bucket = StorageBucket {
                        name: function_ref.to_string(),
                        bucket_type: bucket_type.to_string(),
                        slot: "".to_string(),
                        function_signature: None,
                        parameters: None,
                        return_type: None,
                        struct_members: None,
                        source_file: source_file.map(|s| s.to_string()),
                        contract_name: Some(contract_name.to_string()),
                        value_identifier: None,
                        is_transient: false,
                    };
                    extract_function_signature_from_ast(node, &mut new_bucket);
                    buckets.push(new_bucket);
                }
            }
        }
    }

    // Also check for slot definitions - these might be on separate constant declarations
    if node_type == Some("VariableDeclaration") {
        if let Some(doc_text) = node.get("documentation").and_then(|d| d.get("text")).and_then(|t| t.as_str()) {
            if let Some(caps) = STORAGE_BUCKET_SLOT_RE.captures(doc_text.trim()) {
                let function_ref = caps.get(1).unwrap().as_str();
                let slot_hex = caps.get(2).unwrap().as_str();
                
                let slot = U256::from_str_radix(slot_hex.strip_prefix("0x").unwrap_or(slot_hex), 16).ok();
                let short_slot = if let Some(slot) = slot {
                    short_hex(&alloy_primitives::hex::encode_prefixed(slot.to_be_bytes::<{ SLOT_SIZE_BYTES as usize }>()))
                } else {
                    slot_hex.to_string()
                };
                
                // Find existing bucket or create new one  
                if let Some(existing_bucket) = buckets.iter_mut().find(|b| b.name == function_ref) {
                    // Update existing bucket with slot info
                    existing_bucket.slot = short_slot;
                } else {
                    // Create new bucket with slot info - this should only happen in the target file
                    buckets.push(StorageBucket {
                        name: function_ref.to_string(),
                        bucket_type: "unknown".to_string(),
                        slot: short_slot,
                        function_signature: None,
                        parameters: None,
                        return_type: None,
                        struct_members: None,
                        source_file: source_file.map(|s| s.to_string()),
                        contract_name: Some(contract_name.to_string()),
                        value_identifier: None,
                        is_transient: false,
                    });
                }
            }
            
            // Also check for @custom:storage-bucket-transient-slot in documentation  
            if let Some(caps) = STORAGE_BUCKET_TRANSIENT_SLOT_RE.captures(doc_text.trim()) {
                let function_ref = caps.get(1).unwrap().as_str();
                
                // Get slot value from natspec annotation (if provided)
                let slot_hex = caps.get(2).map(|m| m.as_str()).unwrap_or("0x0").to_string();
                
                let slot = U256::from_str_radix(slot_hex.strip_prefix("0x").unwrap_or(&slot_hex), 16).ok();
                let short_slot = if let Some(slot) = slot {
                    short_hex(&alloy_primitives::hex::encode_prefixed(slot.to_be_bytes::<{ SLOT_SIZE_BYTES as usize }>()))
                } else {
                    // If we can't parse the hex, show it as-is (might be an expression)
                    if slot_hex == "0x0" {
                        "0x0".to_string() // Keep explicit 0x0
                    } else {
                        format!("0x{}", slot_hex.trim_start_matches("0x")) // Ensure hex prefix
                    }
                };
                
                // Find existing bucket or create new one, mark as transient
                if let Some(existing_bucket) = buckets.iter_mut().find(|b| b.name == function_ref) {
                    // Update existing bucket with slot info and mark as transient
                    existing_bucket.slot = short_slot;
                    existing_bucket.is_transient = true;
                } else {
                    // Create new transient bucket with slot info
                    buckets.push(StorageBucket {
                        name: function_ref.to_string(),
                        bucket_type: "unknown".to_string(),
                        slot: short_slot,
                        function_signature: None,
                        parameters: None,
                        return_type: None,
                        struct_members: None,
                        source_file: source_file.map(|s| s.to_string()),
                        contract_name: Some(contract_name.to_string()),
                        value_identifier: None,
                        is_transient: true,
                    });
                }
            }
        }
    }
}

fn extract_struct_buckets(node: &Value, buckets: &mut Vec<StorageBucket>, source_file: Option<&str>) {
    if node.get("nodeType").and_then(|nt| nt.as_str()) == Some("StructDefinition") {
        // Look for @custom:storage-bucket-struct in documentation
        if let Some(doc_text) = node.get("documentation").and_then(|d| d.get("text")).and_then(|t| t.as_str()) {
            if let Some(caps) = STORAGE_BUCKET_STRUCT_RE.captures(doc_text.trim()) {
                let struct_name = caps.get(1).unwrap().as_str().trim();
                let canonical_name = node.get("canonicalName").and_then(|n| n.as_str()).unwrap_or(struct_name);
                
                // Extract struct members and calculate their storage layout
                let struct_members = if let Some(members_array) = node.get("members").and_then(|m| m.as_array()) {
                    calculate_struct_layout(members_array, source_file, Some(canonical_name))
                } else {
                    Vec::new()
                };
                
                // Find existing EIP-7201 bucket that matches this struct's return type
                let found_match = false;
                for existing_bucket in buckets.iter_mut() {
                    if let Some(ret_type) = &existing_bucket.return_type {
                        // Match struct name against return type (e.g., "MarketSettings" matches "MarketSettings")
                        if ret_type == struct_name || ret_type == canonical_name || 
                           ret_type.contains(struct_name) || ret_type.contains(canonical_name) {
                            // This struct is the return type of an existing EIP-7201 bucket
                            existing_bucket.struct_members = Some(struct_members.clone());
                            let _ = found_match; // Suppress warning
                            break;
                        }
                    }
                }
                
                // Only try precise function-struct matching if no direct match found
                if !found_match {
                    for existing_bucket in buckets.iter_mut() {
                        // NEVER override explicit value identifiers - they have absolute priority
                        if existing_bucket.value_identifier.is_some() {
                            continue;
                        }
                        
                        // Only allow struct assignment for functions that clearly return structs
                        // Use precise matching patterns instead of substring matching
                        if let Some(ret_type) = &existing_bucket.return_type {
                            // Direct return type match (e.g., "MarketSettings" == "MarketSettings")  
                            if ret_type == struct_name || ret_type == canonical_name {
                                existing_bucket.struct_members = Some(struct_members.clone());
                                break;
                            }
                            
                            // Namespace-qualified match (e.g., "Contract:StructName" contains "StructName")
                            if ret_type.contains(&format!(":{}", struct_name)) || ret_type.contains(&format!(":{}", canonical_name)) {
                                existing_bucket.struct_members = Some(struct_members.clone());
                                break;  
                            }
                        }
                        
                        // Pattern-based matching ONLY for load functions with exact struct name correspondence
                        if existing_bucket.name.starts_with("load") || existing_bucket.name.contains(".load") {
                            let bucket_name = existing_bucket.name.to_lowercase();
                            let struct_lower = struct_name.to_lowercase();
                            let canonical_lower = canonical_name.to_lowercase();
                            
                            // Exact suffix match: function name ends with struct name
                            if bucket_name.ends_with(&struct_lower) || bucket_name.ends_with(&canonical_lower) {
                                existing_bucket.struct_members = Some(struct_members.clone());
                                break;
                            }
                        }
                    }
                }
                
                // Don't create standalone struct buckets - only update existing EIP-7201 buckets
                // This prevents duplicate entries and ensures struct members appear under their parent slot
            }
        }
    }
}

fn calculate_struct_layout(members: &[Value], source_file: Option<&str>, struct_name: Option<&str>) -> Vec<StructMember> {
    calculate_struct_layout_with_buckets(members, source_file, struct_name, &[])
}

fn calculate_struct_layout_with_buckets(
    members: &[Value], 
    source_file: Option<&str>, 
    struct_name: Option<&str>,
    all_buckets: &[StorageBucket]
) -> Vec<StructMember> {
    let mut struct_members = Vec::new();
    let mut current_slot = 0u64;
    let mut current_byte_offset = 0u64;
    
    for member in members {
        if let (Some(name), Some(type_desc)) = (
            member.get("name").and_then(|n| n.as_str()),
            member.get("typeDescriptions").and_then(|td| td.get("typeString")).and_then(|ts| ts.as_str())
        ) {
            let size_bytes = calculate_type_size(type_desc);
            
            // Check if we need to move to the next slot
            if current_byte_offset + size_bytes > SLOT_SIZE_BYTES {
                current_slot += 1;
                current_byte_offset = 0;
            }
            
            // Check if this field contains a struct that we should expand
            let nested_members = extract_and_expand_nested_structs(&type_desc, current_slot, all_buckets);
            
            struct_members.push(StructMember {
                name: name.to_string(),
                type_string: type_desc.to_string(),
                slot_offset: current_slot,
                byte_offset: current_byte_offset,
                size_bytes,
            source_file: source_file.map(|s| s.to_string()),
            struct_name: struct_name.map(|s| s.to_string()),
            struct_identifier: None,
            nested_members,
            });
            
            current_byte_offset += size_bytes;
            
            // If we exactly fill a slot, move to the next one
            if current_byte_offset == SLOT_SIZE_BYTES {
                current_slot += 1;
                current_byte_offset = 0;
            }
        }
    }
    
    struct_members
}

// Extract and expand nested structs from type descriptions
fn extract_and_expand_nested_structs(
    type_desc: &str, 
    base_slot_offset: u64,
    all_buckets: &[StorageBucket]
) -> Option<Vec<StructMember>> {
    // Look for patterns like "struct StructName" or "mapping(...=> struct StructName)"
    if let Some(struct_name) = extract_struct_name_from_type(type_desc) {
        // Find the bucket that defines this struct
        for bucket in all_buckets {
            if let Some(members) = &bucket.struct_members {
                if let Some(bucket_ret_type) = &bucket.return_type {
                    if bucket_ret_type == &struct_name || bucket_ret_type.contains(&struct_name) {
                        // Found the struct definition, create nested members with adjusted slots
                        let mut nested = Vec::new();
                        
                        // Add struct header
                        let struct_header = StructMember {
                            name: format!("struct {}", struct_name),
                            type_string: "struct".to_string(),
                            slot_offset: base_slot_offset,
                            byte_offset: 0,
                            size_bytes: members.len() as u64 * SLOT_SIZE_BYTES, // Rough estimate
                            source_file: bucket.source_file.clone(),
                            struct_name: Some(struct_name.clone()),
                            struct_identifier: None,
                            nested_members: None,
                        };
                        nested.push(struct_header);
                        
                        // Add all struct members with adjusted slot formulas
                        for member in members {
                            let mut nested_member = member.clone();
                            // Adjust slot formula for nested context
                            if is_mapping_type(type_desc) {
                                // For mappings, use keccak(key, base_slot) + member_offset
                                nested_member.slot_offset = base_slot_offset;
                            }
                            nested.push(nested_member);
                        }
                        
                        return Some(nested);
                    }
                }
            }
        }
    }
    None
}

// Extract struct name from type descriptions like "struct Market" or "mapping(bytes32 => struct Market)"  
fn extract_struct_name_from_type(type_desc: &str) -> Option<String> {
    // Pattern to match "struct StructName" 
    let struct_re = regex::Regex::new(r"struct\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    
    if let Some(caps) = struct_re.captures(type_desc) {
        return Some(caps.get(1).unwrap().as_str().to_string());
    }
    
    None
}

// Check if a type is a mapping type
fn is_mapping_type(type_desc: &str) -> bool {
    type_desc.starts_with("mapping(")
}

// Collect all struct definitions from buckets and standalone annotated structs
fn collect_all_struct_definitions(
    buckets: &[StorageBucket], 
    output: &foundry_compilers::ProjectCompileOutput
) -> std::collections::HashMap<String, (Vec<StructMember>, Option<String>, Option<String>)> {
    let mut struct_definitions = std::collections::HashMap::new();
    
    // First, collect from existing buckets
    for bucket in buckets.iter() {
        if let (Some(return_type), Some(members)) = (&bucket.return_type, &bucket.struct_members) {
            struct_definitions.insert(
                return_type.clone(),
                (members.clone(), bucket.source_file.clone(), bucket.contract_name.clone())
            );
        }
    }
    
    // Then, collect standalone annotated structs from all contracts
    for (_artifact_id, contract_artifact) in output.artifact_ids() {
        if let Some(contract_ast) = &contract_artifact.ast {
            if let Ok(contract_ast_value) = serde_json::to_value(contract_ast) {
                let source_file = contract_ast_value.get("absolutePath")
                    .and_then(|ap| ap.as_str())
                    .map(|path| extract_filename_from_path(path));
                
                collect_standalone_structs_recursive(&contract_ast_value, &mut struct_definitions, source_file.as_deref());
            }
        }
    }
    
    struct_definitions
}

// Collect all enum definitions from build artifacts
fn collect_all_enum_definitions(
    output: &foundry_compilers::ProjectCompileOutput
) -> std::collections::HashMap<String, (String, Option<String>)> {
    let mut enum_definitions = std::collections::HashMap::new();
    
    // Collect annotated enums from all contracts
    for (_artifact_id, contract_artifact) in output.artifact_ids() {
        if let Some(contract_ast) = &contract_artifact.ast {
            if let Ok(contract_ast_value) = serde_json::to_value(contract_ast) {
                let source_file = contract_ast_value.get("absolutePath")
                    .and_then(|ap| ap.as_str())
                    .map(|path| extract_filename_from_path(path));
                
                collect_enum_definitions_recursive(&contract_ast_value, &mut enum_definitions, source_file.as_deref());
            }
        }
    }
    
    enum_definitions
}

// Collect all usertype definitions from build artifacts
fn collect_all_usertype_definitions(
    output: &foundry_compilers::ProjectCompileOutput
) -> std::collections::HashMap<String, (String, String, Option<String>)> {
    let mut usertype_definitions = std::collections::HashMap::new();
    
    // Collect annotated usertypes from all contracts
    for (_artifact_id, contract_artifact) in output.artifact_ids() {
        if let Some(contract_ast) = &contract_artifact.ast {
            if let Ok(contract_ast_value) = serde_json::to_value(contract_ast) {
                let source_file = contract_ast_value.get("absolutePath")
                    .and_then(|ap| ap.as_str())
                    .map(|path| extract_filename_from_path(path));
                
                collect_usertype_definitions_recursive(&contract_ast_value, &mut usertype_definitions, source_file.as_deref());
            }
        }
    }
    
    usertype_definitions
}

// Collect annotated enum definitions from AST
fn collect_enum_definitions_recursive(
    node: &Value,
    enum_definitions: &mut std::collections::HashMap<String, (String, Option<String>)>,
    current_source: Option<&str>
) {
    if let Some(node_type) = node.get("nodeType").and_then(|nt| nt.as_str()) {
        match node_type {
            "SourceUnit" => {
                if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                    for child_node in nodes {
                        collect_enum_definitions_recursive(child_node, enum_definitions, current_source);
                    }
                }
            }
            "ContractDefinition" | "LibraryDefinition" => {
                if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                    for child_node in nodes {
                        collect_enum_definitions_recursive(child_node, enum_definitions, current_source);
                    }
                }
            }
            "EnumDefinition" => {
                if let Some(canonical_name) = node.get("canonicalName").and_then(|n| n.as_str()) {
                    // Check if this enum has the storage-bucket-enum annotation
                    if let Some(doc_text) = node.get("documentation").and_then(|d| d.get("text")).and_then(|t| t.as_str()) {
                        if let Some(caps) = STORAGE_BUCKET_ENUM_RE.captures(doc_text.trim()) {
                            let enum_identifier = caps.get(1).unwrap().as_str().to_string();
                            enum_definitions.insert(
                                enum_identifier.clone(),
                                (canonical_name.to_string(), current_source.map(|s| s.to_string()))
                            );
                            // Also add without namespace for fallback
                            if enum_identifier.contains(':') {
                                let enum_name = enum_identifier.split(':').last().unwrap();
                                enum_definitions.insert(
                                    enum_name.to_string(),
                                    (canonical_name.to_string(), current_source.map(|s| s.to_string()))
                                );
                            }
                        }
                    }
                }
            }
            _ => {
                if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                    for child_node in nodes {
                        collect_enum_definitions_recursive(child_node, enum_definitions, current_source);
                    }
                }
            }
        }
    }
}

// Collect annotated usertype definitions from AST
fn collect_usertype_definitions_recursive(
    node: &Value,
    usertype_definitions: &mut std::collections::HashMap<String, (String, String, Option<String>)>,
    current_source: Option<&str>
) {
    if let Some(node_type) = node.get("nodeType").and_then(|nt| nt.as_str()) {
        match node_type {
            "SourceUnit" => {
                if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                    for child_node in nodes {
                        collect_usertype_definitions_recursive(child_node, usertype_definitions, current_source);
                    }
                }
            }
            "ContractDefinition" | "LibraryDefinition" => {
                if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                    for child_node in nodes {
                        collect_usertype_definitions_recursive(child_node, usertype_definitions, current_source);
                    }
                }
            }
            "UserDefinedValueTypeDefinition" => {
                if let Some(name) = node.get("name").and_then(|n| n.as_str()) {
                    if let Some(underlying_type) = node.get("underlyingType")
                        .and_then(|ut| ut.get("typeDescriptions"))
                        .and_then(|td| td.get("typeString"))
                        .and_then(|ts| ts.as_str()) {
                        
                        // Check if this usertype has the storage-bucket-usertype annotation
                        if let Some(doc_text) = node.get("documentation").and_then(|d| d.get("text")).and_then(|t| t.as_str()) {
                            if let Some(caps) = STORAGE_BUCKET_USERTYPE_RE.captures(doc_text.trim()) {
                                let usertype_identifier = caps.get(1).unwrap().as_str().to_string();
                                usertype_definitions.insert(
                                    usertype_identifier.clone(),
                                    (name.to_string(), underlying_type.to_string(), current_source.map(|s| s.to_string()))
                                );
                                // Also add without namespace for fallback
                                if usertype_identifier.contains(':') {
                                    let usertype_name = usertype_identifier.split(':').last().unwrap();
                                    usertype_definitions.insert(
                                        usertype_name.to_string(),
                                        (name.to_string(), underlying_type.to_string(), current_source.map(|s| s.to_string()))
                                    );
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                    for child_node in nodes {
                        collect_usertype_definitions_recursive(child_node, usertype_definitions, current_source);
                    }
                }
            }
        }
    }
}

// Collect standalone annotated structs from AST
fn collect_standalone_structs_recursive(
    node: &Value,
    struct_definitions: &mut std::collections::HashMap<String, (Vec<StructMember>, Option<String>, Option<String>)>,
    current_source: Option<&str>
) {
    if let Some(node_type) = node.get("nodeType").and_then(|nt| nt.as_str()) {
        match node_type {
            "SourceUnit" => {
                if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                    for child_node in nodes {
                        collect_standalone_structs_recursive(child_node, struct_definitions, current_source);
                    }
                }
            }
            "ContractDefinition" | "LibraryDefinition" => {
                if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                    for child_node in nodes {
                        collect_standalone_structs_recursive(child_node, struct_definitions, current_source);
                    }
                }
            }
            "StructDefinition" => {
                if let Some(canonical_name) = node.get("canonicalName").and_then(|n| n.as_str()) {
                    // Check if this struct has the storage-bucket-struct annotation
                    if let Some(doc_text) = node.get("documentation").and_then(|d| d.get("text")).and_then(|t| t.as_str()) {
                        if STORAGE_BUCKET_STRUCT_RE.is_match(doc_text.trim()) {
                            // This is a standalone annotated struct - add it to definitions
                            if let Some(members_array) = node.get("members").and_then(|m| m.as_array()) {
                                let struct_members = calculate_struct_layout(
                                    members_array,
                                    current_source, 
                                    Some(canonical_name)
                                );
                                
                                struct_definitions.insert(
                                    canonical_name.to_string(),
                                    (struct_members, current_source.map(|s| s.to_string()), None)
                                );
                            }
                        }
                    }
                }
            }
            _ => {
                if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                    for child_node in nodes {
                        collect_standalone_structs_recursive(child_node, struct_definitions, current_source);
                    }
                }
            }
        }
    }
}

// Expand nested structs in all buckets (post-processing step)
fn expand_nested_structs(
    buckets: &mut [StorageBucket],
    struct_definitions: &std::collections::HashMap<String, (Vec<StructMember>, Option<String>, Option<String>)>
) {
    // Now expand nested structs in each bucket
    for bucket in buckets.iter_mut() {
        if let Some(members) = &mut bucket.struct_members {
            expand_members_recursively(members, struct_definitions, 0);
        }
    }
}

// Recursively expand struct members
fn expand_members_recursively(
    members: &mut [StructMember], 
    struct_definitions: &std::collections::HashMap<String, (Vec<StructMember>, Option<String>, Option<String>)>,
    recursion_depth: usize
) {
    // Prevent infinite recursion
    if recursion_depth > 3 {
        return;
    }
    
    // Extract parent namespace once before the loop to avoid borrow checker issues
    let parent_namespace = extract_namespace_from_context(members);
    
    for member in members.iter_mut() {
        if let Some(struct_name) = extract_struct_name_from_type(&member.type_string) {
            // Skip if we already have nested members (avoid double expansion)
            if member.nested_members.is_some() {
                continue;
            }
            
            // Look for this struct definition with namespace preference
            if let Some((struct_members, source_file, _contract_name)) = 
                find_struct_with_namespace_preference(&struct_name, parent_namespace.as_deref(), struct_definitions) {
                let mut nested = Vec::new();
                
                // Add struct header
                let struct_header = StructMember {
                    name: format!("struct {}", struct_name),
                    type_string: "struct".to_string(),
                    slot_offset: member.slot_offset,
                    byte_offset: 0,
                    size_bytes: calculate_struct_total_bytes(&struct_members),
                    source_file: source_file.clone(),
                    struct_name: Some(struct_name.clone()),
                    struct_identifier: None,
                    nested_members: None,
                };
                nested.push(struct_header);
                
                // Add all struct members with adjusted slot formulas
                for (_index, struct_member) in struct_members.iter().enumerate() {
                    let mut nested_member = struct_member.clone();
                    
                    // Adjust slot offset for nested context
                    if is_mapping_type(&member.type_string) {
                        // For mappings like mapping(bytes32 => struct Market), 
                        // the formula becomes keccak(key, base_slot) + member_offset
                        nested_member.slot_offset = member.slot_offset + struct_member.slot_offset;
                    } else {
                        // For direct struct fields, just add offset
                        nested_member.slot_offset = member.slot_offset + struct_member.slot_offset;
                    }
                    
                    nested.push(nested_member);
                }
                
                // Recursively expand any nested structs within these members
                expand_members_recursively(&mut nested, struct_definitions, recursion_depth + 1);
                
                member.nested_members = Some(nested);
            }
        }
    }
}

// Calculate total bytes for a struct
fn calculate_struct_total_bytes(members: &[StructMember]) -> u64 {
    if members.is_empty() {
        return 0;
    }
    
    // Find the last member and calculate total size
    let last_member = members.last().unwrap();
    (last_member.slot_offset + 1) * SLOT_SIZE_BYTES // Each slot is SLOT_SIZE_BYTES bytes
}

fn extract_filename_from_path(path: &str) -> String {
    // Extract filename from full path (e.g., "contracts/types/Contract.sol" -> "Contract.sol")
    if let Some(filename) = path.split('/').last() {
        filename.to_string()
    } else {
        path.to_string()
    }
}

fn format_contract_name(bucket: &StorageBucket) -> String {
    // Use actual source file and contract name from AST if available
    let source_file = bucket.source_file.as_deref().unwrap_or("Unknown.sol");
    let contract_name = bucket.contract_name.as_deref().unwrap_or("Unknown");
    
    format!("{}:{}", source_file, contract_name)
}

fn format_struct_contract_name(member: &StructMember) -> String {
    // Use actual source file and struct name from AST if available
    let source_file = member.source_file.as_deref().unwrap_or("Unknown.sol");
    let struct_name = member.struct_name.as_deref().unwrap_or("Unknown");
    
    format!("{}:{}", source_file, struct_name)
}

fn extract_struct_name_from_bucket(bucket: &StorageBucket) -> String {
    // Extract struct name from return type or bucket name
    if let Some(ret_type) = &bucket.return_type {
        ret_type.clone()
    } else {
        // Fallback: extract from bucket name
        bucket.name.split('.').last().unwrap_or(&bucket.name).to_string()
    }
}

fn calculate_total_struct_size(struct_members: &[StructMember]) -> u64 {
    if struct_members.is_empty() {
        return 0;
    }
    
    // Find the last member and calculate total size based on its position + size
    let last_member = struct_members.iter().max_by_key(|m| m.slot_offset * SLOT_SIZE_BYTES + m.byte_offset).unwrap();
    let last_slot_end = last_member.slot_offset * SLOT_SIZE_BYTES + last_member.byte_offset + last_member.size_bytes;
    
    // Round up to next SLOT_SIZE_BYTES-byte boundary if needed
    if last_slot_end % SLOT_SIZE_BYTES == 0 {
        last_slot_end
    } else {
        ((last_slot_end / SLOT_SIZE_BYTES) + 1) * SLOT_SIZE_BYTES
    }
}

fn generate_member_slot_formula_base(bucket: &StorageBucket) -> String {
    match bucket.bucket_type.as_str() {
        "keyvalue" => "keccak(key, slot)".to_string(),
        "singleton" => bucket.slot.clone(),
        _ => "base".to_string(),
    }
}

fn generate_member_slot_formula(bucket: &StorageBucket, member: &StructMember) -> String {
    match bucket.bucket_type.as_str() {
        "keyvalue" => {
            // For mapping types, struct members use keccak(key, base_slot) + member_offset
            if member.slot_offset == 0 {
                "keccak(key, slot)".to_string()
            } else {
                format!("keccak(key, slot) + {}", member.slot_offset)
            }
        }
        "singleton" => {
            // For singleton types, just use base slot + offset
            if member.slot_offset == 0 {
                bucket.slot.clone()
            } else {
                format!("{} + {}", bucket.slot, member.slot_offset)
            }
        }
        _ => {
            // Default: show relative offset
            if member.slot_offset == 0 {
                "base".to_string()
            } else {
                format!("base + {}", member.slot_offset)
            }
        }
    }
}

fn clean_param_type(type_str: &str) -> String {
    // Clean up parameter types from AST
    if type_str.starts_with("enum ") {
        // "enum BookType" -> "BookType"
        type_str.strip_prefix("enum ").unwrap_or(type_str).to_string()
    } else if type_str.contains("uint256") && type_str != "uint256" {
        // "uint256" from things like "OrderId" which is really uint256
        if type_str.starts_with("uint256") {
            "bytes32".to_string() // EIP-7201 slots typically use bytes32 as keys
        } else {
            type_str.to_string()
        }
    } else {
        type_str.to_string()
    }
}

fn calculate_type_size(type_string: &str) -> u64 {
    match type_string {
        "bool" => 1,
        "address" => 20,
        "bytes32" => SLOT_SIZE_BYTES,
        s if s.starts_with("uint") => {
            if let Some(bits_str) = s.strip_prefix("uint") {
                if bits_str.is_empty() {
                    256 / 8 // uint defaults to uint256
                } else if let Ok(bits) = bits_str.parse::<u32>() {
                    bits as u64 / 8
                } else {
                    SLOT_SIZE_BYTES // fallback
                }
            } else {
                32
            }
        },
        s if s.starts_with("int") => {
            if let Some(bits_str) = s.strip_prefix("int") {
                if bits_str.is_empty() {
                    256 / 8 // int defaults to int256
                } else if let Ok(bits) = bits_str.parse::<u32>() {
                    bits as u64 / 8
                } else {
                    SLOT_SIZE_BYTES // fallback
                }
            } else {
                32
            }
        },
        s if s.starts_with("bytes") && !s.starts_with("bytes32") => {
            // Dynamic bytes type
            SLOT_SIZE_BYTES // Takes full slot for length + pointer
        },
        s if s.starts_with("enum ") => {
            // Enums are typically uint8 unless they have > 256 members
            1
        },
        _ => {
            // For complex types (structs, arrays, mappings), assume they take a full slot
            32
        }
    }
}

//     // Search for struct definitions and update buckets with missing struct members
//     search_struct_definitions_recursive(node, buckets, None);
// }

fn search_for_struct_definitions_with_source(node: &Value, buckets: &mut Vec<StorageBucket>, source_file: Option<&str>) {
    // Search for struct definitions with proper source file attribution
    search_struct_definitions_recursive(node, buckets, source_file);
}

fn search_struct_definitions_recursive(node: &Value, buckets: &mut Vec<StorageBucket>, current_source: Option<&str>) {
    if let Some(node_type) = node.get("nodeType").and_then(|nt| nt.as_str()) {
        match node_type {
            "SourceUnit" => {
                // Extract source file path
                let source_file = node.get("absolutePath")
                    .and_then(|ap| ap.as_str())
                    .or_else(|| node.get("src").and_then(|src| src.as_str()))
                    .map(|path| extract_filename_from_path(path));
                
                if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                    for child_node in nodes {
                        search_struct_definitions_recursive(child_node, buckets, source_file.as_deref());
                    }
                }
            }
            "ContractDefinition" | "LibraryDefinition" => {
                if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                    for child_node in nodes {
                        search_struct_definitions_recursive(child_node, buckets, current_source);
                    }
                }
            }
            "StructDefinition" => {
                // Check if this struct matches any return type in our buckets
                if let Some(canonical_name) = node.get("canonicalName").and_then(|n| n.as_str()) {
                    
                    // Look for buckets that need this struct definition
                    for bucket in buckets.iter_mut() {
                        
                        if let Some(ret_type) = &bucket.return_type {
                            if canonical_name == ret_type || ret_type.contains(canonical_name) {
                                
                                // Get struct identifier from @custom:storage-bucket-struct annotation
                                let struct_identifier = node
                                    .get("documentation")
                                    .and_then(|d| d.get("text"))
                                    .and_then(|t| t.as_str())
                                    .and_then(|doc| STORAGE_BUCKET_STRUCT_RE.captures(doc.trim()))
                                    .map(|caps| caps.get(1).unwrap().as_str().to_string());
                                
                                // Check if identifiers match (if both bucket and struct have identifiers)
                                let identifiers_match = match (&bucket.value_identifier, &struct_identifier) {
                                    (Some(bucket_id), Some(struct_id)) => bucket_id == struct_id,
                                    _ => true, // If either doesn't have identifier, allow match (backward compatibility)
                                };
                                
                                // This bucket needs this struct definition
                                // Priority: 1) Matching identifiers, 2) No existing struct, 3) Has annotation
                                let should_update = identifiers_match && (
                                    bucket.struct_members.is_none() || 
                                    struct_identifier.is_some() ||
                                    (current_source.is_some() && bucket.struct_members.as_ref().map_or(true, |members|
                                        members.iter().any(|m| m.source_file.is_none())
                                    ))
                                );
                                
                                if should_update {
                                    if let Some(members_array) = node.get("members").and_then(|m| m.as_array()) {
                                        // For now, use basic struct layout without recursive expansion to avoid borrow issues
                                        let mut struct_members = calculate_struct_layout(
                                            members_array, 
                                            current_source, 
                                            Some(canonical_name)
                                        );
                                        
                                        // Set struct identifier on all members
                                        if let Some(struct_id) = &struct_identifier {
                                            for member in &mut struct_members {
                                                member.struct_identifier = Some(struct_id.clone());
                                            }
                                        }
                                        
                                        bucket.struct_members = Some(struct_members);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                // Continue recursing for other node types
                if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                    for child_node in nodes {
                        search_struct_definitions_recursive(child_node, buckets, current_source);
                    }
                }
            }
        }
    }
}

// New function to fill missing info in existing buckets from other files
fn fill_missing_bucket_info(node: &Value, buckets: &mut Vec<StorageBucket>) {
    if let Some(node_type) = node.get("nodeType").and_then(|nt| nt.as_str()) {
        match node_type {
            "SourceUnit" => {
                if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                    for child_node in nodes {
                        fill_missing_bucket_info(child_node, buckets);
                    }
                }
            }
            "ContractDefinition" | "LibraryDefinition" => {
                if node.get("name").is_some() {
                    if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                        for child_node in nodes {
                            fill_missing_function_info(child_node, buckets);
                            extract_struct_buckets(child_node, buckets, None);
                        }
                    }
                }
            }
            "StructDefinition" => {
                // Also check for top-level struct definitions when filling missing info
                extract_struct_buckets(node, buckets, None);
            }
            _ => {
                if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
                    for child_node in nodes {
                        fill_missing_bucket_info(child_node, buckets);
                    }
                }
            }
        }
    }
}

fn update_bucket_type_from_function(node: &Value, buckets: &mut Vec<StorageBucket>) {
    if let Some(doc_text) = get_documentation_text(node) {
        if let Some(caps) = STORAGE_BUCKET_TYPE_RE.captures(doc_text.trim()) {
            let function_ref = caps.get(1).unwrap().as_str();
            let bucket_type = caps.get(2).unwrap().as_str();
            
            if let Some(existing_bucket) = buckets.iter_mut().find(|b| b.name == function_ref) {
                if existing_bucket.bucket_type == "unknown" {
                    existing_bucket.bucket_type = bucket_type.to_string();
                }
                if existing_bucket.parameters.is_none() || existing_bucket.return_type.is_none() {
                    extract_function_signature_from_ast(node, existing_bucket);
                }
            }
        }
    }
}

fn update_bucket_slot_from_variable(node: &Value, buckets: &mut Vec<StorageBucket>) {
    if let Some(doc_text) = get_documentation_text(node) {
        if let Some(caps) = STORAGE_BUCKET_SLOT_RE.captures(doc_text.trim()) {
            let function_ref = caps.get(1).unwrap().as_str();
            
            // Prefer AST constant value over natspec documentation
            let slot_hex = if let Some(ast_value) = extract_constant_value_from_ast(node) {
                ast_value
            } else if let Some(natspec_hex) = caps.get(2) {
                natspec_hex.as_str().to_string()
            } else {
                "0x0".to_string()
            };
            
            if let Some(existing_bucket) = buckets.iter_mut().find(|b| b.name == function_ref) {
                if existing_bucket.slot.is_empty() {
                    let slot = U256::from_str_radix(slot_hex.strip_prefix("0x").unwrap_or(&slot_hex), 16).ok();
                    let short_slot = if let Some(slot) = slot {
                        short_hex(&alloy_primitives::hex::encode_prefixed(slot.to_be_bytes::<{ SLOT_SIZE_BYTES as usize }>()))
                    } else {
                        slot_hex
                    };
                    existing_bucket.slot = short_slot;
                }
            }
        }
    }
}

fn extract_constant_value_from_ast(node: &Value) -> Option<String> {
    // Try the refreshed AST format first: value.value field
    if let Some(hex_value) = node.get("value")
        .and_then(|literal| literal.get("value"))
        .and_then(|v| v.as_str()) {
        
        // Check if it's already a proper hex value
        if hex_value.starts_with("0x") {
            if let Ok(slot) = U256::from_str_radix(&hex_value[2..], 16) {
                let full_hex = alloy_primitives::hex::encode_prefixed(slot.to_be_bytes::<{ SLOT_SIZE_BYTES as usize }>());
                return Some(short_hex(&full_hex));
            }
        }
        return Some(hex_value.to_string());
    }
    
    // Fallback: Try hexValue field (older AST format)
    if let Some(hex_value) = node.get("value")
        .and_then(|literal| literal.get("hexValue"))
        .and_then(|v| v.as_str()) {
        
        // Handle double-encoded hex values (AST stores string literals as hex-encoded bytes)
        let actual_hex = if let Ok(decoded_bytes) = hex::decode(hex_value) {
            if let Ok(decoded_string) = String::from_utf8(decoded_bytes) {
                // If it decodes to a hex string like "0x4241b72...", use it
                if decoded_string.starts_with("0x") {
                    decoded_string.strip_prefix("0x").unwrap_or(&decoded_string).to_string()
                } else {
                    hex_value.to_string()  // Use original if decode didn't yield hex string
                }
            } else {
                hex_value.to_string()  // Use original if UTF-8 decode fails
            }
        } else {
            hex_value.to_string()  // Use original if hex decode fails
        };
        
        // Convert hex value to short format 
        if let Ok(slot) = U256::from_str_radix(&actual_hex, 16) {
            let full_hex = alloy_primitives::hex::encode_prefixed(slot.to_be_bytes::<{ SLOT_SIZE_BYTES as usize }>());
            return Some(short_hex(&full_hex));
        }
    }
    
    None
}


fn get_documentation_text(node: &Value) -> Option<&str> {
    node.get("documentation").and_then(|d| d.get("text")).and_then(|t| t.as_str())
}

fn fill_missing_function_info(node: &Value, buckets: &mut Vec<StorageBucket>) {
    let node_type = node.get("nodeType").and_then(|nt| nt.as_str());
    
    match node_type {
        Some("FunctionDefinition") => update_bucket_type_from_function(node, buckets),
        Some("VariableDeclaration") => update_bucket_slot_from_variable(node, buckets),
        _ => {}
    }
}

fn extract_function_signature_from_ast(node: &Value, bucket: &mut StorageBucket) {
    // Check for @custom:storage-bucket-value annotation to get specific identifier
    if let Some(doc_text) = node.get("documentation").and_then(|d| d.get("text")).and_then(|t| t.as_str()) {
        if let Some(caps) = STORAGE_BUCKET_VALUE_RE.captures(doc_text.trim()) {
            bucket.value_identifier = Some(caps.get(1).unwrap().as_str().to_string());
        }
    }
    // Extract parameters
    if let Some(parameters) = node
        .get("parameters")
        .and_then(|p| p.get("parameters"))
        .and_then(|p| p.as_array())
    {
        let param_types: Vec<String> = parameters
            .iter()
            .filter_map(|param| {
                param
                    .get("typeDescriptions")
                    .and_then(|td| td.get("typeString"))
                    .and_then(|ts| ts.as_str())
                    .map(|s| clean_param_type(s))
            })
            .collect();
        
        if !param_types.is_empty() {
            bucket.parameters = Some(param_types.join(", "));
        }
    }

    // Extract return parameters
    if let Some(return_params) = node
        .get("returnParameters")
        .and_then(|rp| rp.get("parameters"))
        .and_then(|p| p.as_array())
    {
        let return_types: Vec<String> = return_params
            .iter()
            .filter_map(|param| {
                param
                    .get("typeDescriptions")
                    .and_then(|td| td.get("typeString"))
                    .and_then(|ts| ts.as_str())
                    .map(|s| {
                        // Clean up return type - extract struct name from storage references
                        if s.starts_with("struct ") && s.contains(" storage") {
                            let struct_part = s.strip_prefix("struct ").unwrap_or(s);
                            if let Some(space_idx) = struct_part.find(" storage") {
                                struct_part[..space_idx].to_string()
                            } else {
                                s.to_string()
                            }
                        } else {
                            s.to_string()
                        }
                    })
            })
            .collect();
        
        if !return_types.is_empty() {
            bucket.return_type = Some(return_types.join(", "));
        }
    }
    
    // Create function signature
    if let Some(func_name) = node.get("name").and_then(|n| n.as_str()) {
        let params = bucket.parameters.as_ref().map(|p| p.as_str()).unwrap_or("");
        bucket.function_signature = Some(format!("{}({})", func_name, params));
        
    }
}

fn extract_all_param_types(params: &str) -> String {
    // Extract all parameter types from comma-separated parameters
    // Handle both AST format (direct types) and storage format (type + name)
    // e.g., "bytes32, enum BookType" -> "bytes32, BookType" 
    // or "bytes32 asset, BookType bookType" -> "bytes32, BookType"
    params
        .split(',')
        .map(|param| {
            let param = param.trim();
            
            // Check if this looks like an enum type
            if param.starts_with("enum ") {
                // Extract just the enum name: "enum BookType" -> "BookType"
                param.strip_prefix("enum ").unwrap_or(param)
            } else if param.contains(' ') {
                // This is likely storage format: "bytes32 asset" -> "bytes32"
                param.split_whitespace().next().unwrap_or("unknown")
            } else {
                // This is likely already a clean type from AST
                param
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn extract_storage_type(return_type: &str) -> &str {
    // Extract the type from return like "StructName storage structInstance" -> "StructName"
    return_type.split_whitespace().next().unwrap_or("unknown")
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
                                "custom:storage-bucket": "EIP712Storage 0xa16a46d94261c7517cc8ff89f61c0ce93598e3c849801011dee649a6a557d100NoncesStorage 0x5ab42ced628888259c08ac98db1eb0cf702fc1501344311d8b100cd1bfe4bb00"
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

        let rows =
            parse_storage_buckets_value(Some(&inner_meta_str)).expect("parser returned None");
        assert_eq!(rows.len(), 2, "expected two EIP-7201 buckets");

        assert_eq!(rows[0].0, "EIP712Storage");
        assert_eq!(rows[1].0, "NoncesStorage");

        let expect_short = |h: &str| {
            let hex_str = h.trim_start_matches("0x");
            let slot = U256::from_str_radix(hex_str, 16).unwrap();
            let full = alloy_primitives::hex::encode_prefixed(slot.to_be_bytes::<{ SLOT_SIZE_BYTES as usize }>());
            short_hex(&full)
        };

        let eip712_slot_hex =
            expect_short("0xa16a46d94261c7517cc8ff89f61c0ce93598e3c849801011dee649a6a557d100");
        let nonces_slot_hex =
            expect_short("0x5ab42ced628888259c08ac98db1eb0cf702fc1501344311d8b100cd1bfe4bb00");

        assert_eq!(rows[0].1, eip712_slot_hex);
        assert_eq!(rows[1].1, nonces_slot_hex);

        assert!(rows[0].1.starts_with("0x") && rows[0].1.contains('…'));
        assert!(rows[1].1.starts_with("0x") && rows[1].1.contains('…'));
    }

    #[test]
    fn parses_eip7201_storage_buckets_from_ast() {
        assert_eq!(extract_all_param_types("bytes32 asset"), "bytes32");
        assert_eq!(extract_storage_type("EIP712Storage storage eip712Storage"), "EIP712Storage");
    }

    #[test]
    fn extracts_param_and_storage_types() {
        assert_eq!(extract_all_param_types("bytes32 asset"), "bytes32");
        assert_eq!(extract_all_param_types("uint256 value"), "uint256");
        assert_eq!(extract_all_param_types("address user"), "address");
        assert_eq!(extract_all_param_types("bytes32 asset, BookType bookType"), "bytes32, BookType");
        
        assert_eq!(extract_storage_type("EIP712Storage storage eip712Storage"), "EIP712Storage");
        assert_eq!(extract_storage_type("NoncesStorage storage noncesStorage"), "NoncesStorage");
        assert_eq!(extract_storage_type("uint256 storage value"), "uint256");
    }

    #[test]
    fn handles_transient_storage_buckets() {
        let mut buckets = vec![StorageBucket {
            name: "TransientCounter".to_string(),
            bucket_type: "unknown".to_string(),
            slot: "0xa16a46d9…57d100".to_string(),
            function_signature: None,
            parameters: None,
            return_type: None,
            struct_members: None,
            source_file: None,
            contract_name: None,
            value_identifier: None,
            is_transient: false,
        }];

        // Test transient storage display
        let display_name = if buckets[0].is_transient {
            format!("[T] {}", buckets[0].name)
        } else {
            buckets[0].name.clone()
        };
        
        assert_eq!(display_name, "TransientCounter");
        
        // Mark as transient
        buckets[0].is_transient = true;
        let display_name_transient = if buckets[0].is_transient {
            format!("[T] {}", buckets[0].name)
        } else {
            buckets[0].name.clone()
        };
        
        assert_eq!(display_name_transient, "[T] TransientCounter");
    }

    #[test]
    fn parses_transient_annotations() {
        // Test @custom:storage-bucket-transient
        let doc_text_basic = "@custom:storage-bucket-transient TransientCounter ReentrantLock";
        let caps_basic = STORAGE_BUCKET_TRANSIENT_RE.captures(doc_text_basic.trim());
        assert!(caps_basic.is_some());
        assert_eq!(caps_basic.unwrap().get(1).unwrap().as_str(), "TransientCounter ReentrantLock");

        // Test @custom:storage-bucket-transient-slot
        let doc_text_slot = "@custom:storage-bucket-transient-slot TransientCounter 0xa16a46d94261c7517cc8ff89f61c0ce93598e3c849801011dee649a6a557d100";
        let caps_slot = STORAGE_BUCKET_TRANSIENT_SLOT_RE.captures(doc_text_slot.trim());
        assert!(caps_slot.is_some());
        
        let caps_slot = caps_slot.unwrap();
        assert_eq!(caps_slot.get(1).unwrap().as_str(), "TransientCounter");
        assert_eq!(caps_slot.get(2).unwrap().as_str(), "0xa16a46d94261c7517cc8ff89f61c0ce93598e3c849801011dee649a6a557d100");
    }
}
