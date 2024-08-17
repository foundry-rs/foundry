use super::{error::ScanFileError, visitor::CheatcodeVisitor};
use eyre::Result;
use forge_fmt::{offset_to_line_column, parse2, FormatterError, Visitable};
use foundry_common::fs;
use solang_parser::pt::Loc;
use std::{
    fmt,
    path::{Path, PathBuf},
};
use yansi::Paint;

/// Scan a single file for `unsafe` cheatcode usage.
pub fn find_cheatcodes_in_file(path: &Path) -> Result<SolFileMetrics, ScanFileError> {
    let contents = fs::read_to_string(path)?;
    let cheatcodes = find_cheatcodes_in_string(&contents, Some(path))?;
    Ok(SolFileMetrics { contents, cheatcodes, file: path.to_path_buf() })
}

/// Scan a string for unsafe cheatcodes.
pub fn find_cheatcodes_in_string(
    src: &str,
    path: Option<&Path>,
) -> Result<UnsafeCheatcodes, FormatterError> {
    let mut parsed = parse2(src, path)?;
    let mut visitor = CheatcodeVisitor::default();
    parsed.pt.visit(&mut visitor).unwrap();
    Ok(visitor.cheatcodes)
}

/// Scan result for a single Solidity file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SolFileMetrics {
    /// The Solidity file
    pub file: PathBuf,

    /// The file's contents.
    pub contents: String,

    /// The unsafe cheatcodes found.
    pub cheatcodes: UnsafeCheatcodes,
}

/// Formats the metrics for a single file using [`fmt::Display`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SolFileMetricsPrinter<'a, 'b> {
    pub metrics: &'a SolFileMetrics,
    pub root: &'b Path,
}

impl<'a, 'b> fmt::Display for SolFileMetricsPrinter<'a, 'b> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let SolFileMetricsPrinter { metrics, root } = *self;

        let file = metrics.file.strip_prefix(root).unwrap_or(&metrics.file);

        macro_rules! print_unsafe_fn {
            ($($name:literal => $field:ident),*) => {$(
                let $field = &metrics.cheatcodes.$field[..];
                if !$field.is_empty() {
                    writeln!(f, "  {}  {}", metrics.cheatcodes.$field.len().red(), $name.red())?;

                    for &loc in $field {
                        let content = &metrics.contents[loc.range()];
                        let (line, col) = offset_to_line_column(&metrics.contents, loc.start());
                        let pos = format!("  --> {}:{}:{}", file.display(), line, col);
                        writeln!(f,"{}", pos.red())?;
                        for line in content.lines() {
                            writeln!(f, "      {}", line.red())?;
                        }
                    }
                }
               )*};
        }

        if !metrics.cheatcodes.is_empty() {
            writeln!(f, "{}    {}", metrics.cheatcodes.len().red(), file.display().red())?;
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnsafeCheatcodes {
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

impl UnsafeCheatcodes {
    /// Whether there are any unsafe calls.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The total number of unsafe calls.
    pub fn len(&self) -> usize {
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
        let s = r"
        contract A is Test {
            function do_ffi() public {
                string[] memory inputs = new string[](1);
                vm.ffi(inputs);
            }
        }
        ";

        let count = find_cheatcodes_in_string(s, None).unwrap();
        assert_eq!(count.ffi.len(), 1);
        assert!(!count.is_empty());
    }

    #[test]
    fn can_find_call_in_assignment() {
        let s = r"
        contract A is Test {
            function do_ffi() public {
                string[] memory inputs = new string[](1);
                bytes stuff = vm.ffi(inputs);
            }
        }
        ";

        let count = find_cheatcodes_in_string(s, None).unwrap();
        assert_eq!(count.ffi.len(), 1);
        assert!(!count.is_empty());
    }
}
