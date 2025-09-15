//! Solidity stack trace support for test failures.

use alloy_primitives::{
    Address,
    map::{HashMap, HashSet},
};
use foundry_compilers::{
    ArtifactId, ProjectCompileOutput,
    artifacts::{Libraries, NodeType},
};
use foundry_evm::traces::{CallTrace, SparsedTraceArena};
use std::{fmt, path::PathBuf};
use yansi::Paint;
mod solidity;
pub mod source_map;
use crate::backtrace::source_map::collect_source_data;
pub use solidity::{PcToSourceMapper, SourceLocation};
pub use source_map::{LibraryInfo, PcSourceMapper, SourceData};

/// Collects [`LibraryInfo`] and holds references to [`ProjectCompileOutput`] and [`Config`] for
/// backtrace generation.
pub struct BacktraceBuilder<'a> {
    /// Libraries part of the current source contracts both internal and linked/external libraries.
    library_sources: HashSet<LibraryInfo>,
    /// Reference to project output for on-demand source loading
    output: &'a ProjectCompileOutput,
    /// Project root
    root: PathBuf,
}

impl<'a> BacktraceBuilder<'a> {
    /// Instantiates a backtrace builder from a [`ProjectCompileOutput`] and [`Config`].
    ///
    /// Collects artifact IDs and libraries without loading source data upfront.
    pub fn new(
        output: &'a ProjectCompileOutput,
        root: PathBuf,
        parsed_libs: Option<Libraries>,
    ) -> Self {
        let mut library_sources = HashSet::default();

        if let Some(parsed_libs) = &parsed_libs {
            for (path, libs) in &parsed_libs.libs {
                for (lib_name, addr_str) in libs {
                    if let Ok(addr) = addr_str.parse::<Address>() {
                        library_sources.insert(LibraryInfo::linked(
                            path.clone(),
                            lib_name.clone(),
                            addr,
                        ));
                    }
                }
            }
        }

        // Process all artifacts - collect artifact IDs and library info
        for (artifact_id, artifact) in output.artifact_ids() {
            // Check if this is a library artifact and collect library info
            if let Some(ast) = &artifact.ast {
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

                            let lib_path = artifact_id
                                .source
                                .strip_prefix(&root)
                                .unwrap_or(&artifact_id.source)
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

        Self { library_sources, output, root }
    }

    /// Generates a backtrace from a [`SparsedTraceArena`].
    pub fn from_traces(&self, arena: &SparsedTraceArena) -> Backtrace<'_> {
        let artifact_ids = self
            .output
            .artifact_ids()
            .map(|(id, _)| id.with_stripped_file_prefixes(&self.root))
            .collect::<HashSet<_>>();

        // Resolve addresses to artifacts using trace labels
        let artifacts_by_address = self.resolve_addresses(arena, &artifact_ids);

        let mut sources = HashMap::default();
        let external_lib_artifacts = self.library_sources.iter().filter_map(|lib| {
            if lib.is_linked() {
                return artifact_ids
                    .iter()
                    .find(|id| id.identifier() == format!("{}:{}", lib.path.display(), lib.name));
            }
            None
        });

        // Collect source data for artifacts
        for artifact_id in artifacts_by_address.values().chain(external_lib_artifacts) {
            // Find the actual artifact from the output
            for (output_id, artifact) in self.output.artifact_ids() {
                let stripped_id = output_id.with_stripped_file_prefixes(&self.root);
                if stripped_id == *artifact_id {
                    // Find the build_id for this specific artifact
                    let build_id = self
                        .output
                        .compiled_artifacts()
                        .artifact_files()
                        .chain(self.output.cached_artifacts().artifact_files())
                        .find_map(|af| {
                            // Match by checking if this artifact file corresponds to the same
                            // artifact
                            if std::ptr::eq(&raw const af.artifact, artifact as *const _) {
                                return Some(af.build_id.clone());
                            }
                            None
                        });

                    if let Some(build_id) = build_id
                        && let Some(data) =
                            collect_source_data(artifact, self.output, &self.root, &build_id)
                    {
                        sources.insert(artifact_id.clone(), data);
                    }
                    break;
                }
            }
        }

        let mut backtrace = Backtrace::new(artifacts_by_address, sources, &self.library_sources);

        backtrace.extract_frames(arena);

        backtrace
    }

    /// Resolves contract addresses to [`ArtifactId`]s using trace labels and provided
    /// [`ArtifactId`]s.
    fn resolve_addresses(
        &self,
        arena: &SparsedTraceArena,
        artifact_ids: &HashSet<ArtifactId>,
    ) -> HashMap<Address, ArtifactId> {
        let mut artifacts_by_address = HashMap::default();

        // Build contracts mapping from decoded traces
        for node in arena.arena.nodes() {
            if let Some(decoded) = &node.trace.decoded
                && let Some(label) = &decoded.label
            {
                // Find the artifact ID for this contract label from our collected artifact IDs
                for artifact_id in artifact_ids {
                    if artifact_id.name == *label {
                        // Store the artifact ID directly
                        artifacts_by_address.insert(node.trace.address, artifact_id.clone());
                        break;
                    }
                }
            }
        }

        artifacts_by_address
    }
}

/// A Solidity stack trace for a test failure.
///
/// Generates a backtrace from a [`SparsedTraceArena`] by leveraging [`SourceData`] and
/// [`LibraryInfo`].
///
/// It uses the program counter (PC) from the traces to map to a specific source location for the
/// call.
///
/// Each step/call in the backtrace is classified as a [`BacktraceFrame`].
pub struct Backtrace<'a> {
    /// The frames of the backtrace, from innermost (where the revert happened) to outermost.
    frames: Vec<BacktraceFrame>,
    /// PC to source mappers for each contract
    pc_mappers: HashMap<Address, PcSourceMapper>,
    /// Library sources (both internal and linked libraries)
    library_sources: &'a HashSet<LibraryInfo>,
}

impl<'a> Backtrace<'a> {
    /// Sets source data from pre-collected artifacts.
    pub fn new(
        artifacts_by_address: HashMap<Address, ArtifactId>,
        mut sources: HashMap<ArtifactId, SourceData>,
        library_sources: &'a HashSet<LibraryInfo>,
    ) -> Self {
        // Store library sources globally
        let mut backtrace =
            Self { frames: Vec::new(), pc_mappers: HashMap::default(), library_sources };

        let mut source_data = HashMap::new();
        // Map source data to contract addresses using the artifact ID, taking ownership
        for (addr, artifact_id) in artifacts_by_address {
            if let Some(data) = sources.remove(&artifact_id) {
                source_data.insert(addr, data);
            }
        }

        // Map linked library source data to their deployed addresses
        for lib_info in backtrace.library_sources {
            if lib_info.is_linked()
                && let Some(lib_addr) = lib_info.address
                && let Some(artifact_id) = sources.iter().find_map(|(artifact_id, _)| {
                    if artifact_id.name == lib_info.name {
                        return Some(artifact_id.clone());
                    }
                    None
                })
                && let Some(data) = sources.remove(&artifact_id)
            {
                source_data.insert(lib_addr, data);
            }
        }

        // Build PC source mappers for each contract
        for (addr, data) in source_data {
            backtrace.pc_mappers.insert(addr, PcSourceMapper::new(data));
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

    /// Handles frames that may be originating from an internal library for which bytecode gets
    /// inlined.
    ///
    /// If so, it will return a tuple of two frames: the first is the internal library frame and the
    /// second is the contract frame that called the library.
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
            self.library_sources,
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

    /// Returns true if the backtrace is empty.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
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
