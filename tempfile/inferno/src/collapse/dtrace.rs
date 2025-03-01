use std::borrow::Cow;
use std::collections::VecDeque;
use std::io::{self, prelude::*};

use log::warn;

use crate::collapse::common::{self, CollapsePrivate, Occurrences};

/// `dtrace` folder configuration options.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Options {
    /// Include function offset (except leafs).
    ///
    /// Default is `false`.
    pub includeoffset: bool,

    /// The number of threads to use.
    ///
    /// Default is the number of logical cores on your machine.
    pub nthreads: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            includeoffset: false,
            nthreads: *common::DEFAULT_NTHREADS,
        }
    }
}

/// A stack collapser for the output of dtrace `ustrace()`.
///
/// To construct one, either use `dtrace::Folder::default()` or create an [`Options`] and use
/// `dtrace::Folder::from(options)`.
pub struct Folder {
    /// Vector for processing java stuff
    cache_inlines: Vec<String>,

    /// The number of stacks per job to send to the threadpool.
    nstacks_per_job: usize,

    /// Function entries on the stack in this entry thus far.
    stack: VecDeque<String>,

    /// Keep track of stack string size while we consume a stack
    stack_str_size: usize,

    opt: Options,
}

impl From<Options> for Folder {
    fn from(mut opt: Options) -> Self {
        if opt.nthreads == 0 {
            opt.nthreads = 1;
        }
        Self {
            cache_inlines: Vec::new(),
            nstacks_per_job: common::DEFAULT_NSTACKS_PER_JOB,
            stack: VecDeque::default(),
            stack_str_size: 0,
            opt,
        }
    }
}

impl Default for Folder {
    fn default() -> Self {
        Options::default().into()
    }
}

impl CollapsePrivate for Folder {
    fn pre_process<R>(&mut self, reader: &mut R, _: &mut Occurrences) -> io::Result<()>
    where
        R: io::BufRead,
    {
        // Consumer the header...
        let mut line = Vec::new();
        loop {
            line.clear();
            if reader.read_until(0x0A, &mut line)? == 0 {
                // We reached the end :( this should not happen.
                warn!("File ended while skipping headers");
                return Ok(());
            };
            if String::from_utf8_lossy(&line).trim().is_empty() {
                return Ok(());
            }
        }
    }

    fn collapse_single_threaded<R>(
        &mut self,
        mut reader: R,
        occurrences: &mut Occurrences,
    ) -> io::Result<()>
    where
        R: io::BufRead,
    {
        let mut line = Vec::new();
        loop {
            line.clear();
            if reader.read_until(0x0A, &mut line)? == 0 {
                break;
            }
            let s = String::from_utf8_lossy(&line);
            let line = s.trim();
            if line.is_empty() {
                continue;
            } else if let Ok(count) = line.parse::<usize>() {
                self.on_stack_end(count, occurrences);
            } else {
                self.on_stack_line(line);
            }
        }
        // If we reach this point in the code and there's still something in our
        // state (`self.stack` and `self.stack_str_size`), it means the input
        // did not terminate at the end of a stack; rather, it terminated in
        // the middle of a stack. In this case, we consider the input data
        // invalid and return an io::Error to the user.
        if !self.stack.is_empty() || self.stack_str_size != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Input data ends in the middle of a stack.",
            ));
        }
        Ok(())
    }

    fn is_applicable(&mut self, input: &str) -> Option<bool> {
        let mut found_empty_line = false;
        let mut found_stack_line = false;
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

            let line = line.trim();
            if line.is_empty() {
                found_empty_line = true;
            } else if found_empty_line {
                if line.parse::<usize>().is_ok() {
                    return Some(found_stack_line);
                } else if line.contains('`')
                    || (line.starts_with("0x") && usize::from_str_radix(&line[2..], 16).is_ok())
                {
                    found_stack_line = true;
                } else {
                    // This is not a stack or count line
                    return Some(false);
                }
            }
        }
        None
    }

    // This method should do the same thing as:
    // ```
    // fn is_end_of_stack(&self, line: &[u8]) -> bool {
    //     match std::str::from_utf8(line) {
    //         Ok(line) => {
    //             let line = line.trim();
    //             match line.parse::<usize>() {
    //                 Ok(_) => true,
    //                 Err(_) => false,
    //             }
    //         }
    //         Err(_) => false,
    //     }
    // }
    // ```
    // But it is much faster since it works directly on bytes and because all we're interested in is
    // whether the provided bytes **can** be parsed into a `usize`, not which `usize` they parse into.
    // Also, we don't need to validate that the input is utf8.
    //
    // Benchmarking results for the two methods:
    // * Using the method above: 281 MiB/s
    // * Using the method below: 437 MiB/s
    //
    fn would_end_stack(&mut self, line: &[u8]) -> bool {
        // In order to return `true`, as we iterate over the provided bytes, we need to progress
        // through each of the follow states, in order; if we can't, immediately return `false`.
        #[allow(clippy::enum_variant_names)]
        enum State {
            StartOfLine,  // Accept any number of whitespace characters
            MiddleOfLine, // Accept any number of ascii digits
            EndOfLine,    // Accept any number of whitespace characters
        }
        let mut state = State::StartOfLine;
        for b in line {
            let c = *b as char;
            match state {
                State::StartOfLine => {
                    if c.is_whitespace() {
                        continue;
                    } else if c.is_ascii_digit() {
                        state = State::MiddleOfLine;
                    } else {
                        return false;
                    }
                }
                State::MiddleOfLine => {
                    if c.is_ascii_digit() {
                        continue;
                    } else if c.is_whitespace() {
                        state = State::EndOfLine;
                    } else {
                        return false;
                    }
                }
                State::EndOfLine => {
                    if c.is_whitespace() {
                        continue;
                    } else {
                        return false;
                    }
                }
            }
        }
        matches!(state, State::EndOfLine)
    }

    fn clone_and_reset_stack_context(&self) -> Self {
        Self {
            cache_inlines: self.cache_inlines.clone(),
            nstacks_per_job: self.nstacks_per_job,
            stack: VecDeque::default(),
            stack_str_size: 0,
            opt: self.opt.clone(),
        }
    }

    fn nstacks_per_job(&self) -> usize {
        self.nstacks_per_job
    }

    fn set_nstacks_per_job(&mut self, n: usize) {
        self.nstacks_per_job = n;
    }

    fn nthreads(&self) -> usize {
        self.opt.nthreads
    }

    fn set_nthreads(&mut self, n: usize) {
        self.opt.nthreads = n;
    }
}

impl Folder {
    // This function approximates the Perl regex s/(::.*)[(<].*/$1/
    // from https://github.com/brendangregg/FlameGraph/blob/1b1c6deede9c33c5134c920bdb7a44cc5528e9a7/stackcollapse.pl#L88
    fn uncpp(probe: &str) -> &str {
        if let Some(scope) = probe.find("::") {
            if let Some(open) = probe[scope + 2..].rfind(|c| c == '(' || c == '<') {
                &probe[..scope + 2 + open]
            } else {
                probe
            }
        } else {
            probe
        }
    }

    fn remove_offset(line: &str) -> (bool, bool, bool, &str) {
        let mut has_inlines = false;
        let mut could_be_cpp = false;
        let mut has_semicolon = false;
        let mut last_offset = line.len();
        // This seems risky, but dtrace stacks are c-strings as can be seen in the function
        // responsible for printing them:
        // https://github.com/opendtrace/opendtrace/blob/1a03ea5576a9219a43f28b4f159ff8a4b1f9a9fd/lib/libdtrace/common/dt_consume.c#L1331
        let bytes = line.as_bytes();
        for offset in 0..bytes.len() {
            match bytes[offset] {
                b'>' if offset > 0 && bytes[offset - 1] == b'-' => has_inlines = true,
                b':' if offset > 0 && bytes[offset - 1] == b':' => could_be_cpp = true,
                b';' => has_semicolon = true,
                b'+' => last_offset = offset,
                _ => (),
            }
        }
        (
            has_inlines,
            could_be_cpp,
            has_semicolon,
            &line[..last_offset],
        )
    }

    // DTrace doesn't properly demangle Rust function names, so fix those.
    fn fix_rust_symbol<'a>(&self, frame: &'a str) -> Cow<'a, str> {
        let mut parts = frame.splitn(2, '`');
        if let (Some(pname), Some(func)) = (parts.next(), parts.next()) {
            if self.opt.includeoffset {
                let mut parts = func.rsplitn(2, '+');
                if let (Some(offset), Some(func)) = (parts.next(), parts.next()) {
                    if let Cow::Owned(func) =
                        common::fix_partially_demangled_rust_symbol(func.trim_end())
                    {
                        return Cow::Owned(format!("{}`{}+{}", pname, func, offset));
                    } else {
                        return Cow::Borrowed(frame);
                    }
                }
            }

            if let Cow::Owned(func) = common::fix_partially_demangled_rust_symbol(func.trim_end()) {
                return Cow::Owned(format!("{}`{}", pname, func));
            }
        }

        Cow::Borrowed(frame)
    }

    // we have a stack line that shows one stack entry from the preceding event, like:
    //
    //     unix`tsc_gethrtimeunscaled+0x21
    //     genunix`gethrtime_unscaled+0xa
    //     genunix`syscall_mstate+0x5d
    //     unix`sys_syscall+0x10e
    //       1
    fn on_stack_line(&mut self, line: &str) {
        let (has_inlines, could_be_cpp, has_semicolon, mut frame) = if self.opt.includeoffset {
            (true, true, true, line)
        } else {
            Self::remove_offset(line)
        };

        if could_be_cpp {
            frame = Self::uncpp(frame);
        }

        let frame = if frame.is_empty() {
            Cow::Borrowed("-")
        } else {
            self.fix_rust_symbol(frame)
        };

        if has_inlines {
            let mut inline = false;
            for func in frame.split("->") {
                let mut func = if has_semicolon {
                    func.trim_start_matches('L').replace(';', ":")
                } else {
                    func.trim_start_matches('L').to_owned()
                };
                if inline {
                    func.push_str("_[i]")
                };
                inline = true;
                self.stack_str_size += func.len() + 1;
                self.cache_inlines.push(func);
            }
            while let Some(func) = self.cache_inlines.pop() {
                self.stack.push_front(func);
            }
        } else if has_semicolon {
            self.stack.push_front(frame.replace(';', ":"))
        } else {
            self.stack.push_front(frame.to_string())
        }
    }

    fn on_stack_end(&mut self, count: usize, occurrences: &mut Occurrences) {
        // allocate a string that is long enough to hold the entire stack string
        let mut stack_str = String::with_capacity(self.stack_str_size);

        let mut first = true;
        // add the other stack entries (if any)
        let last = self.stack.len() - 1;
        for (i, e) in self.stack.drain(..).enumerate() {
            if first {
                first = false
            } else {
                stack_str.push(';');
            }
            //trim leaf offset if these were retained:
            if self.opt.includeoffset && i == last {
                stack_str.push_str(Self::remove_offset(&e).3);
            } else {
                stack_str.push_str(&e);
            }
        }

        // count it!
        occurrences.insert_or_add(stack_str, count);

        // reset for the next event
        self.stack_str_size = 0;
        self.stack.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use once_cell::sync::Lazy;
    use pretty_assertions::assert_eq;
    use rand::prelude::*;

    use super::*;
    use crate::collapse::common;
    use crate::collapse::Collapse;

    static INPUT: Lazy<Vec<PathBuf>> = Lazy::new(|| {
        common::testing::check_flamegraph_git_submodule_initialised();
        [
            "./flamegraph/example-dtrace-stacks.txt",
            "./tests/data/collapse-dtrace/flamegraph-bug.txt",
            "./tests/data/collapse-dtrace/hex-addresses.txt",
            "./tests/data/collapse-dtrace/java.txt",
            "./tests/data/collapse-dtrace/only-header-lines.txt",
            "./tests/data/collapse-dtrace/scope_with_no_argument_list.txt",
        ]
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>()
    });

    #[test]
    fn cpp_test() {
        let probe = "TestClass::TestClass2(const char*)[__1cJTestClass2t6Mpkc_v_]";
        assert_eq!("TestClass::TestClass2", Folder::uncpp(probe));

        let probe = "TestClass::TestClass2::TestClass3(const char*)[__1cJTestClass2t6Mpkc_v_]";
        assert_eq!("TestClass::TestClass2::TestClass3", Folder::uncpp(probe));

        let probe = "TestClass::TestClass2<blargh>(const char*)[__1cJTestClass2t6Mpkc_v_]";
        assert_eq!("TestClass::TestClass2<blargh>", Folder::uncpp(probe));

        let probe =
            "TestClass::TestClass2::TestClass3<blargh>(const char*)[__1cJTestClass2t6Mpkc_v_]";
        assert_eq!(
            "TestClass::TestClass2::TestClass3<blargh>",
            Folder::uncpp(probe)
        );
    }

    #[test]
    fn test_collapse_multi_dtrace() -> io::Result<()> {
        let mut folder = Folder::default();
        common::testing::test_collapse_multi(&mut folder, &INPUT)
    }

    #[test]
    fn test_collapse_multi_dtrace_non_utf8() {
        let invalid_utf8 = &[0xf0, 0x28, 0x8c, 0xbc];
        let mk_stack = |bytes: &[u8]| [b"genunix`cv_broadcast+0x1", bytes, b"\n1\n\n"].concat();
        let invalid_stack = &mk_stack(invalid_utf8);
        let valid_stack = &mk_stack(b"");

        let mut input = Vec::new();
        for _ in 0..100 {
            input.extend_from_slice(valid_stack);
        }
        input.extend_from_slice(invalid_stack);
        for _ in 0..100 {
            input.extend_from_slice(valid_stack);
        }

        let mut folder = Folder {
            nstacks_per_job: 1,
            opt: Options {
                nthreads: 12,
                ..Options::default()
            },
            ..Folder::default()
        };
        <Folder as Collapse>::collapse(&mut folder, &input[..], io::sink()).unwrap();
    }

    #[test]
    fn test_collapse_multi_dtrace_simple() -> io::Result<()> {
        common::testing::check_flamegraph_git_submodule_initialised();
        let path = "./flamegraph/example-dtrace-stacks.txt";
        let mut file = fs::File::open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let mut folder = Folder::default();
        <Folder as Collapse>::collapse(&mut folder, &bytes[..], io::sink())
    }

    #[test]
    fn test_collapse_dtrace_would_end_stack() {
        let mut folder = Folder::default();
        assert!(!folder.would_end_stack(b"function_name"));
        assert!(!folder.would_end_stack(b"  function_name  "));
        assert!(!folder.would_end_stack(b"  "));
        assert!(!folder.would_end_stack(b""));
        assert!(!folder.would_end_stack(b""));
        assert!(!folder.would_end_stack(b"  function_name  "));
        assert!(folder.would_end_stack(b"  256  "));

        assert!(!folder.would_end_stack(b"  "));
        assert!(!folder.would_end_stack(b"  "));
        assert!(!folder.would_end_stack(b""));
        assert!(!folder.would_end_stack(b"function_name"));
        assert!(!folder.would_end_stack(b"  function_name  "));
        assert!(!folder.would_end_stack(b"  function_name  "));
        assert!(!folder.would_end_stack(b" "));
        assert!(folder.would_end_stack(b"  12  "));

        assert!(!folder.would_end_stack(b"function_name"));
        assert!(!folder.would_end_stack(b"  function_name  "));
        assert!(!folder.would_end_stack(b"  function_name  "));
        assert!(!folder.would_end_stack(b" "));
        assert!(!folder.would_end_stack(b"  function_name  "));
        assert!(!folder.would_end_stack(b"  function_name  "));
        assert!(folder.would_end_stack(b"  3  "));

        assert!(!folder.would_end_stack(b"function_name"));
        assert!(!folder.would_end_stack(b"  function_name  "));
        assert!(!folder.would_end_stack(b"  5function_name  "));
        assert!(!folder.would_end_stack(b" "));
        assert!(!folder.would_end_stack(b"  function_name  "));
        assert!(!folder.would_end_stack(b"  function_name  "));
        assert!(folder.would_end_stack(b"  3  "));

        assert!(!folder.would_end_stack(b"function_name"));
        assert!(!folder.would_end_stack(b"  function_name  "));
        assert!(!folder.would_end_stack(b"  5424 f"));
        assert!(!folder.would_end_stack(b" "));
        assert!(!folder.would_end_stack(b"  function_name  "));
        assert!(!folder.would_end_stack(b"  function_name  "));
        assert!(folder.would_end_stack(b"  3  "));
    }

    /// Varies the nstacks_per_job parameter and outputs the 10 fastests configurations by file.
    ///
    /// Command: `cargo test bench_nstacks_dtrace --release -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_nstacks_dtrace() -> io::Result<()> {
        let mut folder = Folder::default();
        common::testing::bench_nstacks(&mut folder, &INPUT)
    }

    #[test]
    #[ignore]
    /// Fuzz test the multithreaded collapser.
    ///
    /// Command: `cargo test fuzz_collapse_dtrace --release -- --ignored --nocapture`
    fn fuzz_collapse_dtrace() -> io::Result<()> {
        let seed = thread_rng().gen::<u64>();
        println!("Random seed: {}", seed);
        let mut rng = SmallRng::seed_from_u64(seed);

        let mut buf_actual = Vec::new();
        let mut buf_expected = Vec::new();
        let mut count = 0;

        let inputs = common::testing::read_inputs(&INPUT)?;

        loop {
            let nstacks_per_job = rng.gen_range(1..=500);
            let options = Options {
                includeoffset: rng.gen(),
                nthreads: rng.gen_range(2..=32),
            };

            for (path, input) in inputs.iter() {
                buf_actual.clear();
                buf_expected.clear();

                let mut folder = {
                    let mut options = options.clone();
                    options.nthreads = 1;
                    Folder::from(options)
                };
                folder.nstacks_per_job = nstacks_per_job;
                <Folder as Collapse>::collapse(&mut folder, &input[..], &mut buf_expected)?;
                let expected = std::str::from_utf8(&buf_expected[..]).unwrap();

                let mut folder = Folder::from(options.clone());
                folder.nstacks_per_job = nstacks_per_job;
                <Folder as Collapse>::collapse(&mut folder, &input[..], &mut buf_actual)?;
                let actual = std::str::from_utf8(&buf_actual[..]).unwrap();

                if actual != expected {
                    eprintln!(
                        "Failed on file: {}\noptions: {:#?}\n",
                        path.display(),
                        options
                    );
                    assert_eq!(actual, expected);
                }
            }

            count += 1;
            if count % 10 == 0 {
                println!("Successfully ran {} fuzz tests.", count);
            }
        }
    }
}
