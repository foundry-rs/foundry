//! Solidity stack trace support for test failures.

use alloy_primitives::{Address, Bytes};
use foundry_common::contracts::ContractsByAddress;
use foundry_compilers::artifacts::sourcemap::SourceMap;
use foundry_evm::traces::SparsedTraceArena;
use revm::DatabaseRef;
use std::{collections::HashMap, fmt};
use yansi::Paint;

mod solidity;
mod source_map;

pub use solidity::{PcToSourceMapper, SourceLocation};
pub use source_map::PcSourceMapper;

/// A Solidity stack trace for a test failure.
#[derive(Debug, Clone, Default)]
pub struct Backtrace {
    /// The frames of the backtrace, from innermost (where the revert happened) to outermost.
    pub frames: Vec<BacktraceFrame>,
}

impl Backtrace {
    /// Creates a new backtrace with the given frames.
    pub fn new(frames: Vec<BacktraceFrame>) -> Self {
        Self { frames }
    }

    /// Returns true if the backtrace is empty.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

impl fmt::Display for Backtrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.frames.is_empty() {
            return Ok(());
        }

        writeln!(f, "{}", Paint::yellow("Stack trace:"))?;

        for frame in self.frames.iter() {
            write!(f, "  ")?;
            write!(f, "at ")?;
            writeln!(f, "{}", frame.format())?;
        }

        Ok(())
    }
}

/// A single frame in a backtrace.
#[derive(Debug, Clone)]
pub struct BacktraceFrame {
    /// The contract address where this frame is executing.
    pub contract_address: Address,
    /// The contract name, if known.
    pub contract_name: Option<String>,
    /// The function name, if known.
    pub function_name: Option<String>,
    /// The source file path.
    pub file: Option<String>,
    /// The line number in the source file.
    pub line: Option<usize>,
    /// The column number in the source file.
    pub column: Option<usize>,
}

impl BacktraceFrame {
    /// Creates a new backtrace frame.
    pub fn new(contract_address: Address) -> Self {
        Self {
            contract_address,
            contract_name: None,
            function_name: None,
            file: None,
            line: None,
            column: None,
        }
    }

    /// Sets the contract name.
    pub fn with_contract_name(mut self, name: String) -> Self {
        self.contract_name = Some(name);
        self
    }

    /// Sets the function name.
    pub fn with_function_name(mut self, name: String) -> Self {
        self.function_name = Some(name);
        self
    }

    /// Sets the source location.
    pub fn with_source_location(mut self, file: String, line: usize, column: usize) -> Self {
        self.file = Some(file);
        self.line = Some(line);
        self.column = Some(column);
        self
    }

    /// Returns a formatted string for this frame.
    pub fn format(&self) -> String {
        let mut result = String::new();

        // Format: file:line:column or just ContractName if no file info
        if let Some(ref file) = self.file {
            // Start with file path
            result.push_str(file);

            // Add line and column directly after file path
            if let Some(line) = self.line {
                result.push(':');
                result.push_str(&line.to_string());
                if let Some(column) = self.column {
                    result.push(':');
                    result.push_str(&column.to_string());
                }
            }
        } else {
            // No file info - try to show at least something useful
            // Format: ContractName or address if no name available
            if let Some(ref contract) = self.contract_name {
                // Try to infer file path from contract name
                if contract.contains(':') {
                    // Already has file path like "src/SomeFile.sol:ContractName"
                    result.push_str(contract);
                } else {
                    // Just contract name - we don't know the file path
                    // Show as <ContractName> to indicate missing file info
                    result.push('<');
                    result.push_str(contract);
                    result.push('>');
                }
            } else {
                // No contract name, show address
                result.push_str(&format!("<Contract {}>", self.contract_address));
            }

            // Add function if available
            if let Some(ref func) = self.function_name {
                result.push('.');
                result.push_str(func);
                result.push_str("()");
            }

            // Only add line:column if we have at least line info
            if self.line.is_some()
                && let Some(line) = self.line
            {
                result.push(':');
                result.push_str(&line.to_string());
                if let Some(column) = self.column {
                    result.push(':');
                    result.push_str(&column.to_string());
                } else {
                    result.push_str(":0");
                }
            }
        }

        result
    }
}

impl fmt::Display for BacktraceFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format())
    }
}

/// Extracts a backtrace from a call trace arena.
///
/// This function walks the call trace to find the revert point and builds
/// a stack trace from the call hierarchy.
pub fn extract_backtrace<DB: DatabaseRef>(
    arena: &SparsedTraceArena,
    contracts: &ContractsByAddress,
    source_maps: &HashMap<Address, (SourceMap, SourceMap)>, // (creation, runtime)
    sources: &HashMap<Address, Vec<(String, String)>>,      // Source files per contract
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
        if let Some(decoded) = &node.trace.decoded
            && let Some(ref label) = decoded.label
            && let Some(contract_part) = label.split("::").next()
        {
            address_to_name.insert(node.trace.address, contract_part.to_string());
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
                PcSourceMapper::new(&bytecode, runtime_map.clone(), contract_sources.clone()),
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
        tracing::info!("Trace has {} steps for address {}", trace.steps.len(), contract_address);

        if let Some(source_location) = get_source_location_from_trace(
            trace,
            contract_address,
            &pc_mappers,
        ) {
            frame = frame.with_source_location(
                source_location.file,
                source_location.line,
                source_location.column,
            );
        } else {
            // Fallback: try to infer something from contract name
            if let Some(sources_list) = sources.get(&contract_address)
                && let Some((file_path, _)) = sources_list.first()
            {
                // TODO: Fix this. We don't have line/column info here
                frame = frame.with_source_location(
                    file_path.clone(),
                    0, // Unknown line
                    0, // Unknown column
                );
            }
        }

        // Only add the frame if it has meaningful information
        // Skip frames that are just forge-std internals with no real location
        let should_add = if let Some(ref file) = frame.file {
            // Skip forge-std frames with line 0 (these are compiler-generated)
            !(file.contains("lib/forge-std") && frame.line == Some(0))
        } else {
            // Include frames with contract/function names even without file info
            frame.contract_name.is_some() || frame.function_name.is_some()
        };

        if should_add {
            frames.push(frame);
        } else {
            tracing::info!(
                "Skipping frame for {} - forge-std internal or no meaningful info",
                contract_address
            );
        }

        // Move to parent node
        current_idx = node.parent;
    }

    // Reverse frames to have innermost first
    frames.reverse();

    if !frames.is_empty() { Some(Backtrace::new(frames)) } else { None }
}

/// Gets the source location from trace.
fn get_source_location_from_trace(
    trace: &foundry_evm::traces::CallTrace,
    contract_address: Address,
    pc_mappers: &HashMap<Address, PcSourceMapper>,
) -> Option<source_map::SourceLocation> {
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
        mapper.map_pc(pc)
    } else {
        None
    }
}

