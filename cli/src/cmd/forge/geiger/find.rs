//! scan a file for unsafe usage

use crate::cmd::forge::geiger::{error::ScanFileError, visitor::CheatcodeVisitor};
use forge_fmt::{offset_to_line_column, parse, Visitable};
use foundry_common::fs;
use solang_parser::{diagnostics::Diagnostic, pt::Loc};
use std::{
    fmt,
    path::{Path, PathBuf},
};
use yansi::Paint;

/// Scan a single file for `unsafe` usage.
pub fn find_cheatcodes_in_file(path: &Path) -> Result<SolFileMetrics, ScanFileError> {
    let content = fs::read_to_string(path)?;
    let cheatcodes = find_cheatcodes_in_string(&content)
        .map_err(|diagnostic| ScanFileError::ParseSol(diagnostic, path.to_path_buf()))?;
    Ok(SolFileMetrics { content, cheatcodes, file: path.to_path_buf() })
}

pub fn find_cheatcodes_in_string(src: &str) -> Result<CheatcodeCounter, Vec<Diagnostic>> {
    let mut parsed = parse(src)?;
    let mut visitor = CheatcodeVisitor::default();
    let _ = parsed.pt.visit(&mut visitor);
    Ok(visitor.cheatcodes)
}

/// Scan result for a single `.sol` file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SolFileMetrics {
    /// The sol file
    pub file: PathBuf,
    /// The file's content
    pub content: String,
    /// Cheatcode metrics.
    pub cheatcodes: CheatcodeCounter,
}

pub struct SolFileMetricsPrinter<'a, 'b> {
    pub metrics: &'a SolFileMetrics,
    pub root: &'b Path,
}

impl<'a, 'b> fmt::Display for SolFileMetricsPrinter<'a, 'b> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let SolFileMetricsPrinter { metrics, root } = self;

        let file = metrics.file.strip_prefix(root).unwrap_or(&metrics.file);

        macro_rules! print_unsafe_fn {
            ($($name:literal => $field:ident),*) => {
               $ (
                    if !metrics.cheatcodes.$field.is_empty() {
                        writeln!(f, "  {}  {}", Paint::red(metrics.cheatcodes.$field.len()), Paint::red($name))?;

                        let mut iter = metrics.cheatcodes.$field.iter().peekable();
                        while let Some(loc) = iter.next() {
                            let function_call = &metrics.content.as_bytes()[loc.start().. loc.end()];
                            let (line, col) = offset_to_line_column(&metrics.content, loc.start(), loc.end());
                            let pos = format!("  --> {}:{}:{}", file.display(),  line, col);
                            writeln!(f,"{}", Paint::red(pos))?;
                            let content = String::from_utf8_lossy(function_call);
                            let mut lines = content.lines().peekable();
                            while let Some(line) = lines.next() {
                                writeln!(f, "      {}", Paint::red(line))?;
                            }
                        }
                    }
               )*

            };
        }

        if metrics.cheatcodes.has_unsafe() {
            writeln!(
                f,
                "{}    {}",
                Paint::red(metrics.cheatcodes.count()),
                Paint::red(file.display())
            )?;
            print_unsafe_fn!(
                "ffi" => ffi,
                "readFile" => read_file,
                "readLine" => read_line,
                "writeFile" => write_file,
                "writeLine" => write_line,
                "removeFile" => remove_file,
                "closeFile" => close_file,
                "setEnv" => set_env,
                "deriveKey" => derive_key
            );
        } else {
            writeln!(f, "0    {}", file.display())?
        }

        Ok(())
    }
}

/// Unsafe usage metrics collection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CheatcodeCounter {
    pub ffi: Vec<Loc>,
    pub read_file: Vec<Loc>,
    pub read_line: Vec<Loc>,
    pub write_file: Vec<Loc>,
    pub write_line: Vec<Loc>,
    pub remove_file: Vec<Loc>,
    pub close_file: Vec<Loc>,
    pub set_env: Vec<Loc>,
    pub derive_key: Vec<Loc>,
}

impl CheatcodeCounter {
    pub fn has_unsafe(&self) -> bool {
        !self.ffi.is_empty() ||
            !self.read_file.is_empty() ||
            !self.read_line.is_empty() ||
            !self.write_file.is_empty() ||
            !self.write_line.is_empty() ||
            !self.close_file.is_empty() ||
            !self.set_env.is_empty() ||
            !self.derive_key.is_empty() ||
            !self.remove_file.is_empty()
    }

    pub fn count(&self) -> usize {
        self.ffi.len() +
            self.read_file.len() +
            self.read_line.len() +
            self.write_file.len() +
            self.write_line.len() +
            self.close_file.len() +
            self.set_env.len() +
            self.derive_key.len() +
            self.remove_file.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_find_calls() {
        let s = r#"
        contract A is Test {
            function do_ffi() public {
                string[] memory inputs = new string[](1);
                vm.ffi(inputs);
            }
        }
        "#;

        let count = find_cheatcodes_in_string(s).unwrap();
        assert_eq!(count.ffi.len(), 1);
        assert!(count.has_unsafe());
    }

    #[test]
    fn can_find_call_in_assignment() {
        let s = r#"
        contract A is Test {
            function do_ffi() public {
                string[] memory inputs = new string[](1);
                bytes stuff = vm.ffi(inputs);
            }
        }
        "#;

        let count = find_cheatcodes_in_string(s).unwrap();
        assert_eq!(count.ffi.len(), 1);
        assert!(count.has_unsafe());
    }
}
