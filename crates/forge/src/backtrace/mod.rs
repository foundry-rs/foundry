//! Solidity stack trace support for test failures.

use alloy_primitives::{Address, Bytes};
use foundry_common::contracts::ContractsByAddress;
use foundry_compilers::artifacts::sourcemap::SourceMap;
use foundry_evm::traces::SparsedTraceArena;
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

        writeln!(f, "{}", Paint::yellow("Backtrace:"))?;

        for frame in &self.frames {
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
    /// Format: <CONTRACT_NAME>.<FUNCTION_NAME> (FILE:LINE:COL)
    pub fn format(&self) -> String {
        let mut result = String::new();

        // Start with contract name
        if let Some(ref contract) = self.contract_name {
            // Extract just the contract name if it includes a path
            let contract_only =
                if let Some(pos) = contract.rfind(':') { &contract[pos + 1..] } else { contract };
            result.push_str(contract_only);
        } else {
            // No contract name, show address
            result.push_str(&self.contract_address.to_string());
        }

        // Add function name if available
        if let Some(ref func) = self.function_name {
            result.push('.');
            result.push_str(func);
        }

        // Add location in parentheses if available
        if self.file.is_some() || self.line.is_some() {
            result.push_str(" (");

            if let Some(ref file) = self.file {
                result.push_str(file);
            } else {
                result.push_str("unknown");
            }

            if let Some(line) = self.line {
                result.push(':');
                result.push_str(&line.to_string());

                if let Some(column) = self.column {
                    result.push(':');
                    result.push_str(&column.to_string());
                } else {
                    result.push_str(":0");
                }
            }

            result.push(')');
        }

        result
    }
}

impl fmt::Display for BacktraceFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format())
    }
}

/// Extracts a backtrace from a decoded call trace arena with source information.
pub fn extract_backtrace(
    arena: &SparsedTraceArena,
    _contracts: &ContractsByAddress,
    source_maps: &HashMap<Address, (SourceMap, SourceMap)>, // (creation, runtime)
    sources: &HashMap<Address, Vec<(String, String)>>,      // Source files per contract
    deployed_bytecodes: &HashMap<Address, Bytes>,           // Deployed bytecode for each contract
) -> Option<Backtrace> {
    let resolved_arena = &arena.arena;

    if resolved_arena.nodes().is_empty() {
        return None;
    }

    // Build PC source mappers for each contract
    let mut pc_mappers: HashMap<Address, PcSourceMapper> = HashMap::new();

    for (addr, (_creation_map, runtime_map)) in source_maps {
        if let Some(contract_sources) = sources.get(addr)
            && let Some(bytecode) = deployed_bytecodes.get(addr)
        {
            pc_mappers.insert(
                *addr,
                PcSourceMapper::new(bytecode, runtime_map.clone(), contract_sources.clone()),
            );
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
        let mut frame = BacktraceFrame::new(contract_address);

        // Get contract and function names from decoded trace
        if let Some(decoded) = &trace.decoded {
            // Get contract name from label
            if let Some(ref label) = decoded.label {
                // Label format: "ContractName::functionName" or just "ContractName"
                let parts: Vec<&str> = label.split("::").collect();
                if parts.len() > 1 {
                    // We have both contract and function in the label
                    let contract_name = parts[0];
                    let func_part = parts[1];
                    // Remove parentheses and arguments from function name
                    let func_name = if let Some(paren_pos) = func_part.find('(') {
                        &func_part[..paren_pos]
                    } else {
                        func_part
                    };
                    frame = frame
                        .with_contract_name(contract_name.to_string())
                        .with_function_name(func_name.to_string());
                } else {
                    // Label only has contract name
                    frame = frame.with_contract_name(label.to_string());
                }
            }

            // Get function name from call_data if we don't have it yet
            if frame.function_name.is_none()
                && let Some(ref call_data) = decoded.call_data
            {
                // Use the signature from decoded call data, but remove args
                let sig = &call_data.signature;
                let func_name =
                    if let Some(paren_pos) = sig.find('(') { &sig[..paren_pos] } else { sig };
                frame = frame.with_function_name(func_name.to_string());
            }
        }

        // Try to get source location from PC mapper
        if let Some(source_location) = trace.steps.last().and_then(|last_step| {
            pc_mappers.get(&contract_address).and_then(|m| m.map_pc(last_step.pc))
        }) {
            frame = frame.with_source_location(
                source_location.file,
                source_location.line,
                source_location.column,
            );
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
        }

        // Move to parent node
        current_idx = node.parent;
    }

    // Reverse frames to have innermost first
    frames.reverse();

    if !frames.is_empty() { Some(Backtrace::new(frames)) } else { None }
}
