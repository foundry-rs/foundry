use std::io::{self, BufRead};

use log::warn;

use crate::collapse::common::Occurrences;
use crate::collapse::Collapse;

// The call graph begins after this line.
static HEADER: &str = "Function Stack,CPU Time:Self,Module";

/// `vtune` folder configuration options.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Options {
    /// Don't include modules with function names.
    ///
    /// Default is `false`.
    pub no_modules: bool,
}

/// A stack collapser for CSV call graphs created with the VTune `amplxe-cl` tool.
///
/// To construct one, either use `vtune::Folder::default()` or create an [`Options`] and use
/// `vtune::Folder::from(options)`.
#[derive(Clone, Default)]
pub struct Folder {
    /// Function on the stack in this entry thus far.
    stack: Vec<String>,

    opt: Options,
}

impl Collapse for Folder {
    fn collapse<R, W>(&mut self, mut reader: R, writer: W) -> io::Result<()>
    where
        R: io::BufRead,
        W: io::Write,
    {
        // Consume the header...
        let mut line = Vec::new();
        loop {
            line.clear();
            if reader.read_until(0x0A, &mut line)? == 0 {
                warn!("File ended before header");
                return Ok(());
            };
            let l = String::from_utf8_lossy(&line);
            if l.starts_with(HEADER) {
                break;
            }
        }

        // Process the data...
        let mut occurrences = Occurrences::new(1);
        loop {
            line.clear();
            if reader.read_until(0x0A, &mut line)? == 0 {
                break;
            }
            let l = String::from_utf8_lossy(&line);
            let line = l.trim_end();
            if line.is_empty() {
                continue;
            } else {
                self.on_line(line, &mut occurrences)?;
            }
        }

        // Write the results...
        occurrences.write_and_clear(writer)?;

        // Reset the state...
        self.stack.clear();
        Ok(())
    }

    /// Check for header
    fn is_applicable(&mut self, input: &str) -> Option<bool> {
        let mut input = input.as_bytes();
        let mut line = String::new();
        loop {
            line.clear();
            if let Ok(n) = input.read_line(&mut line) {
                if n == 0 {
                    break;
                }
            } else {
                return Some(false);
            }

            if line.starts_with(HEADER) {
                return Some(true);
            }
        }
        None
    }
}

impl From<Options> for Folder {
    fn from(opt: Options) -> Self {
        Folder {
            opt,
            ..Default::default()
        }
    }
}

impl Folder {
    fn line_parts<'a>(&self, line: &'a str) -> Option<(&'a str, &'a str, &'a str)> {
        let mut line = if let Some(line) = line.strip_prefix('"') {
            // The function name will be in quotes if it contains spaces.
            line.splitn(2, "\",")
        } else {
            // We split on a string because we need to match the type of the other if branch.
            #[allow(clippy::single_char_pattern)]
            line.splitn(2, ",")
        };

        let func = line.next()?;
        let mut line = line.next()?.splitn(2, ',');
        let time = line.next()?;
        let module = if self.opt.no_modules {
            ""
        } else {
            line.next()?
        };

        Some((func, time, module))
    }

    fn on_line(&mut self, line: &str, occurrences: &mut Occurrences) -> io::Result<()> {
        if let Some(spaces) = line.find(|c| c != ' ') {
            let prev_depth = self.stack.len();
            let depth = spaces + 1;

            if depth <= prev_depth {
                // If the depth of this line is less than the previous one,
                // it means the previous line was a leaf node and we should
                // pop the stack back to one before the current depth.
                for _ in 0..=prev_depth - depth {
                    self.stack.pop();
                }
            } else if depth > prev_depth + 1 {
                return invalid_data_error!("Skipped indentation level at line:\n{}", line);
            }

            if let Some((func, time, module)) = self.line_parts(&line[spaces..]) {
                if let Ok(time) = time.parse::<f64>() {
                    let time_ms = (time * 1000.0).round() as usize;
                    if module.is_empty() {
                        self.stack.push(func.to_string());
                    } else {
                        self.stack.push(format!("{}`{}", module, func));
                    }
                    if time_ms > 0 {
                        self.write_stack(occurrences, time_ms);
                    }
                } else {
                    return invalid_data_error!("Invalid `CPU Time:Self` field: {}", time);
                }
            } else {
                return invalid_data_error!("Unable to parse stack line:\n{}", line);
            }
        }

        Ok(())
    }

    fn write_stack(&self, occurrences: &mut Occurrences, time: usize) {
        occurrences.insert(self.stack.join(";"), time);
    }
}
