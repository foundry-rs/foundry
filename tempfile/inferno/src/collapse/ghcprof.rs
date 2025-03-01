use std::io::{self, BufRead};

use log::warn;

use crate::collapse::common::Occurrences;
use crate::collapse::Collapse;

// These are the identifying words of the callgraph table, note that ticks and bytes columns are optional so not present
static START_LINE: &[&str] = &[
    "COST", "CENTRE", "MODULE", "SRC", "no.", "entries", "%time", "%alloc", "%time", "%alloc",
];

/// `ghcprof` folder configuration options.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Options {
    /// Column to source associated value from, default is `Source::PercentTime`.
    pub source: Source,
}

/// Which prof column to use as the cost centre of the output stacks
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub enum Source {
    #[default]
    /// The indivial %time column representing individual time as a percent of the total
    PercentTime,
    /// The ticks column representing individual runtime ticks
    Ticks,
    /// The bytes column representing individual bytes allocated
    Bytes,
}

/// A stack collapser for the output of `ghc`'s prof files.
///
/// To construct one, either use `ghcprof::Folder::default()` or create an [`Options`] and use
/// `ghcprof::Folder::from(options)`.
#[derive(Clone, Default)]
pub struct Folder {
    /// Cost for the current stack frame.
    current_cost: usize,

    /// Function on the stack in this entry thus far.
    stack: Vec<String>,

    opt: Options,
}

// The starting character offset of important columns
#[derive(Debug)]
struct Cols {
    cost_centre: usize,
    module: usize,
    source: usize,
}

impl Collapse for Folder {
    fn collapse<R, W>(&mut self, mut reader: R, writer: W) -> io::Result<()>
    where
        R: io::BufRead,
        W: io::Write,
    {
        // Consume the header...
        let mut line = Vec::new();
        let cols = loop {
            line.clear();
            if reader.read_until(b'\n', &mut line)? == 0 {
                warn!("File ended before start of call graph");
                return Ok(());
            };
            let l = String::from_utf8_lossy(&line);

            if l.split_whitespace()
                .take(START_LINE.len())
                .eq(START_LINE.iter().cloned())
            {
                let cost_centre = 0;
                let module = l.find("MODULE").unwrap_or(0);
                // Pick out these fixed columns, first two are individual only
                // "%time %alloc   %time %alloc"
                // `ticks` and `bytes` columns are optional and might appear on the end
                // ticks header is right aligned
                // bytes header is right aligned
                //   - BUT it has a max width of 9 whilst its values can exceed (but are always space separted)
                // "%time %alloc   %time %alloc  ticks  bytes"
                let source = match self.opt.source {
                    Source::PercentTime => l
                        .find("%time")
                        .expect("%time is present from matching START_LINE"),
                    // See note above about ticks and bytes columns
                    Source::Ticks => one_off_end_of_col_before(l.as_ref(), "ticks")?,
                    Source::Bytes => one_off_end_of_col_before(l.as_ref(), "bytes")?,
                };
                break Cols {
                    cost_centre,
                    module,
                    source,
                };
            }
        };
        // Skip one line
        reader.read_until(b'\n', &mut line)?;

        // Process the data...
        let mut occurrences = Occurrences::new(1);
        loop {
            line.clear();
            if reader.read_until(b'\n', &mut line)? == 0 {
                // The format is not expected to contain any blank lines within the callgraph
                break;
            }
            let l = String::from_utf8_lossy(&line);
            let line = l.trim_end();
            if line.is_empty() {
                break;
            } else {
                self.on_line(line, &mut occurrences, &cols)?;
            }
        }

        // Write the results...
        occurrences.write_and_clear(writer)?;

        // Reset the state...
        self.current_cost = 0;
        self.stack.clear();
        Ok(())
    }

    /// Check for start line of a call graph.
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

            if line
                .split_whitespace()
                .take(START_LINE.len())
                .eq(START_LINE.iter().cloned())
            {
                return Some(true);
            }
        }
        None
    }
}

fn one_off_end_of_col_before(line: &str, col: &str) -> io::Result<usize> {
    let col_start = match line.find(col) {
        Some(col_start) => col_start,
        _ => return invalid_data_error!("Expected '{col}' column but it was not present"),
    };
    let col_end = match line[..col_start].rfind(|c: char| !c.is_whitespace()) {
        Some(col_end) => col_end,
        _ => return invalid_data_error!("Expected a column before '{col}' but there was none"),
    };
    Ok(col_end + 1)
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
    // Handle call graph lines of the form:
    //
    // MAIN           MAIN ...
    //  CAF           Options.Applicative.Builder ...
    //   defaultPrefs Options.Applicative.Builder ...
    //    idm         Options.Applicative.Builder ...
    //    prefs       Options.Applicative.Builder ...
    //   fullDesc     Options.Applicative.Builder ...
    //   hidden       Options.Applicative.Builder ...
    //   option       Options.Applicative.Builder ...
    //    metavar     Options.Applicative.Builder ...
    //  CAF           Options.Applicative.Builder.Internal ...
    //   internal     Options.Applicative.Builder.Internal ...
    //   noGlobal     Options.Applicative.Builder.Internal ...
    //   optionMod    Options.Applicative.Builder.Internal ...

    fn on_line(
        &mut self,
        line: &str,
        occurrences: &mut Occurrences,
        cols: &Cols,
    ) -> io::Result<()> {
        if let Some(indent_chars) = line.find(|c| c != ' ') {
            let prev_len = self.stack.len();
            let depth = indent_chars;

            if depth < prev_len {
                // If the line is not a child, pop stack to the stack before the new depth
                self.stack.truncate(depth);
            } else if depth != prev_len {
                return invalid_data_error!("Skipped indentation level at line:\n{}", line);
            }
            // There can be non-ascii names so take care to char offset not byte offset
            let string_range = |col_start: usize| {
                line.chars()
                    .skip(col_start)
                    .skip_while(|c| c.is_whitespace())
                    // it is expected that the values to extract do not contain whitespace
                    // since this is used for functions/modules/costs where it is not allowed
                    .take_while(|c| !c.is_whitespace())
                    .collect::<String>()
            };
            let cost = string_range(cols.source);
            if let Ok(cost) = cost.trim().parse::<f64>() {
                let func = string_range(cols.cost_centre);
                let module = string_range(cols.module);
                // The columns we extract costs from all exclude the cost of their children
                self.current_cost = match self.opt.source {
                    // We must `insert_or_add` a `usize` so convert to per-mille to not lose the 1dp
                    Source::PercentTime => cost * 10.0,
                    Source::Ticks => cost,
                    Source::Bytes => cost,
                } as usize;
                self.stack
                    .push(format!("{}.{}", module.trim(), func.trim()));
                // identical stacks from other threads can appear so need to insert or add
                occurrences.insert_or_add(self.stack.join(";"), self.current_cost);
            } else {
                return invalid_data_error!("Invalid cost field: \"{}\"", cost);
            }
        }

        Ok(())
    }
}
