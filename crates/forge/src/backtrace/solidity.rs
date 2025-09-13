//! Source mapping utilities for converting program counters to source locations.

use alloy_primitives::Bytes;
use foundry_compilers::artifacts::sourcemap::{SourceElement, SourceMap};
use revm::bytecode::opcode;
use std::collections::HashMap;

/// A source location in a Solidity file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    /// The source file path.
    pub file: String,
    /// The line number (1-indexed).
    pub line: usize,
    /// The column number (1-indexed).
    pub column: usize,
    /// The byte offset in the source file.
    pub offset: usize,
    /// The length of the source fragment.
    pub length: usize,
}

/// Maps program counters to source locations.
pub struct PcToSourceMapper {
    /// PC to source element mapping.
    pc_to_source: HashMap<usize, SourceElement>,
    /// Source file contents by file index.
    sources: HashMap<usize, SourceFileInfo>,
}

/// Information about a source file.
struct SourceFileInfo {
    /// The file path.
    path: String,
    /// Line start offsets for quick line/column calculation.
    line_offsets: Vec<usize>,
}

impl PcToSourceMapper {
    /// Creates a new PC-to-source mapper.
    pub fn new(
        bytecode: &Bytes,
        source_map: &SourceMap,
        sources: Vec<(String, String)>, // (path, content) pairs
    ) -> Self {
        let pc_to_source = Self::build_pc_map(bytecode, source_map);

        let mut source_files = HashMap::new();
        for (index, (path, content)) in sources.into_iter().enumerate() {
            let line_offsets = Self::compute_line_offsets(&content);
            source_files.insert(index, SourceFileInfo { path, line_offsets });
        }

        Self { pc_to_source, sources: source_files }
    }

    /// Builds a mapping from PC to source element.
    fn build_pc_map(bytecode: &Bytes, source_map: &SourceMap) -> HashMap<usize, SourceElement> {
        let mut map = HashMap::new();
        let mut pc = 0;
        let mut source_index = 0;

        for byte in bytecode {
            // Check if this PC has a source mapping
            if source_index < source_map.len() {
                if let Some(element) = source_map.get(source_index) {
                    map.insert(pc, element.clone());
                }
                source_index += 1;
            }

            // Handle multi-byte PUSH instructions
            if (opcode::PUSH1..=opcode::PUSH32).contains(byte) {
                let push_size = (*byte - opcode::PUSH1 + 1) as usize;
                pc += push_size;
            }

            pc += 1;
        }

        map
    }

    /// Maps a program counter to a source location.
    pub fn map_pc(&self, pc: usize) -> Option<SourceLocation> {
        let element = self.pc_to_source.get(&pc)?;
        let file_index = element.index()?;
        let source_file = self.sources.get(&(file_index as usize))?;

        let offset = element.offset();
        let length = element.length();
        let (line, column) = self.offset_to_line_column(source_file, offset as usize);

        Some(SourceLocation {
            file: source_file.path.clone(),
            line,
            column,
            offset: offset as usize,
            length: length as usize,
        })
    }

    /// Converts a byte offset to line and column numbers (1-indexed).
    fn offset_to_line_column(&self, source: &SourceFileInfo, offset: usize) -> (usize, usize) {
        // Binary search to find the line
        let line_index =
            source.line_offsets.binary_search(&offset).unwrap_or_else(|i| i.saturating_sub(1));

        let line = line_index + 1; // Convert to 1-indexed
        let line_start = source.line_offsets[line_index];
        let column = offset - line_start + 1; // Convert to 1-indexed

        (line, column)
    }

    /// Computes line start offsets for a source file.
    fn compute_line_offsets(content: &str) -> Vec<usize> {
        let mut offsets = vec![0];
        for (i, ch) in content.char_indices() {
            if ch == '\n' {
                offsets.push(i + 1);
            }
        }
        offsets
    }
}
