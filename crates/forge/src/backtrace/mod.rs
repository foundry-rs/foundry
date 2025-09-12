//! Solidity stack trace support for test failures.

use alloy_primitives::{
    Address,
    map::{HashMap, HashSet},
};
use foundry_common::ContractsByArtifact;
use foundry_compilers::{ProjectCompileOutput, artifacts::NodeType};
use foundry_config::Config;
use foundry_evm::traces::{CallTrace, SparsedTraceArena};
use std::{
    fmt,
    path::{Path, PathBuf},
};
use yansi::Paint;

mod solidity;
pub mod source_map;

pub use solidity::{PcToSourceMapper, SourceLocation};
pub use source_map::{LibraryInfo, PcSourceMapper, SourceData};

use crate::backtrace::source_map::collect_source_data;

pub struct BacktraceBuilder {
    /// Mapping of [`ArtifactId::identifier`] i.e <source_path>:<contract_name> to [`SourceData`]
    ///
    /// [`ArtifactId::identifier`]: foundry_compilers::ArtifactId::identifier
    pub source_data: HashMap<String, SourceData>,
    /// Libraries part of the current source contracts both internal and linked/external libraries.
    pub library_sources: HashSet<LibraryInfo>,
}

impl BacktraceBuilder {
    /// Instantiates a backtrace builder from a [`ProjectCompileOutput`] and [`Config`].
    ///
    /// Collects the sources and libraries part of the current source contracts both internal and
    /// linked/external.
    pub fn new(output: &ProjectCompileOutput, config: &Config) -> Self {
        let mut source_data = HashMap::default();
        let mut library_sources = HashSet::default();

        // Collect linked libraries from config
        for lib_mapping in &config.libraries {
            // Parse library mappings like
            // "src/libraries/ExternalMathLib.sol:ExternalMathLib:0x1234..."
            let parts: Vec<&str> = lib_mapping.split(':').collect();
            if parts.len() == 3 {
                let path_str = parts[0];
                let lib_name = parts[1];
                let addr_str = parts[2];
                if let Ok(addr) = addr_str.parse::<Address>() {
                    let lib_path = Path::new(path_str)
                        .strip_prefix(&config.root)
                        .unwrap_or(Path::new(path_str))
                        .to_path_buf();
                    library_sources.insert(LibraryInfo::linked(
                        lib_path,
                        lib_name.to_string(),
                        addr,
                    ));
                }
            }
        }

        // Collect all library artifacts
        for (lib_id, lib_artifact) in output.artifact_ids() {
            // Check if this is a library artifact
            if let Some(ast) = &lib_artifact.ast {
                for node in &ast.nodes {
                    if node.node_type == NodeType::ContractDefinition {
                        let is_library = node
                            .other
                            .get("contractKind")
                            .and_then(|v| v.as_str())
                            .map(|kind| kind == "library")
                            .unwrap_or(false);

                        if is_library
                            && let Some(name) = node.other.get("name").and_then(|v| v.as_str())
                        {
                            let byte_range = node
                                .src
                                .length
                                .filter(|&l| l > 0)
                                .map(|length| (node.src.start, node.src.start + length));

                            let lib_path = lib_id
                                .source
                                .strip_prefix(&config.root)
                                .unwrap_or(&lib_id.source)
                                .to_path_buf();

                            library_sources.insert(LibraryInfo::internal(
                                lib_path,
                                name.to_string(),
                                byte_range,
                            ));
                        }
                    }
                }
            }
        }

        for (artifact_id, artifact) in output.artifact_ids() {
            // Find the build_id for this specific artifact
            let build_id = output
                .compiled_artifacts()
                .artifact_files()
                .chain(output.cached_artifacts().artifact_files())
                .find(|af| {
                    // Match by checking if this artifact file corresponds to the
                    // same artifact
                    std::ptr::eq(&raw const af.artifact, artifact as *const _)
                })
                .map(|af| af.build_id.clone());

            let Some(build_id) = build_id else {
                continue;
            };

            if let Some(data) = collect_source_data(artifact, output, config, &build_id) {
                // Use the stripped identifier for consistency
                let id = artifact_id.with_stripped_file_prefixes(&config.root);
                source_data.insert(id.identifier(), data);
            }
        }

        Self { source_data, library_sources }
    }

    pub fn from_traces(
        &self,
        arena: &SparsedTraceArena,
        known_contracts: &ContractsByArtifact,
    ) -> Backtrace<'_> {
        let resolved_contracts = self.resolve_contracts(arena, known_contracts);

        let mut backtrace =
            Backtrace::new(&resolved_contracts, &self.source_data, &self.library_sources);

        backtrace.extract_frames(arena);

        backtrace
    }

    /// Uses the [`SparsedTraceArena`] to map the contract addresses to their
    /// [`ArtifactId::identifier`].
    ///
    /// [`ArtifactId::identifier`]: foundry_compilers::ArtifactId::identifier
    fn resolve_contracts(
        &self,
        arena: &SparsedTraceArena,
        known_contracts: &ContractsByArtifact,
    ) -> HashMap<Address, String> {
        // Build contracts mapping from decoded traces
        let mut contracts_by_address = HashMap::new();
        for node in arena.arena.nodes() {
            if let Some(decoded) = &node.trace.decoded
                && let Some(label) = &decoded.label
            {
                contracts_by_address.insert(node.trace.address, label.clone());
            }
        }

        // Resolve labels to full identifiers using known_contracts
        let mut resolved_contracts = HashMap::default();
        for (addr, label) in &contracts_by_address {
            let mut found = false;

            // Find the full identifier for this contract label
            for (artifact_id, _) in known_contracts.iter() {
                if artifact_id.name == *label {
                    resolved_contracts.insert(*addr, artifact_id.identifier());
                    found = true;
                    break;
                }
            }

            // If no match found, keep the original label (might be needed for external contracts)
            if !found {
                resolved_contracts.insert(*addr, label.clone());
            }
        }

        resolved_contracts
    }
}

/// A Solidity stack trace for a test failure.
pub struct Backtrace<'a> {
    /// The frames of the backtrace, from innermost (where the revert happened) to outermost.
    frames: Vec<BacktraceFrame>,
    /// PC to source mappers for each contract
    pc_mappers: HashMap<Address, PcSourceMapper>,
    /// Library sources (both internal and linked libraries)
    library_sources: &'a HashSet<LibraryInfo>,
}

impl<'a> Backtrace<'a> {
    /// Returns true if the backtrace is empty.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Sets source data from pre-collected artifacts.
    pub fn new(
        contracts_by_address: &HashMap<Address, String>,
        source_data_by_artifact: &HashMap<String, SourceData>,
        library_sources: &'a HashSet<LibraryInfo>,
    ) -> Self {
        // Store library sources globally
        let mut backtrace =
            Self { frames: Vec::new(), pc_mappers: HashMap::default(), library_sources };

        let mut source_data = HashMap::new();
        // Map source data to contract addresses using the contract identifier.
        for (addr, contract_identifier) in contracts_by_address {
            if let Some(data) = source_data_by_artifact.get(contract_identifier) {
                source_data.insert(*addr, data.clone());
            }
        }

        // Add linked libraries to the address mapping
        let mut linked_lib_addresses: HashMap<String, Address> = HashMap::default();
        for lib_info in backtrace.library_sources {
            if lib_info.is_linked()
                && let Some(lib_addr) = lib_info.address
            {
                linked_lib_addresses.insert(lib_info.name.clone(), lib_addr);
            }
        }

        // Map linked library source data to their deployed addresses
        for (lib_name, lib_addr) in linked_lib_addresses {
            // Find matching artifact by checking if identifier ends with the library name
            for (identifier, data) in source_data_by_artifact {
                if identifier.ends_with(&format!(":{lib_name}")) || identifier == &lib_name {
                    source_data.insert(lib_addr, data.clone());
                    break;
                }
            }
        }

        // Build PC source mappers for each contract
        for (addr, data) in &source_data {
            backtrace.pc_mappers.insert(
                *addr,
                PcSourceMapper::new(&data.bytecode, data.source_map.clone(), data.sources.clone()),
            );
        }

        backtrace
    }

    /// Extracts backtrace frames from a trace arena.
    pub fn extract_frames(&mut self, arena: &SparsedTraceArena) -> bool {
        let resolved_arena = &arena.arena;

        if resolved_arena.nodes().is_empty() {
            return false;
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

        let Some(deepest_idx) = deepest_failed_idx else {
            return false;
        };

        // Build the call stack by walking from the deepest node back to root
        let mut frames = Vec::new();
        let mut current_idx = Some(deepest_idx);

        while let Some(idx) = current_idx {
            let node = &resolved_arena.nodes()[idx];
            let trace = &node.trace;

            if let Some(frame) = self.create_frame(trace) {
                // Check if this is an internal library frame that needs special handling
                if let Some((library_frame, contract_frame)) =
                    self.handle_internal_library_frame(&frame, trace)
                {
                    frames.push(library_frame);
                    frames.push(contract_frame);
                } else {
                    frames.push(frame);
                }
            }

            current_idx = node.parent;
        }

        self.frames = frames;
        !self.frames.is_empty()
    }

    /// Creates a frame from a call trace.
    fn create_frame(&self, trace: &CallTrace) -> Option<BacktraceFrame> {
        let contract_address = trace.address;
        let mut frame = BacktraceFrame::new(contract_address);

        // Try to get source location from PC mapper
        if let Some(source_location) = trace.steps.last().and_then(|last_step| {
            self.pc_mappers.get(&contract_address).and_then(|m| m.map_pc(last_step.pc))
        }) {
            frame = frame
                .with_source_location(
                    source_location.file,
                    source_location.line,
                    source_location.column,
                )
                .with_byte_offset(source_location.offset);
        }

        // Add contract name from decoded trace or linked library info
        if let Some(decoded) = &trace.decoded
            && let Some(label) = &decoded.label
        {
            frame = frame.with_contract_name(label.clone());
        }

        // If no contract name yet, check if this is a linked library
        if frame.contract_name.is_none()
            && let Some(lib_info) = self.find_linked_library(contract_address)
        {
            frame = frame.with_contract_name(lib_info.name.clone());
        }

        // Add function name from decoded trace
        if let Some(decoded) = &trace.decoded
            && let Some(call_data) = &decoded.call_data
        {
            let sig = &call_data.signature;
            let func_name =
                if let Some(paren_pos) = sig.find('(') { &sig[..paren_pos] } else { sig };
            frame = frame.with_function_name(func_name.to_string());
        }

        Some(frame)
    }

    /// Handles internal library frames that need special treatment.
    fn handle_internal_library_frame(
        &self,
        frame: &BacktraceFrame,
        trace: &CallTrace,
    ) -> Option<(BacktraceFrame, BacktraceFrame)> {
        let contract_address = trace.address;

        // Check if this frame is in an internal library
        let source_location = frame.file.as_ref()?;

        let is_internal_library = self
            .library_sources
            .iter()
            .any(|lib| !lib.is_linked() && lib.matches_path(source_location));

        if !is_internal_library {
            return None;
        }

        // Identify which library using byte offset if available
        let library_name = identify_library_for_location(
            source_location,
            frame.byte_offset,
            &self.library_sources,
        )?;

        let mut library_frame = BacktraceFrame::new(contract_address)
            .with_contract_name(library_name)
            .with_source_location(
                source_location.clone(),
                frame.line.unwrap_or(0),
                frame.column.unwrap_or(0),
            );
        if let Some(offset) = frame.byte_offset {
            library_frame = library_frame.with_byte_offset(offset);
        }
        // Add the function name to the library frame
        if let Some(function_name) = &frame.function_name {
            library_frame = library_frame.with_function_name(function_name.clone());
        }

        // Find where in the contract the library was called
        let contract_call_location = trace
            .steps
            .iter()
            .rev()
            .skip(1) // Skip the last step (already in library)
            .find_map(|step| {
                self.pc_mappers.get(&contract_address).and_then(|m| {
                    m.map_pc(step.pc).filter(|loc| {
                        // Check if this step is NOT in a library file
                        !self.library_sources.iter().any(|lib| lib.matches_path(&loc.file))
                    })
                })
            })?;

        let mut contract_frame = BacktraceFrame::new(contract_address)
            .with_source_location(
                contract_call_location.file,
                contract_call_location.line,
                contract_call_location.column,
            )
            .with_byte_offset(contract_call_location.offset);

        // Add contract and function names to the contract frame
        if let Some(contract_name) = &frame.contract_name {
            contract_frame = contract_frame.with_contract_name(contract_name.clone());
        }
        if let Some(function_name) = &frame.function_name {
            contract_frame = contract_frame.with_function_name(function_name.clone());
        }

        Some((library_frame, contract_frame))
    }

    /// Finds a linked library by address.
    fn find_linked_library(&self, address: Address) -> Option<&LibraryInfo> {
        self.library_sources.iter().find(|lib| lib.address == Some(address) && lib.is_linked())
    }
}

impl<'a> fmt::Display for Backtrace<'a> {
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
    /// The byte offset in the source file.
    pub byte_offset: Option<usize>,
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
            byte_offset: None,
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

    /// Sets the byte offset.
    pub fn with_byte_offset(mut self, offset: usize) -> Self {
        self.byte_offset = Some(offset);
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

/// Helper function to identify which library contains a source location.
fn identify_library_for_location(
    source_path: &PathBuf,
    byte_offset: Option<usize>,
    library_sources: &HashSet<LibraryInfo>,
) -> Option<String> {
    // Find libraries matching this source file (internal libraries only)
    let libs_in_file: Vec<_> = library_sources
        .iter()
        .filter(|lib| !lib.is_linked() && lib.matches_path(source_path))
        .collect();

    match libs_in_file.len() {
        0 => None,
        1 => Some(libs_in_file[0].name.clone()),
        _ => {
            // Multiple libraries in the same file - use byte offset to determine which one
            if let Some(offset) = byte_offset {
                // Find the library that contains this byte offset
                for lib in libs_in_file {
                    if let Some((start, end)) = lib.byte_range
                        && offset >= start
                        && offset < end
                    {
                        return Some(lib.name.clone());
                    }
                }
            }
            // Cannot determine which library without a valid byte offset
            None
        }
    }
}
