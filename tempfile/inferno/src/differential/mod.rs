use std::fs::File;
use std::io::{self, prelude::*};
use std::path::Path;

use ahash::AHashMap;
use log::warn;

const READER_CAPACITY: usize = 128 * 1024;

#[derive(Debug, Clone, Copy, Default)]
struct Counts {
    first: usize,
    second: usize,
}

/// Configure the generated output.
///
/// All options default to off.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Normalize the first profile count to match the second.
    ///
    /// This can help in scenarios where you take profiles at different times, under varying
    /// load. If you generate a differential flame graph without setting this flag, everything
    /// will look red if the load increased, or blue if it decreased. If this flag is set,
    /// the first profile is balanced so you get the full red/blue spectrum.
    pub normalize: bool,

    /// Strip hex numbers (addresses) of the form "0x45ef2173" and replace with "0x...".
    pub strip_hex: bool,
}

/// Produce an output that can be used to generate a differential flame graph.
///
/// The readers are expected to contain folded stack lines of before and after profiles with
/// the following whitespace-separated fields:
///
///  - A semicolon-separated list of frame names (e.g., `main;foo;bar;baz`).
///  - A sample count for the given stack.
///
/// The output written to the `writer` will be similar to the inputs, except there will be two
/// sample count columns -- one for each profile.
pub fn from_readers<R1, R2, W>(opt: Options, before: R1, after: R2, writer: W) -> io::Result<()>
where
    R1: BufRead,
    R2: BufRead,
    W: Write,
{
    let mut stack_counts = AHashMap::default();
    let total1 = parse_stack_counts(opt, &mut stack_counts, before, true)?;
    let total2 = parse_stack_counts(opt, &mut stack_counts, after, false)?;
    if opt.normalize && total1 != total2 {
        for counts in stack_counts.values_mut() {
            counts.first = (counts.first as f64 * total2 as f64 / total1 as f64) as usize;
        }
    }
    write_stacks(&stack_counts, writer)
}

/// Produce an output that can be used to generate a differential flame graph from
/// a before and an after profile.
///
/// See [`from_readers`] for the input and output formats.
pub fn from_files<P1, P2, W>(
    opt: Options,
    file_before: P1,
    file_after: P2,
    writer: W,
) -> io::Result<()>
where
    P1: AsRef<Path>,
    P2: AsRef<Path>,
    W: Write,
{
    let file1 = File::open(file_before)?;
    let reader1 = io::BufReader::with_capacity(READER_CAPACITY, file1);
    let file2 = File::open(file_after)?;
    let reader2 = io::BufReader::with_capacity(READER_CAPACITY, file2);
    from_readers(opt, reader1, reader2, writer)
}

// Populate stack_counts based on lines from the reader and returns the sum of the sample counts.
fn parse_stack_counts<R>(
    opt: Options,
    stack_counts: &mut AHashMap<String, Counts>,
    mut reader: R,
    is_first: bool,
) -> io::Result<usize>
where
    R: BufRead,
{
    let mut total = 0;
    let mut line = Vec::new();
    let mut stripped_fractional_samples = false;
    loop {
        line.clear();

        if reader.read_until(0x0A, &mut line)? == 0 {
            break;
        }

        let l = String::from_utf8_lossy(&line);
        if let Some((stack, count)) =
            parse_line(&l, opt.strip_hex, &mut stripped_fractional_samples)
        {
            let counts = stack_counts.entry(stack).or_default();
            if is_first {
                counts.first += count;
            } else {
                counts.second += count;
            }
            total += count;
        } else {
            warn!("Unable to parse line: {}", l);
        }
    }

    Ok(total)
}

// Write three-column lines with the folded stack trace and two value columns,
// one for each profile.
fn write_stacks<W>(stack_counts: &AHashMap<String, Counts>, mut writer: W) -> io::Result<()>
where
    W: Write,
{
    for (stack, &Counts { first, second }) in stack_counts {
        writeln!(writer, "{} {} {}", stack, first, second)?;
    }
    Ok(())
}

// Parse stack and sample count from line.
fn parse_line(
    line: &str,
    strip_hex: bool,
    stripped_fractional_samples: &mut bool,
) -> Option<(String, usize)> {
    let samplesi = line.rfind(' ')?;
    let mut samples = line[samplesi + 1..].trim_end();

    // Strip fractional part (if any);
    // foobar 1.klwdjlakdj
    //
    // The Perl version keeps the fractional part but inferno
    // strips them in its flamegraph implementation anyway.
    if let Some(doti) = samples.find('.') {
        if !samples[..doti]
            .chars()
            .chain(samples[doti + 1..].chars())
            .all(|c| c.is_ascii_digit())
        {
            return None;
        }
        // Warn if we're stripping a non-zero fractional part, but only the first time.
        if !*stripped_fractional_samples && !samples[doti + 1..].chars().all(|c| c == '0') {
            *stripped_fractional_samples = true;
            warn!("The input data has fractional sample counts that will be truncated to integers");
        }
        samples = &samples[..doti];
    }

    let nsamples = samples.parse::<usize>().ok()?;
    let stack = line[..samplesi].trim_end();
    if strip_hex {
        Some((strip_hex_address(stack), nsamples))
    } else {
        Some((stack.to_string(), nsamples))
    }
}

// Replace all hex strings like "0x45ef2173" with "0x...".
fn strip_hex_address(mut stack: &str) -> String {
    let mut stripped = String::with_capacity(stack.len());
    while let Some(idx) = stack.find("0x") {
        stripped.push_str(&stack[..idx + 2]);
        let ndigits = stack[idx + 2..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .count();
        if ndigits > 0 {
            stripped.push_str("...");
        }
        stack = &stack[idx + 2 + ndigits..];
    }
    stripped.push_str(stack);
    stripped
}
