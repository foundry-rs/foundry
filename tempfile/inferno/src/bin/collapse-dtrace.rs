use std::io;
use std::path::PathBuf;

use clap::{ArgAction, Parser};
use env_logger::Env;
use inferno::collapse::dtrace::{Folder, Options};
use inferno::collapse::{Collapse, DEFAULT_NTHREADS};
use once_cell::sync::Lazy;

static NTHREADS: Lazy<String> = Lazy::new(|| DEFAULT_NTHREADS.to_string());

#[derive(Debug, Parser)]
#[clap(
    name = "inferno-collapse-dtrace",
    about,
    after_help = "\
[1] This processes the result of the dtrace ustack() as run with:
        dtrace -x ustackframes=100 -n 'profile-97 /pid == 12345 && arg1/ { @[ustack()] = count(); } tick-60s { exit(0); }'
    or including kernel time:
        dtrace -x ustackframes=100 -n 'profile-97 /pid == 12345/ { @[ustack()] = count(); } tick-60s { exit(0); }'
    "
)]
struct Opt {
    // ************* //
    // *** FLAGS *** //
    // ************* //
    /// Include offsets
    #[clap(long = "includeoffset")]
    includeoffset: bool,

    /// Silence all log output
    #[clap(short = 'q', long = "quiet")]
    quiet: bool,

    /// Verbose logging mode (-v, -vv, -vvv)
    #[clap(short = 'v', long = "verbose", action = ArgAction::Count)]
    verbose: u8,

    // *************** //
    // *** OPTIONS *** //
    // *************** //
    /// Number of threads to use.
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
    /// Dtrace script output file, or STDIN if not specified
    infile: Option<PathBuf>,
}

impl Opt {
    fn into_parts(self) -> (Option<PathBuf>, Options) {
        let mut options = Options::default();
        options.includeoffset = self.includeoffset;
        options.nthreads = self.nthreads;
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
