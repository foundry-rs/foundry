//! scan a file for unsafe usage

use crate::cmd::forge::geiger::error::ScanFileError;
use forge_fmt::{parse, Visitable, Visitor};
use foundry_common::fs;
use serde::{Deserialize, Serialize};
use solang_parser::{
    diagnostics::Diagnostic,
    pt::{Expression, Loc},
};
use std::{convert::Infallible, ops::Add, path::Path};

/// Scan a single file for `unsafe` usage.
pub fn find_cheatcodes_in_file(path: &Path) -> Result<SolFileMetrics, ScanFileError> {
    let content = fs::read_to_string(path)?;
    let cheatcodes = find_cheatcodes_in_string(&content)
        .map_err(|diagnostic| ScanFileError::ParseSol(diagnostic, path.to_path_buf()))?;
    Ok(SolFileMetrics { cheatcodes })
}

pub fn find_cheatcodes_in_string(src: &str) -> Result<CheatcodeCounter, Vec<Diagnostic>> {
    let mut parsed = parse(&src)?;
    let mut visitor = CheatcodeVisitor::default();
    let _ = parsed.pt.visit(&mut visitor);
    Ok(visitor.cheatcodes)
}

#[derive(Default)]
struct CheatcodeVisitor {
    cheatcodes: CheatcodeCounter,
}

impl Visitor for CheatcodeVisitor {
    type Error = Infallible;

    fn visit_expr(&mut self, loc: Loc, expr: &mut Expression) -> Result<(), Self::Error> {
        if let Expression::FunctionCall(loc, lhs, rhs) = expr {
            dbg!(&lhs);
        }
        Ok(())
    }
}

/// Scan result for a single `.sol` file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SolFileMetrics {
    /// Cheatcode metrics.
    pub cheatcodes: CheatcodeCounter,
}

/// Unsafe usage metrics collection.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CheatcodeCounter {
    pub ffi: usize,
    pub read_file: usize,
    pub read_line: usize,
    pub write_file: usize,
    pub write_line: usize,
    pub remove_file: usize,
}

impl CheatcodeCounter {
    pub fn has_unsafe(&self) -> bool {
        self.ffi > 0 ||
            self.read_file > 0 ||
            self.read_line > 0 ||
            self.write_file > 0 ||
            self.write_line > 0 ||
            self.remove_file > 0
    }
}

impl Add for CheatcodeCounter {
    type Output = CheatcodeCounter;

    fn add(self, other: CheatcodeCounter) -> CheatcodeCounter {
        CheatcodeCounter {
            ffi: self.ffi + other.ffi,
            read_file: self.read_file + other.read_file,
            read_line: self.read_line + other.read_line,
            write_file: self.write_file + other.write_file,
            write_line: self.write_line + other.write_line,
            remove_file: self.remove_file + other.remove_file,
        }
    }
}
