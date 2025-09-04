//! Solidity stack trace support for test failures.

use alloy_primitives::{Address, map::HashMap};
use foundry_evm::traces::SparsedTraceArena;
use std::{fmt, path::PathBuf};
use yansi::Paint;

mod solidity;
pub mod source_map;

pub use solidity::{PcToSourceMapper, SourceLocation};
pub use source_map::PcSourceMapper;

use crate::backtrace::source_map::SourceData;

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
    pub file: Option<PathBuf>,
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
    pub fn with_source_location(mut self, file: PathBuf, line: usize, column: usize) -> Self {
        self.file = Some(file);
        self.line = Some(line);
        self.column = Some(column);
        self
    }

    /// Returns a formatted string for this frame.
    /// Format: <CONTRACT_NAME>.<FUNCTION_NAME> (FILE:LINE:COL)
    pub fn format(&self) -> String {
        let mut result = String::new();

        if let Some(contract) = &self.contract_name {
            result.push_str(contract);
        } else {
            // No contract name, show address
            result.push_str(&self.contract_address.to_string());
        }

        // Add function name if available
        if let Some(func) = &self.function_name {
            result.push('.');
            result.push_str(func);
        }

        // Add location in parentheses if available
        if self.file.is_some() || self.line.is_some() {
            result.push_str(" (");

            if let Some(file) = &self.file {
                result.push_str(&file.display().to_string());
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

/// Extracts a backtrace from a [`SparsedTraceArena`] using source data.
pub fn extract_backtrace(
    arena: &SparsedTraceArena,
    source_data: &HashMap<Address, SourceData>,
) -> Option<Backtrace> {
    let resolved_arena = &arena.arena;

    if resolved_arena.nodes().is_empty() {
        return None;
    }

    // Build PC source mappers for each contract
    let mut pc_mappers = HashMap::new();

    for (addr, data) in source_data {
        pc_mappers.insert(
            *addr,
            PcSourceMapper::new(&data.bytecode, data.source_map.clone(), data.sources.clone()),
        );
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

        let contract_address = trace.address;
        let mut frame = BacktraceFrame::new(contract_address);

        // Get contract and function names from decoded trace
        if let Some(decoded) = &trace.decoded {
            // Get contract name from label
            if let Some(label) = &decoded.label {
                frame = frame.with_contract_name(label.clone());
            }

            // Get function name from call_data if we don't have it yet
            if let Some(call_data) = &decoded.call_data {
                // Use the signature from decoded call data. Remove args.
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
        frames.push(frame);

        // Move to parent node
        current_idx = node.parent;
    }
    if !frames.is_empty() { Some(Backtrace::new(frames)) } else { None }
}
