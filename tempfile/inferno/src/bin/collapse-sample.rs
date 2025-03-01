use std::io;
use std::path::PathBuf;

use clap::{ArgAction, Parser};
use env_logger::Env;
use inferno::collapse::sample::{Folder, Options};
use inferno::collapse::Collapse;

#[derive(Debug, Parser)]
#[clap(
    name = "inferno-collapse-sample",
    about,
    after_help = "\
[1] This processes the result of the sample command on macOS:
            sample 1234 -file out.sample_stacks"
)]
struct Opt {
    // ************* //
    // *** FLAGS *** //
    // ************* //
    /// Don't include modules with function names
    #[clap(long = "no-modules")]
    no_modules: bool,

    /// Silence all log output
    #[clap(short = 'q', long = "quiet")]
    quiet: bool,

    /// Verbose logging mode (-v, -vv, -vvv)
    #[clap(short = 'v', long = "verbose", action = ArgAction::Count)]
    verbose: u8,

    // ************ //
    // *** ARGS *** //
    // ************ //
    /// sample output file, or STDIN if not specified
    #[clap(value_name = "PATH")]
    infile: Option<PathBuf>,
}

impl Opt {
    fn into_parts(self) -> (Option<PathBuf>, Options) {
        let mut options = Options::default();
        options.no_modules = self.no_modules;
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
