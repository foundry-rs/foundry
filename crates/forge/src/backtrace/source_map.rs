//! Source map decoding and PC mapping utilities.

use alloy_primitives::Bytes;
use foundry_compilers::{
    Artifact, ProjectCompileOutput,
    artifacts::{ConfigurableContractArtifact, sourcemap::SourceMap},
};
use foundry_evm_core::ic::IcPcMap;
use std::path::{Path, PathBuf};

/// Source data for a single artifact/contract.
/// Contains all the data needed to generate backtraces for a contract.
#[derive(Debug, Clone)]
pub struct SourceData {
    /// Runtime source map for the contract
    pub source_map: SourceMap,
    /// Source files (path, content) indexed by source ID
    pub sources: Vec<(PathBuf, String)>,
    /// Deployed bytecode for accurate PC mapping
    pub bytecode: Bytes,
}

/// Maps program counters to source locations.
pub struct PcSourceMapper {
    /// Mapping from instruction counter to program counter.
    ic_pc_map: IcPcMap,
    /// The source map from Solidity compiler.
    source_map: SourceMap,
    /// Source files indexed by source ID.
    sources: Vec<(PathBuf, String)>, // (file_path, content)
    /// Cached line offset mappings for each source file.
    line_offsets: Vec<Vec<usize>>,
}

impl PcSourceMapper {
    /// Creates a new PC to source mapper.
    pub fn new(source_data: SourceData) -> Self {
        // Build instruction counter to program counter mapping
        let ic_pc_map = IcPcMap::new(source_data.bytecode.as_ref());

        // Pre-calculate line offsets for each source file
        let line_offsets =
            source_data.sources.iter().map(|(_, content)| compute_line_offsets(content)).collect();

        Self {
            ic_pc_map,
            source_map: source_data.source_map,
            sources: source_data.sources,
            line_offsets,
        }
    }

    /// Maps a program counter to source location.
    pub fn map_pc(&self, pc: usize) -> Option<SourceLocation> {
        // Find the instruction counter for this PC
        let ic = self.find_instruction_counter(pc)?;

        // Get the source element for this instruction
        let element = self.source_map.get(ic)?;

        // Get the source file index - returns None if index is -1
        let source_idx_opt = element.index();

        let source_idx = source_idx_opt? as usize;
        if source_idx >= self.sources.len() {
            return None;
        }

        // Get the source file info
        let (file_path, content) = &self.sources[source_idx];

        // Convert byte offset to line and column
        let offset = element.offset() as usize;

        // Check if offset is valid for this source file
        if offset >= content.len() {
            return None;
        }

        let (line, column) = self.offset_to_line_column(source_idx, offset)?;

        trace!(
            file = ?file_path,
            line = line,
            column = column,
            offset = offset,
            "Mapped PC to source location"
        );

        Some(SourceLocation {
            file: file_path.clone(),
            line,
            column,
            length: element.length() as usize,
            offset,
        })
    }

    /// Finds the instruction counter for a given program counter.
    fn find_instruction_counter(&self, pc: usize) -> Option<usize> {
        // The IcPcMap maps IC -> PC, we need the reverse
        // We find the highest IC that has a PC <= our target PC
        let mut best_ic = None;
        let mut best_pc = 0;

        for (ic, mapped_pc) in &self.ic_pc_map.inner {
            let mapped_pc = *mapped_pc as usize;
            if mapped_pc <= pc && mapped_pc >= best_pc {
                best_pc = mapped_pc;
                best_ic = Some(*ic as usize);
            }
        }

        best_ic
    }

    /// Converts a byte offset to line and column numbers.
    fn offset_to_line_column(&self, source_idx: usize, offset: usize) -> Option<(usize, usize)> {
        let line_offsets = self.line_offsets.get(source_idx)?;

        // Find the line containing this offset
        let line = line_offsets.binary_search(&offset).unwrap_or_else(|i| i.saturating_sub(1));

        // Calculate column within the line
        let line_start = if line == 0 { 0 } else { line_offsets[line - 1] + 1 };
        let column = offset.saturating_sub(line_start);

        // Lines and columns are 1-indexed in most editors
        Some((line + 1, column + 1))
    }
}
/// Represents a location in source code.
#[derive(Debug, Clone)]
pub struct SourceLocation {
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
    pub length: usize,
    /// Byte offset in the source file
    /// This specifically useful when one source file contains multiple contracts / libraries.
    pub offset: usize,
}

/// Computes line offset positions in source content.
fn compute_line_offsets(content: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (idx, ch) in content.char_indices() {
        if ch == '\n' {
            offsets.push(idx);
        }
    }
    offsets
}

/// Collects source data for a single artifact.
pub fn collect_source_data(
    artifact: &ConfigurableContractArtifact,
    output: &ProjectCompileOutput,
    root: &Path,
    build_id: &str,
) -> Option<SourceData> {
    // Get source map and bytecode
    let source_map = artifact.get_source_map_deployed()?.ok()?;
    let bytecode = artifact.get_deployed_bytecode_bytes()?.into_owned();

    if bytecode.is_empty() {
        return None;
    }

    // Get sources for this build
    let mut sources = Vec::new();

    // Get the build context for this build_id
    if let Some(build_context) =
        output.builds().find(|(bid, _)| *bid == build_id).map(|(_, ctx)| ctx)
    {
        // Build ordered sources from the build context
        let mut ordered_sources = Vec::with_capacity(build_context.source_id_to_path.len());

        for (source_id, source_path) in &build_context.source_id_to_path {
            // Read source content from file
            let full_path = if source_path.is_absolute() {
                source_path.clone()
            } else {
                root.join(source_path)
            };

            let mut source_content =
                foundry_common::fs::read_to_string(&full_path).unwrap_or_default();
            // Normalize line endings for windows
            if source_content.contains('\r') {
                source_content = source_content.replace("\r\n", "\n");
            }

            // Convert path to relative PathBuf
            let path_buf = source_path.strip_prefix(root).unwrap_or(source_path).to_path_buf();

            ordered_sources.push((*source_id, path_buf, source_content));
        }

        // Sort by source ID to ensure proper ordering
        ordered_sources.sort_by_key(|(id, _, _)| *id);

        // Build the final sources vector in the correct order
        for (id, path_buf, content) in ordered_sources {
            let idx = id as usize;
            if sources.len() <= idx {
                sources.resize(idx + 1, (PathBuf::new(), String::new()));
            }
            sources[idx] = (path_buf, content);
        }
    }

    Some(SourceData { source_map, sources, bytecode })
}
