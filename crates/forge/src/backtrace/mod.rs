//! Solidity stack trace support for test failures.

use alloy_primitives::{Address, map::HashMap};
use foundry_compilers::{ArtifactId, ProjectCompileOutput, artifacts::Libraries};
use foundry_evm::traces::{CallTrace, SparsedTraceArena};
use std::{fmt, path::PathBuf};
use yansi::Paint;

mod source_map;
use source_map::collect_source_data;
pub use source_map::{PcSourceMapper, SourceData};

/// Linked library information for backtrace resolution.
///
/// Contains the path, name, and deployed address of a linked library
/// to enable proper frame resolution in backtraces.
#[derive(Debug, Clone)]
struct LinkedLib {
    /// The source file path of the library
    path: PathBuf,
    /// The name of the library contract
    name: String,
    /// The deployed address of the library
    address: Address,
}

/// Holds references to [`ProjectCompileOutput`] and config for backtrace generation.
pub struct BacktraceBuilder<'a> {
    /// Linked libraries from configuration
    linked_libraries: Vec<LinkedLib>,
    /// Reference to project output for on-demand source loading
    output: &'a ProjectCompileOutput,
    /// Project root
    root: PathBuf,
    /// Disable source locations
    ///
    /// Source locations will be inaccurately reported if the files have been compiled with via-ir
    disable_source_locs: bool,
}

impl<'a> BacktraceBuilder<'a> {
    /// Instantiates a backtrace builder from a [`ProjectCompileOutput`].
    pub fn new(
        output: &'a ProjectCompileOutput,
        root: PathBuf,
        linked_libraries: Option<Libraries>,
        disable_source_locs: bool,
    ) -> Self {
        let linked_libs = linked_libraries
            .map(|libs| {
                libs.libs
                    .iter()
                    .flat_map(|(path, libs_map)| {
                        libs_map.iter().map(move |(name, addr_str)| (path, name, addr_str))
                    })
                    .filter_map(|(path, name, addr_str)| {
                        addr_str.parse().ok().map(|address| LinkedLib {
                            path: path.clone(),
                            name: name.clone(),
                            address,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Self { linked_libraries: linked_libs, output, root, disable_source_locs }
    }

    /// Generates a backtrace from a [`SparsedTraceArena`].
    pub fn from_traces(&self, arena: &SparsedTraceArena) -> Backtrace {
        // Resolve addresses to artifacts using trace labels
        let mut artifacts_by_address = self.resolve_addresses(arena);

        let mut sources = HashMap::default();

        // Add linked library artifacts and their addresses
        for lib in &self.linked_libraries {
            let target_id = format!("{}:{}", lib.path.display(), lib.name);
            if let Some(artifact_id) = self.output.artifact_ids().find_map(|(id, _)| {
                let stripped = id.with_stripped_file_prefixes(&self.root);
                (stripped.identifier() == target_id).then_some(stripped)
            }) {
                artifacts_by_address.insert(lib.address, artifact_id);
            }
        }

        // Collect source data for all needed artifacts
        for artifact_id in artifacts_by_address.values() {
            // Find the actual artifact from the output
            for (output_id, artifact) in self.output.artifact_ids() {
                let stripped_id = output_id.with_stripped_file_prefixes(&self.root);
                if stripped_id == *artifact_id {
                    // Find the build_id for this specific artifact
                    let build_id = self.output.artifact_ids().find_map(|(id, arti)| {
                        // Match by checking if this artifact file corresponds to the same
                        // artifact
                        if std::ptr::eq(arti, artifact) {
                            return Some(id.build_id);
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

        let mut backtrace = Backtrace::new(
            artifacts_by_address,
            sources,
            self.linked_libraries.clone(),
            self.disable_source_locs,
        );

        backtrace.extract_frames(arena);

        backtrace
    }

    /// Resolves contract addresses to [`ArtifactId`]s using trace labels.
    fn resolve_addresses(&self, arena: &SparsedTraceArena) -> HashMap<Address, ArtifactId> {
        let mut artifacts_by_address = HashMap::default();

        // Build contracts mapping from decoded traces
        for node in arena.arena.nodes() {
            if let Some(decoded) = &node.trace.decoded
                && let Some(label) = &decoded.label
            {
                // Only iterate through artifacts to find matches for labels in the trace
                for (output_id, _) in self.output.artifact_ids() {
                    let stripped_id = output_id.with_stripped_file_prefixes(&self.root);
                    if stripped_id.name == *label {
                        // Store the artifact ID directly
                        artifacts_by_address.insert(node.trace.address, stripped_id);
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
/// Generates a backtrace from a [`SparsedTraceArena`] by leveraging [`SourceData`].
///
/// It uses the program counter (PC) from the traces to map to a specific source location for the
/// call.
///
/// Each step/call in the backtrace is classified as a [`BacktraceFrame`].
pub struct Backtrace {
    /// The frames of the backtrace, from innermost (where the revert happened) to outermost.
    frames: Vec<BacktraceFrame>,
    /// PC to source mappers for each contract
    pc_mappers: HashMap<Address, PcSourceMapper>,
    /// Linked libraries from configuration
    linked_libraries: Vec<LinkedLib>,
    /// Disable pinpointing source locations in files
    ///
    /// Should be disabled when via-ir is enabled
    disable_source_locs: bool,
}

impl Backtrace {
    /// Sets source data from pre-collected artifacts.
    fn new(
        artifacts_by_address: HashMap<Address, ArtifactId>,
        mut sources: HashMap<ArtifactId, SourceData>,
        linked_libraries: Vec<LinkedLib>,
        disable_source_locs: bool,
    ) -> Self {
        let mut backtrace = Self {
            frames: Vec::new(),
            pc_mappers: HashMap::default(),
            linked_libraries,
            disable_source_locs,
        };

        // Build PC source mappers for each contract
        if !disable_source_locs {
            for (addr, artifact_id) in artifacts_by_address {
                if let Some(data) = sources.remove(&artifact_id) {
                    backtrace.pc_mappers.insert(addr, PcSourceMapper::new(data));
                }
            }
        }

        backtrace
    }

    /// Extracts backtrace frames from a trace arena.
    fn extract_frames(&mut self, arena: &SparsedTraceArena) {
        let resolved_arena = &arena.arena;

        if resolved_arena.nodes().is_empty() {
            return;
        }

        // Find the deepest failed node (where the actual revert happened)
        let mut deepest_idx = None;
        let mut max_depth = 0;

        for (idx, node) in resolved_arena.nodes().iter().enumerate() {
            if !node.trace.success && node.trace.depth >= max_depth {
                max_depth = node.trace.depth;
                deepest_idx = Some(idx);
            }
        }

        if deepest_idx.is_none() {
            return;
        }

        // Build the call stack by walking from the deepest node back to root
        let mut current_idx = Some(deepest_idx.unwrap());

        while let Some(idx) = current_idx {
            let node = &resolved_arena.nodes()[idx];
            let trace = &node.trace;

            if let Some(frame) = self.create_frame(trace) {
                self.frames.push(frame);
            }

            current_idx = node.parent;
        }
    }

    /// Creates a frame from a call trace.
    fn create_frame(&self, trace: &CallTrace) -> Option<BacktraceFrame> {
        let contract_address = trace.address;
        let mut frame = BacktraceFrame::new(contract_address);

        // Try to get source location from PC mapper
        if !self.disable_source_locs
            && let Some(source_location) = trace.steps.last().and_then(|last_step| {
                self.pc_mappers.get(&contract_address).and_then(|m| m.map_pc(last_step.pc))
            })
        {
            frame = frame
                .with_source_location(
                    source_location.file,
                    source_location.line,
                    source_location.column,
                )
                .with_byte_offset(source_location.offset);
        }

        if let Some(decoded) = &trace.decoded {
            if let Some(label) = &decoded.label {
                frame = frame.with_contract_name(label.clone());
            } else {
                // Check if this is a linked library by address
                if let Some(lib) =
                    self.linked_libraries.iter().find(|l| l.address == contract_address)
                {
                    frame = frame.with_contract_name(lib.name.clone());
                }
            }

            if let Some(call_data) = &decoded.call_data {
                let sig = &call_data.signature;
                let func_name =
                    if let Some(paren_pos) = sig.find('(') { &sig[..paren_pos] } else { sig };
                frame = frame.with_function_name(func_name.to_string());
            }
        }

        Some(frame)
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

        // No contract name, show address
        result.push_str(self.contract_name.as_ref().unwrap_or(&self.contract_address.to_string()));

        // Add function name if available
        result.push_str(&self.function_name.as_ref().map_or(String::new(), |f| format!(".{f}")));

        if let Some(file) = &self.file {
            result.push_str(" (");
            result.push_str(&file.display().to_string());
        }

        if let Some(line) = self.line {
            result.push(':');
            result.push_str(&line.to_string());

            result.push(':');
            result.push_str(&self.column.as_ref().map_or("0".to_string(), |c| c.to_string()));
        }

        // Add location in parentheses if available
        if self.file.is_some() || self.line.is_some() {
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
