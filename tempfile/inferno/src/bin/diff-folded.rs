use std::io::{self, IsTerminal};
use std::path::PathBuf;

use clap::{ArgAction, Parser};
use env_logger::Env;
use inferno::differential::{self, Options};

#[derive(Debug, Parser)]
#[clap(
    name = "inferno-diff-folded",
    about,
    after_help = "\
Creates a differential between two folded stack profiles that can be passed
to inferno-flamegraph to generate a differential flame graph.

  $ inferno-diff-folded folded1 folded2 | inferno-flamegraph > diff2.svg

The flamegraph will be colored based on higher samples (red) and smaller
samples (blue). The frame widths will be based on the 2nd folded profile.
This might be confusing if stack frames disappear entirely; it will make
the most sense to ALSO create a differential based on the 1st profile widths,
while switching the hues. To do this, reverse the order of the folded files
and pass the --negate flag to inferno-flamegraph like this:

  $ inferno-diff-folded folded2 folded1 | inferno-flamegraph --negate > diff1.svg

You can use the inferno-collapse-* tools to generate the folded files."
)]
struct Opt {
    // ************* //
    // *** FLAGS *** //
    // ************* //
    /// Normalize sample counts
    #[clap(short = 'n', long = "normalize")]
    normalize: bool,

    /// Strip hex numbers (addresses)
    #[clap(short = 's', long = "strip-hex")]
    strip_hex: bool,

    /// Silence all log output
    #[clap(short = 'q', long = "quiet")]
    quiet: bool,

    /// Verbose logging mode (-v, -vv, -vvv)
    #[clap(short = 'v', long = "verbose", action = ArgAction::Count)]
    verbose: u8,

    // ************ //
    // *** ARGS *** //
    // ************ //
    /// Path to folded stack profile 1
    #[clap(value_name = "PATH1")]
    path1: PathBuf,

    /// Path to folded stack profile 2
    #[clap(value_name = "PATH2")]
    path2: PathBuf,
}

impl Opt {
    fn into_parts(self) -> (PathBuf, PathBuf, Options) {
        (
            self.path1,
            self.path2,
            Options {
                normalize: self.normalize,
                strip_hex: self.strip_hex,
            },
        )
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

    let (folded1, folded2, options) = opt.into_parts();

    if std::io::stdout().is_terminal() {
        differential::from_files(options, folded1, folded2, io::stdout().lock())
    } else {
        differential::from_files(
            options,
            folded1,
            folded2,
            io::BufWriter::new(io::stdout().lock()),
        )
    }
}
