use evm_disassembler::disassemble_bytes;
use foundry_common::fs;
use foundry_evm::coverage::CoverageReport;
use std::{
    collections::{hash_map, HashMap},
    path::PathBuf,
};

use super::CoverageReporter;

pub struct BytecodeReporter {
    root: PathBuf,
    destdir: PathBuf,
}

impl BytecodeReporter {
    pub fn new(root: PathBuf, destdir: PathBuf) -> BytecodeReporter {
        Self { root, destdir }
    }
}

impl CoverageReporter for BytecodeReporter {
    fn report(self, report: &CoverageReport) -> eyre::Result<()> {
        use std::fmt::Write;

        let no_source_elements = Vec::new();
        let mut line_number_cache = LineNumberCache::new(self.root.clone());

        for (contract_id, hits) in &report.bytecode_hits {
            let ops = disassemble_bytes(hits.bytecode.to_vec())?;
            let mut formatted = String::new();

            let source_elements =
                report.source_maps.get(contract_id).map(|sm| &sm.1).unwrap_or(&no_source_elements);

            for (code, source_element) in std::iter::zip(ops.iter(), source_elements) {
                let hits = hits
                    .hits
                    .get(&(code.offset as usize))
                    .map(|h| format!("[{:03}]", h))
                    .unwrap_or("     ".to_owned());
                let source_id = source_element.index;
                let source_path = source_id.and_then(|i| {
                    report.source_paths.get(&(contract_id.version.clone(), i as usize))
                });

                let code = format!("{:?}", code);
                let start = source_element.offset;
                let end = source_element.offset + source_element.length;

                if let Some(source_path) = source_path {
                    let (sline, spos) = line_number_cache.get_position(source_path, start)?;
                    let (eline, epos) = line_number_cache.get_position(source_path, end)?;
                    writeln!(
                        formatted,
                        "{} {:40} // {}: {}:{}-{}:{} ({}-{})",
                        hits, code, source_path, sline, spos, eline, epos, start, end
                    )?;
                } else if let Some(source_id) = source_id {
                    writeln!(
                        formatted,
                        "{} {:40} // SRCID{}: ({}-{})",
                        hits, code, source_id, start, end
                    )?;
                } else {
                    writeln!(formatted, "{} {:40}", hits, code)?;
                }
            }
            fs::write(
                &self.destdir.join(contract_id.contract_name.clone()).with_extension("asm"),
                formatted,
            )?;
        }

        Ok(())
    }
}

/// Cache line number offsets for source files
struct LineNumberCache {
    root: PathBuf,
    line_offsets: HashMap<String, Vec<usize>>,
}

impl LineNumberCache {
    pub fn new(root: PathBuf) -> Self {
        LineNumberCache { root, line_offsets: HashMap::new() }
    }

    pub fn get_position(&mut self, path: &str, offset: usize) -> eyre::Result<(usize, usize)> {
        let line_offsets = match self.line_offsets.entry(path.to_owned()) {
            hash_map::Entry::Occupied(o) => o.into_mut(),
            hash_map::Entry::Vacant(v) => {
                let text = fs::read_to_string(self.root.join(path))?;
                let mut line_offsets = vec![0];
                for line in text.lines() {
                    let line_offset = line.as_ptr() as usize - text.as_ptr() as usize;
                    line_offsets.push(line_offset);
                }
                v.insert(line_offsets)
            }
        };
        let lo = match line_offsets.binary_search(&offset) {
            Ok(lo) => lo,
            Err(lo) => lo - 1,
        };
        let pos = offset - line_offsets.get(lo).unwrap() + 1;
        Ok((lo, pos))
    }
}
