//! Source map decoding and PC mapping utilities.

use alloy_primitives::Bytes;
use foundry_compilers::artifacts::sourcemap::SourceMap;
use foundry_evm_core::ic::IcPcMap;

/// Maps program counters to source locations.
pub struct PcSourceMapper {
    /// Mapping from instruction counter to program counter.
    ic_pc_map: IcPcMap,
    /// The source map from Solidity compiler.
    source_map: SourceMap,
    /// Source files indexed by source ID.
    sources: Vec<(String, String)>, // (file_path, content)
    /// Cached line offset mappings for each source file.
    line_offsets: Vec<Vec<usize>>,
}

impl PcSourceMapper {
    /// Creates a new PC to source mapper.
    pub fn new(bytecode: &Bytes, source_map: SourceMap, sources: Vec<(String, String)>) -> Self {
        // Build instruction counter to program counter mapping
        let ic_pc_map = IcPcMap::new(bytecode.as_ref());

        // Pre-calculate line offsets for each source file
        let line_offsets =
            sources.iter().map(|(_, content)| compute_line_offsets(content)).collect();

        Self { ic_pc_map, source_map, sources, line_offsets }
    }

    /// Maps a program counter to source location.
    pub fn map_pc(&self, pc: usize) -> Option<SourceLocation> {
        // Find the instruction counter for this PC
        let ic = self.find_instruction_counter(pc)?;

        tracing::info!(pc = pc, ic = ic, map_entries = self.ic_pc_map.len(), "PC to IC mapping");

        // Get the source element for this instruction
        let element = self.source_map.get(ic)?;

        tracing::info!(
            ic = ic,
            source_map_len = self.source_map.len(),
            "Got source element for IC"
        );

        // Get the source file index
        let source_idx_opt = element.index();
        tracing::info!(
            source_idx = ?source_idx_opt,
            sources_count = self.sources.len(),
            "Checking source index"
        );

        let source_idx = source_idx_opt? as usize;
        if source_idx >= self.sources.len() {
            tracing::info!(
                source_idx = source_idx,
                sources_count = self.sources.len(),
                "Source index out of bounds"
            );
            return None;
        }

        // Get the source file info
        let (file_path, _content) = &self.sources[source_idx];

        // Convert byte offset to line and column
        let offset = element.offset() as usize;
        let (line, column) = self.offset_to_line_column(source_idx, offset)?;

        tracing::info!(
            file = file_path,
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
        let column = offset - line_start;

        // Lines and columns are 1-indexed in most editors
        Some((line + 1, column + 1))
    }
}

/// Represents a location in source code.
#[derive(Debug, Clone)]
pub struct SourceLocation {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub length: usize,
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::map::HashMap;

    #[test]
    fn test_line_offsets() {
        let content = "line1\nline2\nline3";
        let offsets = compute_line_offsets(content);
        assert_eq!(offsets, vec![0, 5, 11]);
    }

    #[test]
    fn test_offset_to_line_column() {
        let sources = vec![("test.sol".to_string(), "line1\nline2\nline3".to_string())];
        let mapper = PcSourceMapper {
            ic_pc_map: IcPcMap { inner: HashMap::default() },
            source_map: vec![],
            sources,
            line_offsets: vec![vec![0, 5, 11]],
        };

        // Test various offsets
        assert_eq!(mapper.offset_to_line_column(0, 0), Some((1, 1)));
        assert_eq!(mapper.offset_to_line_column(0, 4), Some((1, 5)));
        assert_eq!(mapper.offset_to_line_column(0, 6), Some((2, 1)));
        assert_eq!(mapper.offset_to_line_column(0, 12), Some((3, 1)));
    }
}
