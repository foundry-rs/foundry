//! Source map decoding and PC mapping utilities.

use alloy_primitives::Bytes;
use foundry_compilers::{ProjectCompileOutput, artifacts::sourcemap::SourceMap};
use foundry_evm_core::ic::IcPcMap;
use std::path::{Path, PathBuf};

/// Source data for a single contract.
#[derive(Debug, Clone)]
pub struct SourceData {
    /// Runtime source map for the contract
    pub source_map: SourceMap,
    /// Deployed bytecode for accurate PC mapping
    pub bytecode: Bytes,
}

/// Maps program counters to source locations.
pub struct PcSourceMapper<'a> {
    /// Mapping from instruction counter to program counter.
    ic_pc_map: IcPcMap,
    /// Source data consists of the source_map and the deployed bytecode
    source_data: SourceData,
    /// Source files i.e source path and content (indexed by source_id)
    sources: &'a [(PathBuf, String)],
    /// Cached line offset mappings for each source file.
    line_offsets: Vec<Vec<usize>>,
}

impl<'a> PcSourceMapper<'a> {
    /// Creates a new PC to source mapper.
    pub fn new(source_data: SourceData, sources: &'a [(PathBuf, String)]) -> Self {
        // Build instruction counter to program counter mapping
        let ic_pc_map = IcPcMap::new(source_data.bytecode.as_ref());

        // Pre-calculate line offsets for each source file
        let line_offsets =
            sources.iter().map(|(_, content)| compute_line_offsets(content)).collect();

        Self { ic_pc_map, source_data, sources, line_offsets }
    }

    /// Maps a program counter to source location.
    pub fn map_pc(&self, pc: usize) -> Option<SourceLocation> {
        // Find the instruction counter for this PC
        let ic = self.find_instruction_counter(pc)?;

        // Get the source element for this instruction
        let element = self.source_data.source_map.get(ic)?;

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

        let line_offsets = self.line_offsets.get(source_idx)?;
        let (line, column) = offset_to_line_column(line_offsets, offset);

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

        for (ic, mapped_pc) in self.ic_pc_map.iter() {
            let mapped_pc = *mapped_pc as usize;
            if mapped_pc <= pc && mapped_pc >= best_pc {
                best_pc = mapped_pc;
                best_ic = Some(*ic as usize);
            }
        }

        best_ic
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

/// Computes the byte offset where each line starts in source content.
fn compute_line_offsets(content: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    offsets.extend(memchr::memchr_iter(b'\n', content.as_bytes()).map(|offset| offset + 1));
    offsets
}

/// Converts a byte offset to 1-indexed line and column numbers.
fn offset_to_line_column(line_offsets: &[usize], offset: usize) -> (usize, usize) {
    let line = line_offsets.partition_point(|&line_start| line_start <= offset) - 1;
    let column = offset - line_offsets[line];
    (line + 1, column + 1)
}

/// Loads sources for a specific ArtifactId.build_id
pub fn load_build_sources(
    build_id: &str,
    output: &ProjectCompileOutput,
    root: &Path,
) -> Option<Vec<(PathBuf, String)>> {
    let build_ctx = output.builds().find(|(bid, _)| *bid == build_id).map(|(_, ctx)| ctx)?;

    // Determine the size needed for sources vector
    // Highest source_id
    let max_source_id = build_ctx.source_id_to_path.keys().max().map_or(0, |id| *id) as usize;

    // Vec of source path and it's content
    let mut sources = vec![(PathBuf::new(), String::new()); max_source_id + 1];

    // Populate sources at their correct indices
    for (source_id, source_path) in &build_ctx.source_id_to_path {
        let idx = *source_id as usize;

        let full_path =
            if source_path.is_absolute() { source_path.clone() } else { root.join(source_path) };
        let mut source_content = foundry_common::fs::read_to_string(&full_path).unwrap_or_default();

        // Normalize line endings for windows
        if source_content.contains('\r') {
            source_content = source_content.replace("\r\n", "\n");
        }

        // Convert path to relative PathBuf
        let path_buf = source_path.strip_prefix(root).unwrap_or(source_path).to_path_buf();

        sources[idx] = (path_buf, source_content);
    }

    Some(sources)
}

#[cfg(test)]
mod tests {
    use super::{compute_line_offsets, offset_to_line_column};

    fn line_column(content: &str, offset: usize) -> (usize, usize) {
        offset_to_line_column(&compute_line_offsets(content), offset)
    }

    #[test]
    fn maps_byte_offsets_to_line_and_column() {
        assert_eq!(line_column("abc", 0), (1, 1));
        assert_eq!(line_column("abc", 2), (1, 3));

        assert_eq!(line_column("abc\ndef", 3), (1, 4));
        assert_eq!(line_column("abc\ndef", 4), (2, 1));
        assert_eq!(line_column("abc\ndef", 6), (2, 3));

        assert_eq!(line_column("a\nbc\ndef", 0), (1, 1));
        assert_eq!(line_column("a\nbc\ndef", 1), (1, 2));
        assert_eq!(line_column("a\nbc\ndef", 2), (2, 1));
        assert_eq!(line_column("a\nbc\ndef", 4), (2, 3));
        assert_eq!(line_column("a\nbc\ndef", 5), (3, 1));
        assert_eq!(line_column("a\nbc\ndef", 7), (3, 3));
    }
}
