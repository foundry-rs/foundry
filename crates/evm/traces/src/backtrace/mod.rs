//! Solidity stack trace support for test failures.

use crate::{CallTrace, SparsedTraceArena};
use alloy_primitives::{Address, Bytes, map::HashMap};
use foundry_compilers::{
    Artifact, ArtifactId, ProjectCompileOutput,
    artifacts::{ConfigurableContractArtifact, Libraries, sourcemap::SourceMap},
};
use std::{fmt, path::PathBuf};
use yansi::Paint;

mod source_map;
use source_map::load_build_sources;
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

/// Holds a reference to [`ProjectCompileOutput`] to fetch artifacts and sources for backtrace
/// generation.
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
    /// Sources grouped by [`ArtifactId::build_id`] to avoid re-reading files for artifacts from
    /// the same build
    ///
    /// The source [`Vec`] is indexed by the compiler source ID, and contains the source path and
    /// source content.
    build_sources_cache: HashMap<String, Vec<(PathBuf, String)>>,
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

        Self {
            linked_libraries: linked_libs,
            output,
            root,
            disable_source_locs,
            build_sources_cache: HashMap::default(),
        }
    }

    /// Generates a backtrace from a [`SparsedTraceArena`].
    pub fn from_traces(&mut self, arena: &SparsedTraceArena) -> Backtrace<'_> {
        // Resolve addresses to artifacts using trace labels and linked libraries
        let artifacts_by_address = self.resolve_addresses(arena);
        for (artifact_id, _) in artifacts_by_address.values() {
            let build_id = &artifact_id.build_id;
            if !self.build_sources_cache.contains_key(build_id)
                && let Some(sources) = load_build_sources(build_id, self.output, &self.root)
            {
                self.build_sources_cache.insert(build_id.clone(), sources);
            }
        }

        Backtrace::new(
            artifacts_by_address,
            &self.build_sources_cache,
            self.linked_libraries.clone(),
            self.disable_source_locs,
            arena,
        )
    }

    /// Resolves contract addresses to [`ArtifactId`] and their [`SourceData`] from trace labels and
    /// linked libraries.
    fn resolve_addresses(
        &self,
        arena: &SparsedTraceArena,
    ) -> HashMap<Address, (ArtifactId, SourceData)> {
        let mut artifacts_by_address = HashMap::default();

        // Collect all labels from traces first
        let label_to_address = arena
            .nodes()
            .iter()
            .filter_map(|node| {
                if let Some(decoded) = &node.trace.decoded
                    && let Some(label) = &decoded.label
                {
                    return Some((label.as_str(), node.trace.address));
                }
                None
            })
            .collect::<HashMap<_, _>>();

        // Build linked library target IDs
        let linked_lib_targets = self
            .linked_libraries
            .iter()
            .map(|lib| (format!("{}:{}", lib.path.display(), lib.name), lib.address))
            .collect::<HashMap<_, _>>();

        let get_source = |artifact: &ConfigurableContractArtifact| -> Option<(SourceMap, Bytes)> {
            let source_map = artifact.get_source_map_deployed()?.ok()?;
            let deployed_bytecode = artifact.get_deployed_bytecode_bytes()?.into_owned();

            if deployed_bytecode.is_empty() {
                return None;
            }

            Some((source_map, deployed_bytecode))
        };

        for (artifact_id, artifact) in self.output.artifact_ids() {
            // Match and insert artifacts using trace labels
            if let Some(address) = label_to_address.get(artifact_id.name.as_str())
                && let Some((source_map, bytecode)) = get_source(artifact)
            {
                // Match and insert artifacts using trace labels
                artifacts_by_address
                    .insert(*address, (artifact_id.clone(), SourceData { source_map, bytecode }));
            } else if let Some(&lib_address) =
                // Match and insert the linked library artifacts
                linked_lib_targets.get(&artifact_id.identifier()).or_else(|| {
                        let id = artifact_id
                            .clone()
                            .with_stripped_file_prefixes(&self.root)
                            .identifier();
                        linked_lib_targets.get(&id)
                    })
                && let Some((source_map, bytecode)) = get_source(artifact)
            {
                // Insert linked libraries
                artifacts_by_address
                    .insert(lib_address, (artifact_id, SourceData { source_map, bytecode }));
            }
        }

        artifacts_by_address
    }
}

/// A Solidity stack trace for a test failure.
///
/// Generates a backtrace from a [`SparsedTraceArena`] by leveraging source maps and bytecode.
///
/// It uses the program counter (PC) from the traces to map to a specific source location for the
/// call.
///
/// Each step/call in the backtrace is classified as a BacktraceFrame
#[non_exhaustive]
pub struct Backtrace<'a> {
    /// The frames of the backtrace, from innermost (where the revert happened) to outermost.
    frames: Vec<BacktraceFrame>,
    /// Map from address to PcSourceMapper
    pc_mappers: HashMap<Address, PcSourceMapper<'a>>,
    /// Linked libraries from configuration
    linked_libraries: Vec<LinkedLib>,
    /// Disable pinpointing source locations in files
    ///
    /// Should be disabled when via-ir is enabled
    disable_source_locs: bool,
}

impl<'a> Backtrace<'a> {
    /// Creates a backtrace from collected artifacts and sources.
    fn new(
        artifacts_by_address: HashMap<Address, (ArtifactId, SourceData)>,
        build_sources: &'a HashMap<String, Vec<(PathBuf, String)>>,
        linked_libraries: Vec<LinkedLib>,
        disable_source_locs: bool,
        arena: &SparsedTraceArena,
    ) -> Self {
        let mut pc_mappers = HashMap::default();

        // Build PC source mappers for each contract
        if !disable_source_locs {
            for (addr, (artifact_id, source_data)) in artifacts_by_address {
                if let Some(sources) = build_sources.get(&artifact_id.build_id) {
                    let mapper = PcSourceMapper::new(source_data, sources);
                    pc_mappers.insert(addr, mapper);
                }
            }
        }

        let mut backtrace =
            Self { frames: Vec::new(), pc_mappers, linked_libraries, disable_source_locs };

        backtrace.extract_frames(arena);

        backtrace
    }

    /// Extracts backtrace frames from a trace arena.
    fn extract_frames(&mut self, arena: &SparsedTraceArena) {
        let resolved_arena = &arena.arena;

        if resolved_arena.nodes().is_empty() {
            return;
        }

        // Find the deepest failed node (where the actual revert happened)
        let mut current_idx = None;
        let mut max_depth = 0;

        for (idx, node) in resolved_arena.nodes().iter().enumerate() {
            if !node.trace.success && node.trace.depth >= max_depth {
                max_depth = node.trace.depth;
                current_idx = Some(idx);
            }
        }

        if current_idx.is_none() {
            return;
        }

        // Build the call stack by walking from the deepest node back to root
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
            } else if let Some(lib) =
                self.linked_libraries.iter().find(|l| l.address == contract_address)
            {
                frame = frame.with_contract_name(lib.name.clone());
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

impl fmt::Display for Backtrace<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.frames.is_empty() {
            return Ok(());
        }

        writeln!(f, "{}", Paint::yellow("Backtrace:"))?;

        for frame in &self.frames {
            write!(f, "  ")?;
            write!(f, "at ")?;
            writeln!(f, "{frame}")?;
        }

        Ok(())
    }
}

/// A single frame in a backtrace.
#[derive(Debug, Clone)]
struct BacktraceFrame {
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
    fn new(contract_address: Address) -> Self {
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
    fn with_contract_name(mut self, name: String) -> Self {
        self.contract_name = Some(name);
        self
    }

    /// Sets the function name.
    fn with_function_name(mut self, name: String) -> Self {
        self.function_name = Some(name);
        self
    }

    /// Sets the source location.
    fn with_source_location(mut self, file: PathBuf, line: usize, column: usize) -> Self {
        self.file = Some(file);
        self.line = Some(line);
        self.column = Some(column);
        self
    }

    /// Sets the byte offset.
    fn with_byte_offset(mut self, offset: usize) -> Self {
        self.byte_offset = Some(offset);
        self
    }
}

// Format: <CONTRACT_NAME>.<FUNCTION_NAME> (FILE:LINE:COL)
impl fmt::Display for BacktraceFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

        write!(f, "{result}")
    }
}
