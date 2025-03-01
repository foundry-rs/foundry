use std::io;
use std::path::PathBuf;

use clap::{ArgAction, Parser};
use env_logger::Env;
use inferno::collapse::guess::{Folder, Options};
use inferno::collapse::{Collapse, DEFAULT_NTHREADS};
use once_cell::sync::Lazy;

static NTHREADS: Lazy<String> = Lazy::new(|| DEFAULT_NTHREADS.to_string());

#[derive(Debug, Parser)]
#[clap(
    name = "inferno-collapse-guess",
    about,
    after_help = "\
[1] Attempts to find an appropriate collapser to use based on the input.
                  "
)]
struct Opt {
    // ************* //
    // *** FLAGS *** //
    // ************* //
    /// Silence all log output
    #[clap(short = 'q', long = "quiet")]
    quiet: bool,

    /// Verbose logging mode (-v, -vv, -vvv)
    #[clap(short = 'v', long = "verbose", action = ArgAction::Count)]
    verbose: u8,

    // *************** //
    // *** OPTIONS *** //
    // *************** //
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
    /// Input file, or STDIN if not specified
    #[clap(value_name = "PATH")]
    infile: Option<PathBuf>,
}

impl Opt {
    fn into_parts(self) -> (Option<PathBuf>, Options) {
        let mut options = Options::default();
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
