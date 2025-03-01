use super::common::{self, CollapsePrivate};
use std::{borrow::Cow, io};

/// Recursive backtrace folder configuration options.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Options {
    /// The number of threads to use.
    ///
    /// Default is the number of logical cores on your machine.
    pub nthreads: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            nthreads: *common::DEFAULT_NTHREADS,
        }
    }
}

/// A "middleware" folder that receives and outputs the folded stack format
/// expected by [`crate::flamegraph::from_lines`], collapsing direct recursive
/// backtraces.
#[derive(Clone)]
pub struct Folder {
    /// The number of stacks per job to send to the threadpool.
    nstacks_per_job: usize,

    // Options...
    opt: Options,
}

impl From<Options> for Folder {
    fn from(mut opt: Options) -> Self {
        if opt.nthreads == 0 {
            opt.nthreads = 1;
        }
        Self {
            nstacks_per_job: common::DEFAULT_NSTACKS_PER_JOB,
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
    fn pre_process<R>(
        &mut self,
        _reader: &mut R,
        _occurrences: &mut super::common::Occurrences,
    ) -> std::io::Result<()>
    where
        R: std::io::BufRead,
    {
        // Don't expect any header.
        Ok(())
    }

    fn collapse_single_threaded<R>(
        &mut self,
        reader: R,
        occurrences: &mut super::common::Occurrences,
    ) -> std::io::Result<()>
    where
        R: std::io::BufRead,
    {
        for line in reader.lines() {
            let line = line?;
            let (stack, count) = Self::line_parts(&line)
                .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;

            occurrences.insert_or_add(Self::collapse_stack(stack.into()).into_owned(), count);
        }
        Ok(())
    }

    fn would_end_stack(&mut self, _line: &[u8]) -> bool {
        // For our purposes, every line is an independent stack
        true
    }

    fn clone_and_reset_stack_context(&self) -> Self {
        self.clone()
    }

    fn is_applicable(&mut self, _input: &str) -> Option<bool> {
        // It seems doubtful that the user would ever want to guess to collapse
        // recursive traces, so let's just never consider ourselves applicable.
        Some(false)
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
    fn line_parts(line: &str) -> Option<(&str, usize)> {
        line.rsplit_once(' ')
            .and_then(|(stack, count)| Some((stack, count.parse().ok()?)))
    }

    fn collapse_stack(stack: Cow<str>) -> Cow<str> {
        // First, determine whether we can avoid allocation by just returning
        // the original stack (in the case that there is no recursion, which is
        // likely the mainline case).
        if !Self::is_recursive(&stack) {
            return stack;
        }

        // There is recursion, so we can't get away without allocating a new
        // String.
        let mut result = String::with_capacity(stack.len());
        let mut last = None;
        for frame in stack.split(';') {
            if last.map_or(true, |l| l != frame) {
                result.push_str(frame);
                result.push(';')
            }
            last = Some(frame);
        }

        // Remove the trailing semicolon
        result.pop();

        result.into()
    }

    /// Determine whether or not a stack contains direct recursion.
    fn is_recursive(stack: &str) -> bool {
        let mut last = None;
        for current in stack.split(';') {
            match last {
                None => {
                    last = Some(current);
                }
                Some(l) => {
                    if l == current {
                        // Recursion!
                        return true;
                    } else {
                        last = Some(current);
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod test {

    use super::*;

    #[test]
    fn test_collapse_stack() {
        assert_eq!(Folder::collapse_stack("".into()), "");
        assert_eq!(Folder::collapse_stack("single".into()), "single");
        assert_eq!(
            Folder::collapse_stack("not;recursive".into()),
            "not;recursive"
        );
        assert_eq!(
            Folder::collapse_stack("has;some;some;recursion;recursion".into()),
            "has;some;recursion"
        );
        assert_eq!(
            Folder::collapse_stack("co;recursive;co;recursive".into()),
            "co;recursive;co;recursive"
        );
    }

    #[test]
    fn test_line_parts() {
        assert_eq!(
            Folder::line_parts("foo;bar;baz 42"),
            Some(("foo;bar;baz", 42))
        );
        assert_eq!(Folder::line_parts(""), None);
        assert_eq!(Folder::line_parts("no;number"), None);
    }
}
