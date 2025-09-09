//! Solidity stack trace support for test failures.

use alloy_primitives::{Address, map::HashMap};
use foundry_evm::traces::{CallTrace, SparsedTraceArena};
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
    let mut pc_mappers = HashMap::default();

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

        // Try to get source location from PC mapper
        if let Some(source_location) = trace.steps.last().and_then(|last_step| {
            pc_mappers.get(&contract_address).and_then(|m| m.map_pc(last_step.pc))
        }) {
            // Check if this source location is in a library file
            let is_library_file = source_data
                .get(&contract_address)
                .map(|data| {
                    data.library_sources
                        .iter()
                        .any(|(lib_path, _, _)| source_location.file == *lib_path)
                })
                .unwrap_or(false);

            if is_library_file {
                if let Some((library_frame, contract_frame)) = handle_library_frames(
                    &source_location,
                    trace,
                    contract_address,
                    source_data,
                    &pc_mappers,
                ) {
                    // Push the library frame
                    frames.push(library_frame);
                    // Use the contract frame with the call location
                    frame = contract_frame;
                }
            } else {
                frame = frame.with_source_location(
                    source_location.file.clone(),
                    source_location.line,
                    source_location.column,
                );
            }
        }

        // Add contract and function names from decoded trace
        if let Some(decoded) = &trace.decoded {
            // Get contract name from label
            if let Some(label) = &decoded.label {
                frame = frame.with_contract_name(label.clone());
            }

            // Get function name from call_data
            if let Some(call_data) = &decoded.call_data {
                // Use the signature from decoded call data. Remove args.
                let sig = &call_data.signature;
                let func_name =
                    if let Some(paren_pos) = sig.find('(') { &sig[..paren_pos] } else { sig };
                frame = frame.with_function_name(func_name.to_string());
            }
        }
        frames.push(frame);

        // Move to parent node
        current_idx = node.parent;
    }
    if !frames.is_empty() { Some(Backtrace::new(frames)) } else { None }
}

/// Handles library frame creation - identifies library and finds contract call location.
/// Returns (library_frame, contract_frame) or None if library can't be identified.
fn handle_library_frames(
    source_location: &source_map::SourceLocation,
    trace: &CallTrace,
    contract_address: Address,
    source_data: &HashMap<Address, SourceData>,
    pc_mappers: &HashMap<Address, PcSourceMapper>,
) -> Option<(BacktraceFrame, BacktraceFrame)> {
    // Try to identify which specific library
    let library_name = source_data
        .get(&contract_address)
        .and_then(|data| identify_library_for_location(source_location, data))?;

    let library_frame = BacktraceFrame::new(contract_address)
        .with_contract_name(library_name)
        .with_source_location(
            source_location.file.clone(),
            source_location.line,
            source_location.column,
        );

    // Find where in the contract the library was called
    let contract_call_location = trace
        .steps
        .iter()
        .rev()
        .skip(1) // Skip the last step (already in library)
        .find_map(|step| {
            pc_mappers.get(&contract_address).and_then(|m| {
                m.map_pc(step.pc).filter(|loc| {
                    // Check if this step is NOT in a library file
                    source_data
                        .get(&contract_address)
                        .map(|data| {
                            !data
                                .library_sources
                                .iter()
                                .any(|(lib_path, _, _)| loc.file == *lib_path)
                        })
                        .unwrap_or(true)
                })
            })
        })?;

    let contract_frame = BacktraceFrame::new(contract_address).with_source_location(
        contract_call_location.file,
        contract_call_location.line,
        contract_call_location.column,
    );

    Some((library_frame, contract_frame))
}

/// Identifies which library contains a source location when multiple libraries exist in the same
/// file.
fn identify_library_for_location(
    source_location: &source_map::SourceLocation,
    data: &SourceData,
) -> Option<String> {
    // Find libraries matching this source file
    let libs_in_file = data
        .library_sources
        .iter()
        .filter(|(lib_path, _, _)| source_location.file == **lib_path)
        .map(|(_, lib_name, range)| (lib_name.clone(), *range))
        .collect::<Vec<_>>();

    match libs_in_file.len() {
        0 => None,
        1 => Some(libs_in_file[0].0.clone()),
        _ => {
            // Multiple libraries in the same file - need to calculate byte offset and check ranges
            // First, try to find the source content to calculate byte offset
            let byte_offset = data
                .sources
                .iter()
                .find(|(path, _)| source_location.file == *path)
                .and_then(|(_, content)| calculate_byte_offset(content, source_location));

            // Now find the library that contains this byte offset
            byte_offset.and_then(|offset| {
                libs_in_file
                    .iter()
                    .find(|(_, range)| {
                        range.is_some_and(|(start, end)| offset >= start && offset < end)
                    })
                    .map(|(name, _)| name.clone())
            })
        }
    }
}

/// Calculates the byte offset in source content from line and column numbers.
fn calculate_byte_offset(
    content: &str,
    source_location: &source_map::SourceLocation,
) -> Option<usize> {
    let mut offset = 0;
    let mut current_line = 1;
    let mut current_col = 1;

    for ch in content.chars() {
        if current_line == source_location.line {
            if current_col == source_location.column {
                return Some(offset);
            }
            current_col += 1;
        }

        if ch == '\n' {
            current_line += 1;
            current_col = 1;
        }
        offset += ch.len_utf8();
    }
    None
}
