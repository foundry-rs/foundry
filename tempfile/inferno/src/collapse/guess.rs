use std::io::prelude::*;
use std::io::{self, Cursor};

use log::{error, info};

use crate::collapse::{self, dtrace, ghcprof, perf, sample, vsprof, vtune, Collapse};

const LINES_PER_ITERATION: usize = 10;

/// Folder configuration options.
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
            nthreads: *collapse::DEFAULT_NTHREADS,
        }
    }
}

/// A collapser that tries to find an appropriate implementation of `Collapse`
/// based on the input, then delegates to that collapser if one is found.
///
/// If no applicable collapser is found, an error will be logged and
/// nothing will be written.
#[derive(Clone)]
pub struct Folder {
    opt: Options,
}

impl From<Options> for Folder {
    fn from(opt: Options) -> Self {
        Self { opt }
    }
}

impl Default for Folder {
    fn default() -> Self {
        Options::default().into()
    }
}

impl Collapse for Folder {
    fn collapse<R, W>(&mut self, mut reader: R, writer: W) -> io::Result<()>
    where
        R: io::BufRead,
        W: io::Write,
    {
        let mut dtrace = {
            let options = dtrace::Options {
                nthreads: self.opt.nthreads,
                ..Default::default()
            };
            dtrace::Folder::from(options)
        };
        let mut perf = {
            let options = perf::Options {
                nthreads: self.opt.nthreads,
                ..Default::default()
            };
            perf::Folder::from(options)
        };
        let mut sample = sample::Folder::default();
        let mut vtune = vtune::Folder::default();
        let mut vsprof = vsprof::Folder::default();
        let mut ghcprof = ghcprof::Folder::default();

        // Each Collapse impl gets its own flag in this array.
        // It gets set to true when the impl has been ruled out.
        let mut not_applicable = [false; 6];

        let mut buffer = String::new();
        loop {
            let mut eof = false;
            for _ in 0..LINES_PER_ITERATION {
                if reader.read_line(&mut buffer)? == 0 {
                    eof = true;
                }
            }

            macro_rules! try_collapse_impl {
                ($collapse:ident, $index:expr) => {
                    if !not_applicable[$index] {
                        match $collapse.is_applicable(&buffer) {
                            Some(false) => {
                                // We can rule this collapser out.
                                not_applicable[$index] = true;
                            }
                            Some(true) => {
                                // We found a collapser that works! Let's use it.
                                info!("Using {} collapser", stringify!($collapse));
                                let cursor = Cursor::new(buffer).chain(reader);
                                return $collapse.collapse(cursor, writer);
                            }
                            None => (), // We're not yet sure if this collapser is appropriate
                        }
                    }
                };
            }
            try_collapse_impl!(perf, 0);
            try_collapse_impl!(dtrace, 1);
            try_collapse_impl!(sample, 2);
            try_collapse_impl!(vtune, 3);
            try_collapse_impl!(vsprof, 4);
            try_collapse_impl!(ghcprof, 5);

            if eof {
                break;
            }
        }

        error!("No applicable collapse implementation found for input");

        Ok(())
    }

    fn is_applicable(&mut self, _line: &str) -> Option<bool> {
        unreachable!()
    }
}
