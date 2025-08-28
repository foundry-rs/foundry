//! Backtrace extraction from call traces.

use super::{Backtrace, BacktraceFrame, BacktraceFrameKind, PcSourceMapper};
use alloy_primitives::{Address, Bytes};
use foundry_common::contracts::ContractsByAddress;
use foundry_compilers::artifacts::sourcemap::SourceMap;
use foundry_evm::traces::SparsedTraceArena;
use revm::DatabaseRef;
use std::collections::HashMap;

/// Extracts a backtrace from a call trace arena.
///
/// This function walks the call trace to find the revert point and builds
/// a stack trace from the call hierarchy.
pub fn extract_backtrace<DB: DatabaseRef>(
    arena: &SparsedTraceArena,
    contracts: &ContractsByAddress,
    source_maps: &HashMap<Address, (SourceMap, SourceMap)>, // (creation, runtime)
    sources: &HashMap<Address, Vec<(String, String)>>, // Source files per contract
    backend: &DB,
    deployed_bytecodes: &HashMap<Address, Bytes>, // Deployed bytecode for each contract
) -> Option<Backtrace> {
    // Get the actual call trace arena
    let resolved_arena = &arena.arena;
    
    if resolved_arena.nodes().is_empty() {
        return None;
    }
    
    // Build a map of addresses to contract names from decoded traces
    let mut address_to_name: HashMap<Address, String> = HashMap::new();
    for node in resolved_arena.nodes() {
        if let Some(decoded) = &node.trace.decoded {
            if let Some(ref label) = decoded.label {
                if let Some(contract_part) = label.split("::").next() {
                    address_to_name.insert(node.trace.address, contract_part.to_string());
                }
            }
        }
    }
    
    // Find the deepest failed node (where the actual revert happened)
    let mut deepest_failed_idx = None;
    let mut max_depth = 0;
    
    for (idx, node) in resolved_arena.nodes().iter().enumerate() {
        if !node.trace.success && node.trace.depth >= max_depth {
            max_depth = node.trace.depth;
            deepest_failed_idx = Some(idx);
        }
    }
    
    let deepest_idx = deepest_failed_idx?;
    
    // Build PC source mappers for each contract
    let mut pc_mappers: HashMap<Address, PcSourceMapper> = HashMap::new();
    
    tracing::info!(
        source_maps_count = source_maps.len(),
        deployed_bytecodes_count = deployed_bytecodes.len(),
        "Building PC mappers"
    );
    
    for (addr, (_creation_map, runtime_map)) in source_maps {
        if let Some(contract_sources) = sources.get(addr) {
            // First try to get the deployed bytecode from our mapping
            let bytecode = if let Some(deployed) = deployed_bytecodes.get(addr) {
                tracing::info!("Using deployed bytecode from mapping for address {}", addr);
                deployed.clone()
            } else {
                // Fallback to getting it from the backend
                if let Ok(Some(account)) = backend.basic_ref(*addr) {
                    if let Some(code) = &account.code {
                        if !code.is_empty() {
                            let bytecode = code.original_bytes();
                            tracing::info!(
                                "Using bytecode from backend for address {} (size: {} bytes)",
                                addr,
                                bytecode.len()
                            );
                            bytecode
                        } else {
                            tracing::info!("Empty bytecode for address {}", addr);
                            continue;
                        }
                    } else {
                        tracing::info!("No code for address {}", addr);
                        continue;
                    }
                } else {
                    tracing::info!("Failed to get account for address {}", addr);
                    continue;
                }
            };
            
            pc_mappers.insert(
                *addr,
                PcSourceMapper::new(&bytecode, runtime_map.clone(), contract_sources.clone())
            );
            tracing::info!("Created PC mapper for address {}", addr);
        } else {
            tracing::info!("No sources found for address {}", addr);
        }
    }
    
    tracing::info!("Total PC mappers created: {}", pc_mappers.len());
    
    // Build the call stack by walking from the deepest node back to root
    let mut frames = Vec::new();
    let mut current_idx = Some(deepest_idx);
    
    while let Some(idx) = current_idx {
        let node = &resolved_arena.nodes()[idx];
        let trace = &node.trace;
        
        // Skip successful calls unless they're in the failure path
        if trace.success && idx != 0 {
            current_idx = node.parent;
            continue;
        }
        
        let contract_address = trace.address;
        
        // Create the frame
        let mut frame = BacktraceFrame::new(contract_address);
        
        // Get contract info  
        if let Some((contract_name, _abi)) = contracts.get(&contract_address) {
            frame = frame.with_contract_name(contract_name.clone());
        } else if let Some(name) = address_to_name.get(&contract_address) {
            frame = frame.with_contract_name(name.clone());
        }
        
        // Get function name from decoded trace if available
        if let Some(decoded) = &trace.decoded {
            if let Some(ref label) = decoded.label {
                let parts: Vec<&str> = label.split("::").collect();
                if parts.len() > 1 {
                    let func_part = parts[1];
                    frame = frame.with_function_name(func_part.to_string());
                } else {
                    frame = frame.with_function_name(label.clone());
                }
            } else if let Some(ref call_data) = decoded.call_data {
                frame = frame.with_function_name(call_data.signature.clone());
            }
        }
        
        // Try to get actual source location from trace
        tracing::info!(
            "Trace has {} steps for address {}",
            trace.steps.len(),
            contract_address
        );
        
        if let Some(source_location) = get_source_location_from_trace(
            trace,
            contract_address,
            &pc_mappers,
            source_maps,
            sources,
        ) {
            frame = frame.with_source_location(
                source_location.file,
                source_location.line,
                source_location.column,
            );
        } else {
            // Fallback: try to infer something from contract name
            if let Some(sources_list) = sources.get(&contract_address) {
                if let Some((file_path, _)) = sources_list.first() {
                    frame = frame.with_source_location(
                        file_path.clone(),
                        0,  // Unknown line
                        0,  // Unknown column
                    );
                }
            }
        }
        
        // Determine frame kind
        let kind = determine_frame_kind(&trace, &frame.function_name);
        frame = frame.with_kind(kind);
        
        frames.push(frame);
        
        // Move to parent node
        current_idx = node.parent;
    }
    
    // Reverse frames to have innermost first
    frames.reverse();
    
    if !frames.is_empty() {
        Some(Backtrace::new(frames))
    } else {
        None
    }
}

/// Gets the source location from trace.
fn get_source_location_from_trace(
    trace: &foundry_evm::traces::CallTrace,
    contract_address: Address,
    pc_mappers: &HashMap<Address, PcSourceMapper>,
    source_maps: &HashMap<Address, (SourceMap, SourceMap)>,
    sources: &HashMap<Address, Vec<(String, String)>>,
) -> Option<super::source_map::SourceLocation> {
    // Find the last step (which should be the revert point)
    let last_step = trace.steps.last()?;
    
    // Get the program counter from the step
    let pc = last_step.pc;
    
    tracing::info!(
        pc = pc,
        address = %contract_address,
        mappers_count = pc_mappers.len(),
        steps_count = trace.steps.len(),
        "Looking for source location"
    );
    
    // Try to get source location from PC mapper
    if let Some(mapper) = pc_mappers.get(&contract_address) {
        return mapper.map_pc(pc);
    }
    
    // Fallback: try to decode directly if we have source maps
    if let Some((_, runtime_map)) = source_maps.get(&contract_address) {
        if let Some(sources_list) = sources.get(&contract_address) {
            // Try to find the source element for this PC
            // This is a simplified approach - in reality we'd need the bytecode
            // and proper IC to PC mapping
            
            // For now, estimate based on step index
            let estimated_ic = trace.steps.len().saturating_sub(1);
            if let Some(element) = runtime_map.get(estimated_ic) {
                if let Some(source_idx) = element.index() {
                    if let Some((file_path, content)) = sources_list.get(source_idx as usize) {
                        let offset = element.offset() as usize;
                        let (line, column) = offset_to_line_column(content, offset);
                        return Some(super::source_map::SourceLocation {
                            file: file_path.clone(),
                            line,
                            column,
                            length: element.length() as usize,
                        });
                    }
                }
            }
        }
    }
    
    None
}

/// Converts a byte offset to line and column numbers.
fn offset_to_line_column(content: &str, offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    
    for (idx, ch) in content.char_indices() {
        if idx >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    
    (line, column)
}

/// Determines the kind of frame based on the trace information.
fn determine_frame_kind(
    _trace: &foundry_evm::traces::CallTrace,
    function_name: &Option<String>,
) -> BacktraceFrameKind {
    if let Some(name) = function_name {
        if name.starts_with("test") {
            BacktraceFrameKind::TestFunction
        } else if name == "fallback" || name == "<fallback>" {
            BacktraceFrameKind::Fallback
        } else if name == "receive" || name == "<receive>" {
            BacktraceFrameKind::Receive
        } else if name == "constructor" || name == "<constructor>" {
            BacktraceFrameKind::Constructor
        } else {
            BacktraceFrameKind::UserFunction
        }
    } else {
        BacktraceFrameKind::UserFunction
    }
}