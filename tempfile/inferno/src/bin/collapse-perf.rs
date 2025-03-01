use std::io;
use std::path::PathBuf;

use clap::{ArgAction, Parser};
use env_logger::Env;
use inferno::collapse::perf::{Folder, Options};
use inferno::collapse::{Collapse, DEFAULT_NTHREADS};
use once_cell::sync::Lazy;

static NTHREADS: Lazy<String> = Lazy::new(|| DEFAULT_NTHREADS.to_string());

#[derive(Debug, Parser)]
#[clap(
    name = "inferno-collapse-perf",
    about,
    after_help = "\
[1] perf script must emit both PID and TIDs for these to work; eg, Linux < 4.1:
        perf script -f comm,pid,tid,cpu,time,event,ip,sym,dso,trace
    for Linux >= 4.1:
        perf script -F comm,pid,tid,cpu,time,event,ip,sym,dso,trace
    If you save this output add --header on Linux >= 3.14 to include perf info."
)]
struct Opt {
    // ************* //
    // *** FLAGS *** //
    // ************* //
    /// Include raw addresses where symbols can't be found
    #[clap(long = "addrs")]
    addrs: bool,

    /// All annotations (--kernel --jit)
    #[clap(long = "all")]
    all: bool,

    /// Annotate jit functions with a `_[j]`
    #[clap(long = "jit")]
    jit: bool,

    /// Annotate kernel functions with a `_[k]`
    #[clap(long = "kernel")]
    kernel: bool,

    /// Include PID with process names
    #[clap(long = "pid")]
    pid: bool,

    /// Include TID and PID with process names
    #[clap(long = "tid")]
    tid: bool,

    /// Silence all log output
    #[clap(short = 'q', long = "quiet")]
    quiet: bool,

    /// Verbose logging mode (-v, -vv, -vvv)
    #[clap(short = 'v', long = "verbose", action = ArgAction::Count)]
    verbose: u8,

    // *************** //
    // *** OPTIONS *** //
    // *************** //
    /// Event filter [default: first encountered event]
    #[clap(long = "event-filter", value_name = "STRING")]
    event_filter: Option<String>,

    /// Number of threads to use
    #[clap(
        short = 'n',
        long = "nthreads",
        default_value = &**NTHREADS,
        value_name = "UINT"
    )]
    nthreads: usize,

    // ************ //
    // *** ARGS *** //
    // ************ //
    #[clap(value_name = "PATH")]
    /// Perf script output file, or STDIN if not specified
    infile: Option<PathBuf>,

    #[clap(long = "skip-after", value_name = "STRING")]
    /// If set, will omit all the parent stack frames of any frame with a matched function name.
    ///
    /// Has no effect on the stack trace if no functions are matched.
    skip_after: Vec<String>,
}

impl Opt {
    fn into_parts(self) -> (Option<PathBuf>, Options) {
        let mut options = Options::default();
        options.include_pid = self.pid;
        options.include_tid = self.tid;
        options.include_addrs = self.addrs;
        options.annotate_jit = self.jit || self.all;
        options.annotate_kernel = self.kernel || self.all;
        options.event_filter = self.event_filter;
        options.nthreads = self.nthreads;
        options.skip_after = self.skip_after;
        (self.infile, options)
    }
}

fn main() -> io::Result<()> {
    let opt = Opt::parse();

    // Initialize logger
    if !opt.quiet {
        env_logger::Builder::from_env(Env::default().default_filter_or(match opt.verbose {
            0 => "warn",
            1 => "info",
            2 => "debug",
            _ => "trace",
        }))
        .format_timestamp(None)
        .init();
    }

    let (infile, options) = opt.into_parts();
    Folder::from(options).collapse_file_to_stdout(infile.as_ref())
}
